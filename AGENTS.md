# Driving LogicPort from an agent

This project exposes the LA1034 logic analyzer as a service an agent can operate
end to end. Everything a person can do in the web dashboard is also an MCP tool
and a REST endpoint, backed by one operation registry — so nothing drifts
between surfaces.

## Setup

Start the daemon (it serves the API and dashboard on port 8471):

```sh
docker compose up -d --build analyzerd
```

For development without hardware, use the simulator backend on port 8472:

```sh
docker compose up -d --build sim
```

Point your MCP client at the daemon's streamable HTTP endpoint:

```json
{ "type": "http", "url": "http://127.0.0.1:8471/mcp" }
```

The daemon also speaks MCP over stdio (`analyzerd mcp`) if you prefer a stdio
transport.

## The operation model

There is one canonical operation per capability (e.g. `device.status`,
`acq.single`, `capture.get`, `sample.apply`). Two ways to call:

- **Generic** — the `op_call` MCP tool takes an operation id and its parameters:

  ```json
  { "name": "op_call", "arguments": { "op": "acq.single", "params": {} } }
  ```

- **Named tools** — a curated set of higher-level MCP tools wrap the most useful
  operations directly: `device_status`, `lease_acquire` / `lease_release`,
  `acquire_single` / `acquire_recurring_start` / `acquire_halt` /
  `acquire_status` / `acquire_wait` / `acquire_trigger_immediate`,
  `capture_list` / `capture_get` / `capture_summary` / `capture_export` /
  `capture_search` / `capture_diff` / `capture_measure` / `capture_state_list`,
  `project_get` / `project_put` / `project_notes`, and `ops_list`.

Discover everything at runtime:

- `ops_list` (or `meta.ops_list`) — every operation id, title, and area.
- `GET /api/ops/<id>/schema` — an operation's JSON parameter schema.

## Leases

Operations that change device or session state are marked mutating and require a
lease. Acquire one first, pass the token, release it when done:

```json
{ "name": "lease_acquire", "arguments": {} }
{ "name": "acquire_single", "arguments": { "wait": true, "lease": "<token>" } }
{ "name": "lease_release",  "arguments": { "lease": "<token>" } }
```

Read-only operations (status, capture reads, searches) do not need a lease.

## A typical session

1. **Check the device** — `device_status` → expect `state: "connected"` and
   `usb_error_count: 0`. If it reports `needs_replug`, tell the user to re-plug;
   the daemon recovers from transient faults on its own but cannot clear a state
   that requires power removal.
2. **Configure** (optional) — `sample.get`, then `sample.apply` to set mode,
   rate, and compression.
3. **Acquire** — `acquire_single` (add `wait: true` to block until the capture
   completes). For continuous capture use `acquire_recurring_start` /
   `acquire_halt`.
4. **Read** — `capture_list` for ids, `capture_get` for RLE or expanded samples
   (paged, optional channel subset), `capture_summary` for edges/rates/min-pulse.
5. **Analyze** — `capture_search` (pattern/edge/value-range/duration),
   `capture_diff` (first divergence between two captures), `capture_measure`
   (any measurement between two points), `capture_state_list`.
6. **Decode** — protocol interpreters are configured per capture; results come
   back as decoded frames.
7. **Import / export** — `project.import_lpf` to load an `.LPF` project;
   `capture_export` for CSV/VCD/text/JSON.

## Verification tools (stimulus board attached)

When an optional stimulus microcontroller is present, extra tools let an agent
select a pattern program and run a hardware-in-the-loop case end to end:
`stimulus_list`, `stimulus_program`, `stimulus_status`, and `verify_run`
(program stimulus → acquire → compare to expected → verdict).

## Conventions

- Every read carries a freshness envelope, so an agent can tell how current the
  data is.
- Errors are structured (`code`, `message`, `hint`, `detail`) — branch on
  `code`; the `hint` is actionable.
- This service is the only supported way to touch the device. Do not open the
  USB interface directly; the daemon owns the interface, paces I/O, and attributes
  results correctly.
