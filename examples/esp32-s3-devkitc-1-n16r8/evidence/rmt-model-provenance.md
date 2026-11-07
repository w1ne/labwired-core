# RMT model provenance

The register-level design check used LabWired Part Knowledge for ESP32-S3. It
returned `VENDOR_ONLY` claims from Espressif CMSIS-SVD v21, SHA-256
`d7629249eba2ad660a960530d124966e9274c8228341380dd4102f251e879e17`:

- `RMT.CH0_TX_CONF0` at `0x60016020` exposes `TX_START`, `DIV_CNT`, `MEM_SIZE`,
  `IDLE_OUT_EN`, and `CONF_UPDATE` at the returned SVD field positions.
- `RMT.SYS_CONF` at `0x600160C0` identifies `SCLK_SEL=1` as `CLK_80MHz`.
- `GPIO.FUNC38_OUT_SEL_CFG` at `0x600045EC` has a 9-bit `OUT_SEL` field for
  selecting an internal output signal or direct GPIO output.

These are vendor-SVD claims, not cross-checked facts. The GPIO model maps its
RMT channel 0 output to selector value 81 in
[`gpio.rs`](../../../crates/core/src/peripherals/esp32s3/gpio.rs); the
numeric selector mapping was not independently confirmed against a physical
board. The system profile uses the existing v1.1 board wiring record, and no
silicon capture was made for this smoke.
