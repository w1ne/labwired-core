# esp32s3-i2c-tmp102 — Plan 4 demo

ESP32-S3 firmware that reads a TMP102 temperature sensor over I²C0 once
per second, prints the temperature over USB-Serial-JTAG, and toggles
GPIO2 when temperature exceeds 30 °C.

## What it does

Each second the SYSTIMER alarm fires. The handler sets a flag; the main
loop then issues `I2c::write_read(0x48, &[0x00], &mut buf[2])`, decodes
a 12-bit temperature from the two-byte response, prints `T = NN.NN C`,
and sets GPIO2 high if T > 30.00 °C, otherwise low.

The firmware uses integer math (`raw_units * 625 / 100`) rather than
`f32`, so the FPU is not required — the simulator's CPU model does not
implement the Xtensa FP coprocessor.

The simulator's TMP102 model drifts +0.5 °C per read, wrapping from 35
back to 20 — so over ~30 reads the temperature sweeps through the
threshold both directions, exercising both edges of the GPIO toggle.

## Run in the simulator

The end-to-end test in `crates/core/tests/e2e_i2c_tmp102.rs` builds the
firmware, runs it on the simulator with the TMP102 model attached at
0x48, and asserts both the JTAG output and the GPIO2 transitions:

```
cargo test -p labwired-core --features esp32s3-fixtures \
    --release --test e2e_i2c_tmp102
```

Expected output (captured from the JTAG sink, ~14 simulated seconds):

```
T = 25.00 C
T = 25.50 C
T = 26.00 C
...
T = 30.00 C
T = 30.50 C   ← GPIO2 goes high here
T = 31.00 C
...
```

The simulator also emits GPIO transition events to the configured
observer; the e2e test asserts the rising edge on GPIO2.

## Run on real hardware

See [RUNBOOK.md](RUNBOOK.md).

## Sources

- ESP32-S3 TRM v1.4 §29 (I²C controller)
- TI TMP102 datasheet (December 2008, Rev. June 2015)
- esp-hal 1.1.0 `i2c::master::I2c`
