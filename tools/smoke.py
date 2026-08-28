#!/usr/bin/env python3
"""D9 clean-runtime smoke across UI, REST, and MCP on the real analyzer."""

from __future__ import annotations

import json
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BASE = "http://127.0.0.1:8471"


def compose(*args: str) -> None:
    subprocess.run(("docker", "compose", *args), cwd=ROOT, check=True)


def request(path: str, body: dict[str, object] | None = None) -> object:
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(
        BASE + path,
        data=data,
        headers={"content-type": "application/json"} if data is not None else {},
    )
    with urllib.request.urlopen(req, timeout=3) as response:
        payload = response.read()
    content_type = response.headers.get_content_type()
    return json.loads(payload) if content_type == "application/json" else payload.decode()


def mcp(identifier: int, method: str, params: dict[str, object]) -> dict[str, object]:
    value = request(
        "/mcp",
        {"jsonrpc": "2.0", "id": identifier, "method": method, "params": params},
    )
    if not isinstance(value, dict) or value.get("error") is not None:
        raise RuntimeError(f"MCP {method} failed: {value!r}")
    return value


def structured(response: dict[str, object]) -> dict[str, object]:
    result = response.get("result")
    content = result.get("structuredContent") if isinstance(result, dict) else None
    if not isinstance(content, dict):
        raise RuntimeError(f"MCP response lacks structuredContent: {response!r}")
    return content


def verify_runtime(started: float) -> None:
    deadline = started + 15
    last_error = "daemon not ready"
    while time.monotonic() < deadline:
        try:
            health = request("/api/health")
            if isinstance(health, dict) and health.get("ok") is True:
                break
        except (OSError, urllib.error.URLError, json.JSONDecodeError) as error:
            last_error = str(error)
        time.sleep(0.1)
    else:
        raise RuntimeError(f"daemon missed 15 s deadline: {last_error}")

    root = request("/")
    if not isinstance(root, str) or '<div id="root"></div>' not in root:
        raise RuntimeError("UI root was not served")
    status: object = None
    while time.monotonic() < deadline:
        try:
            status = request("/api/ops/device.status", {})
            if isinstance(status, dict) and status.get("state") == "connected":
                break
        except (OSError, urllib.error.URLError, json.JSONDecodeError) as error:
            status = {"request_error": str(error)}
        time.sleep(0.05)
    else:
        raise RuntimeError(f"device missed 15 s connected deadline: {status!r}")
    capture = request("/api/ops/acq.single", {"wait": True})
    if not isinstance(capture, dict) or int(capture.get("expanded_len", 0)) <= 0:
        raise RuntimeError(f"REST first capture failed: {capture!r}")
    elapsed = time.monotonic() - started
    if elapsed >= 15:
        raise RuntimeError(f"first capture missed 15 s deadline ({elapsed:.3f}s)")

    initialized = mcp(1, "initialize", {"protocolVersion": "2025-06-18"})
    result = initialized.get("result")
    if not isinstance(result, dict) or result.get("protocolVersion") != "2025-06-18":
        raise RuntimeError("MCP initialize negotiated the wrong protocol")
    device = structured(mcp(2, "tools/call", {"name": "device_status", "arguments": {}}))
    if device.get("state") != "connected":
        raise RuntimeError(f"MCP device_status failed: {device!r}")
    lease = structured(mcp(3, "tools/call", {"name": "lease_acquire", "arguments": {}}))
    token = lease.get("lease")
    if not isinstance(token, str):
        raise RuntimeError("MCP lease_acquire returned no token")
    acquired = structured(
        mcp(
            4,
            "tools/call",
            {"name": "acquire_single", "arguments": {"wait": True, "lease": token}},
        )
    )
    if int(acquired.get("expanded_len", 0)) <= 0:
        raise RuntimeError(f"MCP acquire_single failed: {acquired!r}")
    print(f"D9 smoke passed: first capture in {elapsed:.3f}s; REST and MCP green")


def main() -> int:
    # Build time is intentionally outside the runtime deadline: the D9 limit is
    # from container start to configured device and first displayed capture.
    compose("build", "analyzerd")
    started = time.monotonic()
    compose("up", "-d", "--force-recreate", "analyzerd")
    try:
        verify_runtime(started)
    finally:
        # A failed smoke must never retain the exclusive USB interface and
        # poison the next HIL attempt.  Keep the container for log inspection,
        # but release the analyzer on every exit path.
        subprocess.run(
            ("docker", "compose", "stop", "analyzerd"),
            cwd=ROOT,
            check=False,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (RuntimeError, OSError, subprocess.CalledProcessError) as error:
        print(f"D9 smoke failed: {error}", file=sys.stderr)
        sys.exit(1)
