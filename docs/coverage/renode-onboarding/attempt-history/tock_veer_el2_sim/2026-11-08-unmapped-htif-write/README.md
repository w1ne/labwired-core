# Unmapped HTIF write review run

This direct CLI run used the onboarding profile before it declared an HTIF
console window. The CPU reached the first byte store at `0xD0580000` and
stopped with `memory_violation`; 1 of 4 smoke assertions passed. The per-run
`result.json`, JUnit report, and snapshot are preserved in `simulation-output/`.

Command:

```text
target/debug/labwired test --script examples/renode-onboarding/tock_veer_el2_sim-smoke.yaml --output-dir /tmp/renode-tock-veer-el2-sim-review --no-uart-stdout
```
