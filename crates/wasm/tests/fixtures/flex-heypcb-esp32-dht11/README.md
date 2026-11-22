# HeyPCB ESP32-S3 DHT11 on a single-wire Flex line

Portable Rust-to-Wasm firmware reads the DHT11 sensor of the pinned **HeyPCB ESP32-S3 DHT11** board, revision A, over its single-wire data line. The sensor is the catalog `dht11` pins-only device model (`configs/devices/dht11.yaml`), the descriptor that the detailed path binds to MCU pads. The firmware sends the start signal, times the answer one microsecond at a time, decodes the 40-bit frame, drives the IO48 user LED as a humidity indicator and prints each reading to a Flex console that stands for the board's native USB serial port. No ESP32-S3 CPU or GPIO peripheral is emulated.

The board needed one generic change: the existing `behavior` co-simulation adapter now hosts a catalog `gpio_device` part by `config.device_type`, as it already hosted registered analog sources. There is no new manifest section, tool, importer, MCP command or board-specific code. See [Generic change](#generic-change).

## Source

- Board: [HeyPCB/esp32-sensors-1 at `57a2122a19ac1aa9c092e43fb5b123c461508752`](https://github.com/HeyPCB/esp32-sensors-1/tree/57a2122a19ac1aa9c092e43fb5b123c461508752/boards/dht11), asset `boards/dht11/kicad/esp32_dht11`, license CERN-OHL-S-2.0. [source-map.json](source-map.json) records the Git blob IDs. A regression checks the revision and license against the sibling LDR fixture's source map.
- Line: the pad nets come from the pinned PCB blob. U4 (DHT11, LCSC C117051) pad 2 is on `SNS_IO1`, pad 1 on `3V3`, pad 4 on `GND`, pad 3 not connected. `SNS_IO1` goes to U3 pad 4 (IO4) and to R6 (10 kΩ) pad 2; R6 pad 1 is on `3V3`. The README states "single-wire data on IO4 with a 10k pull-up".
- The source README states that the board was not fabricated, assembled, or powered. The board folder contains no firmware.

## Model

```yaml
cosim_models:
  - id: source-u4-dht11
    adapter: behavior
    step_ns: 1000
    inputs:
      DATA: source.U3.GPIO4          # host level: 0 pulls low, 1 releases
      temperature: environment.temperature
      humidity: environment.humidity
    outputs: { DATA: source.net.SNS_IO1 }   # the line
    config: { device_type: dht11 }
```

An input alias that names a descriptor pin role is the level the host drives; an input alias that names a descriptor input channel is a stimulus in that channel's unit (°C, %). The output is the descriptor's driven role. For an `open_drain_release` role the published level is the wired-AND of the host and the part, which stands for R6.

The firmware ([firmware.rs](firmware.rs), new for this fixture) holds the line low for the `setup` start length (18,000 µs in the two functional scripts), releases it, and polls it every 1 µs. It waits for the response (low, high, low) and then reads 40 bits; a high time over 48 µs is a 1. It checks the checksum, compares the humidity byte with 70 %RH (on) and 65 %RH (off), prints `T=<°C> C RH=<%> %`, and waits 1 s. `loop` returns 1,000,000 + RH × 1000 + T, or 1 (no response), 2 (timeout), 3 (checksum).

Assumptions and limits, also carried in `run_report`:

- The firmware is new portable behavior firmware. It is not firmware from the pinned source and not an ESP32-S3 binary.
- The model samples the line at each 1 µs boundary; a host level change reaches U4 at the next boundary. Firmware instructions take no virtual time.
- The model sends integer RH and T with zero decimal bytes, and clamps T to 0..50 °C.
- Not modeled: analog edges, the R6 current, line capacitance, input thresholds, power-on settling, the sensor's own sampling period, accuracy and response time, USB enumeration and host input.

## Expected values, calculated before the first run

Frame: `(RH, 0, T, 0, RH + T)`. Bit timing from the descriptor: 30 µs high, then 80 µs low and 80 µs high, then for each bit 50 µs low and 27 µs (0) or 70 µs (1) high, then 50 µs low. The start write is at the call time `t0`; U4 sees the release at `t0 + 18,001 µs` (one boundary later). The firmware reads the trailing low at `t0 + 18,001 + 190 + 40 × 50 + 40 × 27 + 43 × n1` µs = `t0 + 21,271 + 43 × n1` µs, where `n1` is the number of one bits in the frame. It then waits 1 s.

| Stimulus | Frame | n1 | Read ends |
| --- | --- | --- | --- |
| 25 °C, 55 %RH | 55, 0, 25, 0, 80 | 10 | 21,701 µs |
| 25 °C, 72 %RH | 72, 0, 25, 0, 97 | 8 | 1,043,316 µs |
| 25 °C, 68 %RH | 68, 0, 25, 0, 93 | 10 | 2,065,017 µs |
| 25 °C, 60 %RH | 60, 0, 25, 0, 85 | 11 | 3,086,761 µs |
| 60 °C, 0 %RH | 0, 0, 50, 0, 50 | 6 | 21,529 µs |
| -10 °C, 100 %RH | 100, 0, 0, 0, 100 | 6 | 1,043,058 µs |
| 24 °C, 55 %RH | 55, 0, 24, 0, 79 | 12 | 2,064,845 µs |

Each read puts 86 edges on `dht_line`: the host low and release, the 30 µs end, the two response edges, 80 bit edges, the end low and the idle high.

## Measured result

`labwired test`, debug CLI. All three scripts exit 0 on their first run. Every value above was observed as calculated.

| Script | Bench stimulus | Firmware result | `dht_line` edges | LED edges (drive) | Final time |
| --- | --- | --- | --- | --- | --- |
| `test-humidity-alarm.yaml` | 55 → 72 → 68 → 60 %RH at 25 °C | 1055025, 1072025, 1068025, 1060025 | 344 | 1,043,316,000 and 3,086,761,000 ns | 4,086,761,000 ns |
| `test-clamp-and-range.yaml` | 60 °C / 0 %, -10 °C / 100 %, 24 °C / 55 % | 1000050, 1100000, 1055024 | 258 | 1,043,058,000 and 2,064,845,000 ns | 3,064,845,000 ns |
| `test-short-start.yaml` (negative control) | 500 µs start, 90 %RH | 1 (no response) | 2 (host only, 1,000 and 501,000 ns) | none | 1,000,600,000 ns |

- Logic analyzer, first read: `dht_line` edges at 1,000, 18,001,000, 18,031,000, 18,111,000 and 18,191,000 ns, then 50 µs low and 27 µs high for the first bit (RH 55 starts with a 0). `dht_drive` has two edges per read. `user_led_state` follows each drive edge 1,000 ns later; at 68 %RH the indicator stays on (hysteresis).
- Console, first script: `T=25 C RH=55 %`, `T=25 C RH=72 %`, `T=25 C RH=68 %`, `T=25 C RH=60 %`. The negative control prints `no DHT11 response`.
- The negative control fails because the catalog model answers only a start low of at least 1 ms (`min_hold_us: 1000`). A stimulus outside the catalog channel range (for example 120 %RH) stops the run with a model error; the unit test `wrong_routes_and_out_of_range_stimulus_are_errors` checks this.
- Wall time, debug CLI, single runs: 3.96 s for the 4.1 s alarm script, 2.89 s, 0.96 s.

Browser run: see [browser-flex-scripts.json](../browser-flex-scripts.json).

## Generic change

`crates/core/src/cosim/gpio_device.rs`. Before this change the `behavior` adapter refused timers and schedules, and `config.device_type` reached only registered analog sources, so no catalog single-wire part could run on the Flex path. The adapter now looks the type up in the catalog. A `gpio_device` descriptor is built as the same `DeclarativeGpioDevice` the detailed path uses, with a 1 GHz device clock so a cycle is one Flex nanosecond and `timing: exact` schedules land on their own times. Every pin role must be routed; an unknown input alias, an output that is not a driven role, a pin list and an out-of-range stimulus are errors. The four unit tests in that file check the DHT11 frame bit by bit on the line, the short-start refusal, the wired-AND, and the errors.

## Onboarding effort

Measured once on 2026-11-20, approximate, from file and command times: about 10 min from the first fetch of the pinned board folder to three passing native scripts, including a 3 min 12 s debug CLI build. The generic adapter and its unit tests took about 13 min before that. No comparable time exists for a chip-specific model of this board, so these numbers support no speed claim.

A pass shows that this firmware satisfied the assertions against the declared model. It is not evidence for ESP32-S3 firmware, the GPIO peripheral, the analog line, sensor accuracy, or chip parity.
