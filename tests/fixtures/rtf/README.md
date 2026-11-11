# Realtime-factor workloads

One realistic stock-Arduino sketch per main board, timed by
`scripts/perf/board_rtf.py` (run from `scripts/perf/board_rtx.py`, which the
Core Perf `board-throughput` job already runs). Each sketch runs forever and
prints `RTF_READY <id>` once, so a dead boot cannot pass as a fast run.
The Pico workload also proves continued display updates; a ready marker alone
does not prove that its application loop kept running.

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

This writes `rtf.json` (board, workload, guest_s, wall_s, Mcycles/s, rtf,
commit, runner). The gate compares `index = best workload RTF / best spin RTx of the same chip`
against `scripts/perf/rtf_baselines.json`, so a faster or slower runner does
not move it; a drop of more than 25% (`--rtf-tolerance`) fails. The
deterministic Ir/step gate (`board_perf.py`) stays the fine-grained detector. After an intended change,
re-baseline with `--update-rtf`.


## RP2040 baseline correction (2026-11-10)

The original `pico_oled_pwm` index, 0.0776, came from
[Core Perf run 42653202653](https://github.com/w1ne/labwired-core/actions/runs/42653202653).
Its ROM was the minimal stub before
[the clean-room ROM implementation](https://github.com/w1ne/labwired-core/commit/15717d9f13f88dd925eca4cc604a7b21785350d4).
That stub let the sketch print its ready marker and draw one frame, then the
first `analogWrite` soft-float call stopped the application from progressing.
The simulator could still spend the full cycle budget by servicing interrupts
and skipping idle time. The old gate therefore timed a stalled application.

A same-binary, same-firmware control that changed only the ROM proved the
semantic difference over the full 8 guest seconds:

| ROM | Distinct OLED frames | Last draw in firmware | PWM enable register |
|-----|---------------------:|----------------------:|--------------------:|
| Old minimal stub | 1 | 100 ms | `0x00` |
| Current clean-room ROM | 79 | 7900 ms | `0x80` (slice 7) |

The current ROM performs the counter redraws and PWM writes that the workload
was intended to measure. Restoring the old execution path would restore an
incorrect result. Only the Pico RTF baseline is remeasured for this correction;
the three other workload baselines and the 25% regression tolerance stay fixed.
The fresh release binary measured index 0.04132 (best workload RTF 3.17206
divided by spin RTx 76.762012; best of three for each).
Measurement inputs, hashes, commands, spin normalization, timing samples and
the old-ROM negative control are recorded in
[`2026-11-10-rp2040-rtf-rom-correction.json`](../../../scripts/perf/receipts/2026-11-10-rp2040-rtf-rom-correction.json).

The gate now checks the existing universal framebuffer history in every timed
Pico run. The sketch redraws every 100 ms. Over 8 seconds, allowing two update
periods for startup and the budget boundary requires at least 78 distinct frame
hashes and a final update at or after 7.8 seconds. These are minimum progress
bounds, not an exact frame-count equality. Missing history, a frozen image,
insufficient updates and early cessation all fail. The old ROM is a live
negative control: it still prints `RTF_READY` and reaches `max_cycles`, but its
single frame at 101531 microseconds is now rejected.
