# External Components — dsPIC33FJ32MC204 (M3)

| Device | Bus | Type | Fidelity | Descriptor |
| :--- | :--- | :--- | :--- | :--- |
| TMP102 temperature sensor | `i2c1` | `tmp102` | sensor model (12-bit left-justified temperature, pointer register, writable CONFIG/TLOW/THIGH); the starter is the model's 25.0 °C | `configs/peripherals/tmp102.yaml` (embedded descriptor) |
| SPI wire loopback | `spi1` | `loopback` | harness, **not silicon**: `SDO` tied to `SDI`, every byte returns unchanged | — |

Both are declared in `examples/dspic33fj32mc204/system-m3.yaml` and attached by
the PIC24 session builder to the CPU-internal controllers:

- the TMP102 goes through the generic external-device factory
  (`build_external_i2c_device`), so `i2c_address: 0x48` and the descriptor
  config behave as on every other I²C-capable chip;
- the loopback is a bring-up jumper model for the SPI master path (there is
  no real SPI slave on the demo board).

UART1 remains CPU-internal (writes to `U1TXREG` reach the console sink).

ADC1 has no external component: analog inputs are injected through the
`Pic24::set_adc_analog(channel, code)` harness seam (the integration test
drives AN4 with `0x2AA`).
