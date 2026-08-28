#!/usr/bin/env python3
"""Run the binary D1-D12 completion contract without granting partial credit."""

from __future__ import annotations

import json
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
REPORT = ROOT / "verification" / "report.json"


@dataclass
class Result:
    gate: str
    passed: bool
    detail: str


def command(*args: str) -> tuple[bool, str]:
    completed = subprocess.run(
        args,
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        check=False,
    )
    lines = [line.strip() for line in completed.stdout.splitlines() if line.strip()]
    detail = lines[-1] if lines else f"exit {completed.returncode}"
    return completed.returncode == 0, detail


def load_report() -> tuple[dict[str, object] | None, str]:
    if not REPORT.is_file():
        return None, "verification/report.json missing"
    try:
        value = json.loads(REPORT.read_text())
    except (OSError, json.JSONDecodeError) as error:
        return None, f"invalid verification/report.json: {error}"
    if not isinstance(value, dict):
        return None, "verification/report.json root is not an object"
    return value, "report loaded"


def report_gate(report: dict[str, object] | None, gate: str) -> Result:
    if report is None:
        return Result(gate, False, "verification/report.json missing")
    gates = report.get("gates")
    entry = gates.get(gate) if isinstance(gates, dict) else None
    status = entry.get("status") if isinstance(entry, dict) else None
    if status != "pass":
        return Result(gate, False, f"report gates.{gate}.status is {status!r}")
    return Result(gate, True, "last archived verification run passed")


def main() -> int:
    results: list[Result] = []
    hil_ok, hil_detail = command("./lp", "hil")
    report, report_detail = load_report()

    unverified = report.get("unverified_ops") if report is not None else None
    d1 = hil_ok and unverified == 0
    detail = hil_detail if not hil_ok else f"unverified_ops={unverified!r}; {report_detail}"
    results.append(Result("D1", d1, detail))
    for gate in ("D2", "D3", "D4", "D5"):
        results.append(report_gate(report, gate))
    ok, detail = command("./lp", "lpf-check")
    results.append(Result("D6", ok, detail))
    ok, detail = command(
        "docker",
        "compose",
        "run",
        "--rm",
        "--no-deps",
        "test",
        "cargo",
        "nextest",
        "run",
        "-p",
        "ops-coverage",
    )
    results.append(Result("D7", ok, detail))

    ok, detail = command("./lp", "test")
    results.append(Result("D8", ok, detail))
    ok, detail = command("./lp", "smoke")
    results.append(Result("D9", ok, detail))

    docs_ok, docs_detail = command("./lp", "docs-check")
    protocol = (ROOT / "docs" / "PROTOCOL.md").read_text()
    generated = ROOT / "docs" / "VERIFICATION-REPORT.md"
    d10 = docs_ok and "[unknown]" not in protocol and generated.is_file()
    if not generated.is_file():
        docs_detail = "docs/VERIFICATION-REPORT.md missing"
    elif "[unknown]" in protocol:
        docs_detail = "docs/PROTOCOL.md contains [unknown]"
    results.append(Result("D10", d10, docs_detail))

    results.append(report_gate(report, "D11"))
    lint_ok, lint_detail = command("./lp", "lint")
    gaps = (ROOT / "docs" / "KNOWN-GAPS.md").read_text().strip()
    d12 = lint_ok and not gaps
    if gaps:
        lint_detail = "docs/KNOWN-GAPS.md is not empty"
    results.append(Result("D12", d12, lint_detail))

    width = max(len(result.detail) for result in results)
    print("Gate  Result  Evidence")
    print("----  ------  " + "-" * min(width, 72))
    for result in results:
        print(f"{result.gate:<4}  {'PASS' if result.passed else 'FAIL':<6}  {result.detail}")
    passed = sum(result.passed for result in results)
    print(f"DONE: {'YES' if passed == 12 else 'NO'} ({passed}/12 gates passed)")
    return 0 if passed == 12 else 1


if __name__ == "__main__":
    sys.exit(main())
