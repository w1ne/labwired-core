# Realtime-factor workloads

One realistic stock-Arduino sketch per main board, timed by
`scripts/perf/board_rtf.py` (run from `scripts/perf/board_rtx.py`, which the
Core Perf `board-throughput` job already runs). Each sketch runs forever and
prints `RTF_READY <id>` once. Every measured run must reach that marker and
the complete cycle budget. The Pico workload also proves continued display
progress; a startup marker alone did not detect its old ROM stall.

| id | board | parts | guest time |
|----|-------|-------|-----------:|
| `uno_feeder` | Arduino Uno (ATmega328P, 16 MHz) | LCD1602 I2C 0x27, 4x4 keypad, DC motor PWM on D11 | 2.0 s |
| `pico_oled_pwm` | Raspberry Pi Pico (RP2040) | SSD1306 128x64 I2C0 0x3C, PWM on GP15 | 8.0 s |
| `c3_i2c_uart` | ESP32-C3 (Wi-Fi never started) | INA219 on I2C0 GPIO4/5, UART0 log | 0.5 s |
| `f411_timer_uart` | Black Pill STM32F411CEU6 | 1 kHz TIM2 interrupt, UART log | 0.5 s |

`firmware.elf` files are built with `arduino-cli` and stripped of debug info
(`<toolchain>-objcopy --strip-debug`):

```
arduino-cli compile -b arduino:avr:uno                      uno_feeder       # arduino:avr 1.8.8
arduino-cli compile -b rp2040:rp2040:rpipico                pico_oled_pwm    # rp2040:rp2040 6.3.2
arduino-cli compile -b esp32:esp32:esp32c3                  c3_i2c_uart      # esp32:esp32 3.3.12
arduino-cli compile -b STMicroelectronics:stm32:GenF4:pnum=BLACKPILL_F411CE f411_timer_uart  # stm32 3.0.0
```

Libraries: LiquidCrystal I2C 1.1.2, Keypad 3.1.1, Adafruit SSD1306 2.5.17,
Adafruit GFX 1.12.6. `c3_i2c_uart/partitions.bin` is the sketch's partition
table from the same build.

Measure locally:

```
cargo build --release -p labwired-cli --features event-scheduler
python3 scripts/perf/board_rtx.py --boards atmega328p,rp2040,esp32c3,stm32f411ceu6
```

This writes `rtf.json`, including each workload sample, the long-spin samples
and denominator, and hashes of the CLI, firmware, ROM, configurations and
harness. The checkout identity is recorded separately; it does not prove
which source built a supplied CLI.

The gate compares `index = best workload RTF / best spin RTx of the same chip`
against `scripts/perf/rtf_baselines.json`. This normalization cancels a common
host-speed factor. Different instruction mixes, host contention and independent
spin optimizations can still move the ratio. A drop of more than 25%
(`--rtf-tolerance`) fails. The deterministic Ir/step gate (`board_perf.py`)
remains the finer detector.

For `pico_oled_pwm`, each eight-second sample must contain at least 75 distinct
nonblank OLED frames and a final frame at or after 7.5 seconds. The sketch
redraws every 100 ms; this allows five updates for startup or missed draws.
The guard checks the existing result's declared I2C0/0x3C display, dimensions,
power state, frame hashes and increasing timestamps. It adds no execution
observer. Missing or malformed evidence fails the measurement.

The former Pico baseline used a ROM without the floating-point table required
by the committed ELF. Replaying that ROM produces one display frame and then
stalls after the ready marker. The corrected ROM produces 79 frames. Its replacement
measurement and positive/negative controls are recorded in
[`2026-11-10-rp2040-rtf-progress.json`](../../../scripts/perf/receipts/2026-11-10-rp2040-rtf-progress.json).

Use `--update-rtf` only after verifying equivalent useful work, reviewing the
reason for a baseline change, and retaining the raw measurement and build
provenance. A failing progress check must never be re-baselined as a fast run.
