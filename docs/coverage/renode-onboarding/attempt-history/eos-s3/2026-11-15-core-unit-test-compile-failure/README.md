# Focused controller test compile failure

Command:

```text
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=$(realpath target) cargo test -p labwired-core --lib pl011_rx_level_drives_other_intr_and_is_gated_to_nvic_7
```

The test build found five `SystemBus` fixtures that omitted the newly cached
`irq_input_receiver_indices` field, at `crates/core/src/bus/tests_main.rs` lines
3873, 4005, 4303, 4591 and 4752. The fixtures were updated after this attempt.
