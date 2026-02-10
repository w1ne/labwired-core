# CI Examples (Golden Scripts)

These YAML scripts are stable, minimal “golden” examples for `labwired test`.

Run from the repo root:

```bash
cargo build --release -p labwired-cli
./target/release/labwired test --script examples/ci/dummy-max-steps.yaml --no-uart-stdout
```

Expected outcomes:

- `dummy-max-steps.yaml`: exit code `0` (passes; asserts `expected_stop_reason: max_steps`)
- `dummy-max-cycles.yaml`: exit code `0` (passes; asserts `expected_stop_reason: max_cycles`)
- `dummy-wall-time.yaml`: exit code `0` (passes; asserts `expected_stop_reason: wall_time`)
- `dummy-fail-uart.yaml`: exit code `1` (fails; asserts UART contains a string that isn’t emitted)
