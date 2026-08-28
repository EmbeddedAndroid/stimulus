#!/usr/bin/env bash
set -euo pipefail

DC=(docker compose)

case "${1:-help}" in
  help)
    echo "usage: ./lp {build|test|lpf-check|lint|docs-check|report|hil|smoke|sh|usb-smoke|done}"
    ;;
  build)
    "${DC[@]}" build analyzerd test
    ;;
  test)
    started=$SECONDS
    "${DC[@]}" run --rm --no-deps test cargo test --workspace
    "${DC[@]}" run --rm --no-deps web-dev sh -lc 'npm ci && npm run build && npm test'
    "${DC[@]}" up -d --build --force-recreate sim
    "${DC[@]}" run --rm e2e
    elapsed=$((SECONDS - started))
    if (( elapsed >= 600 )); then
      echo "no-hardware suite exceeded 10 minutes: ${elapsed}s" >&2
      exit 1
    fi
    echo "no-hardware suite: ${elapsed}s"
    ;;
  lpf-check)
    "${DC[@]}" run --rm --no-deps test cargo test -p lp-lpf
    "${DC[@]}" up -d --build --force-recreate sim
    "${DC[@]}" run --rm e2e npx playwright test -c ../tests/e2e/playwright.config.ts lpf-visual.spec.ts
    echo "LPF corpus and 17 visual snapshots passed"
    ;;
  lint)
    "${DC[@]}" run --rm --no-deps test sh -c 'cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings -D clippy::unwrap_used -D clippy::expect_used && cargo xtask gen --check'
    ;;
  docs-check)
    "${DC[@]}" run --rm --no-deps tools python3 tools/docs_check.py
    ;;
  report)
    "${DC[@]}" run --rm --no-deps test cargo run -p lp-verify -- report
    ;;
  sh)
    "${DC[@]}" run --rm --no-deps test /bin/bash
    ;;
  usb-smoke)
    "${DC[@]}" run --rm --no-deps test cargo run -p lp-ftdi --example probe
    ;;
  hil)
    run_ts="${LP_RUN_TS:-$(date -u +%Y%m%dT%H%M%SZ)}"
    if [[ "${2:-}" == "--soak" ]]; then
      export LP_SOAK=1
      echo "D11 soak enabled (LP_SOAK=1; LP_SOAK_SECS=${LP_SOAK_SECS:-3600}, LP_SOAK_ITERS=${LP_SOAK_ITERS:-100})"
    fi
    hil_args=(--profile hil run --rm)
    stimulus_port="${LP_STIMULUS_PORT:-}"
    if [[ -n "$stimulus_port" && -e "$stimulus_port" ]]; then
      hil_args+=(--device "$stimulus_port:$stimulus_port")
    elif [[ -n "$stimulus_port" ]]; then
      echo "HW_CAP_MISSING: stimulus port does not exist: $stimulus_port" >&2
    fi
    LP_RUN_TS="$run_ts" "${DC[@]}" "${hil_args[@]}" hil
    "${DC[@]}" run --rm --no-deps -e LP_RUN_TS="$run_ts" test sh -c '
      test -s "verification/runs/$LP_RUN_TS/verdicts.jsonl"
      rm -rf verification/runs/latest
      ln -s "$LP_RUN_TS" verification/runs/latest
    '
    "${DC[@]}" run --rm --no-deps test cargo run -p lp-verify -- report
    ;;
  smoke)
    exec python3 tools/smoke.py
    ;;
  done)
    exec python3 tools/done.py
    ;;
  *)
    echo "unknown command: $1" >&2
    exit 2
    ;;
esac
