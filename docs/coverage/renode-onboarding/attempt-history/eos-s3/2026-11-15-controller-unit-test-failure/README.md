# Focused controller test assertion failure

Command:

```text
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=$(realpath target) cargo test -p labwired-core --lib pl011_rx_level_drives_other_intr_and_is_gated_to_nvic_7
```

The model test expected `set_irq_input_level` to report an output transition
while the UART enable gate was closed. The actual `Some(false)` was correct:
the input status changed, but the gated NVIC output did not. The test now
expects `Some(false)` at that point and still checks the later gated rise and
deassertion.
