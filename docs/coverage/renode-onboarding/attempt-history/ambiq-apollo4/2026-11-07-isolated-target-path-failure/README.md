# Isolated target-path attempt

The target-only runner compiled the CLI and Apollo4 smoke firmware successfully
with `CARGO_TARGET_DIR=/dev/shm/renode-round338-target`, then failed before
simulation because the smoke YAML resolves its firmware under the checkout's
`target/` path. This is a harness path mismatch; no Apollo4 instructions ran.
The log and generated metrics are preserved here for audit. The run is not a
platform failure and is not included in the generated scoreboard after a
corrected target path succeeds.
