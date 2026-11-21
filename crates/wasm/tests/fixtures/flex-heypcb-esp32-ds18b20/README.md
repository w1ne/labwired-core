# HeyPCB ESP32-S3 DS18B20 on a Flex 1-Wire line

Portable Rust-to-Wasm firmware reads the Maxim DS18B20 thermometer of the pinned **HeyPCB ESP32-S3 DS18B20** board, revision A. It bit-bangs the 1-Wire line at standard speed with the AN126 slot times: reset and presence, READ ROM with the CRC-8 check, WRITE SCRATCHPAD (TH 30 °C, TL 28 °C, resolution), a read-back of the scratchpad, then once per `loop` CONVERT T, busy polling with read slots, and READ SCRATCHPAD with the CRC-8 check. It prints each reading to a Flex console that stands for the board's native USB serial port, and drives the IO48 user LED as an over-temperature indicator with the TH/TL hysteresis. No ESP32-S3 CPU or GPIO controller is emulated.

This board is a held-out workload, and the first board workload on the `onewire_device` primitive (#2137). U4 is the unchanged catalog `ds18b20` device model (`configs/devices/ds18b20.yaml`). The board needed no Rust model code, no runtime or grammar change, no new manifest section, tool, importer, MCP command or board-specific code.

## Source

- Board: [HeyPCB/esp32-sensors-1 at `57a2122a`](https://github.com/HeyPCB/esp32-sensors-1/tree/57a2122a19ac1aa9c092e43fb5b123c461508752/boards/ds18b20), asset `boards/ds18b20/kicad/esp32_ds18b20`, license CERN-OHL-S-2.0. [source-map.json](source-map.json) records the Git blob IDs of the fetched files. A regression checks the revision and license against the sibling LDR fixture's source map, and the line binding against the source map.
- Line: the pad nets come from the pinned PCB blob. U4 (TO-92) pad 1 is on GND, pad 2 (DQ) on `SNS_IO1`, pad 3 on 3V3. U3 pad 4 (IO4) is on `SNS_IO1`. R6 (4.7 kΩ) pulls `SNS_IO1` to 3V3. U3 pad 25 is on IO48, which drives D1 through R4 (1 kΩ). The README states "1-Wire on IO4 with a 4.7k pull-up, VDD powered".
- Part: the BOM fits `DS18B20(UMW)` (LCSC C376006), which the README calls the "UMW second source of the Maxim part". The model follows the Maxim datasheet.
- The source README states that the board was not fabricated, assembled, or powered. The board folder contains no firmware.

## Model

`system.yaml` places the catalog `ds18b20` model on the `behavior` adapter. Its DQ input is the IO4 drive, its DQ output publishes `SNS_IO1` (the wired-AND of IO4 and U4), and its `temperature` input is a write port in °C. The model's ROM code is `28 01 00 00 00 00 00 29` (family 0x28, serial 1, CRC 0x29). Its scratchpad powers on at +85 °C (0x0550). CONVERT T takes 93.75 ms << R1R0 from the falling edge of its last command bit, and read slots return 0 until then. At 9 to 11 bits the lowest temperature bits are cleared.

The `probe_connected` write port drives the model's `connected` input (#2138). At 0 it stands for an open DQ connection, such as a cracked pad 2 joint: U4 gives no presence pulse and releases DQ, but stays powered and keeps its scratchpad. A probe that loses power and powers on again is not modelled.

Gaps, explicit in `unsupported_checks`: parasite power, ALARM SEARCH and the alarm flag, the analog line waveform and pull-up rise time, accuracy and self-heating.

## Expected values, calculated before the first run

A reset takes 960 µs (480 low, 70 to the presence sample, 410 more). Every write and read slot takes 70 µs, so a byte takes 560 µs. `setup` takes 16.88 ms: three resets, READ ROM and 8 bytes, SKIP ROM + WRITE SCRATCHPAD + 3 bytes, SKIP ROM + READ SCRATCHPAD + 9 bytes. In `loop`, the last CONVERT T bit falls 2.01 ms in; polls open every 1.07 ms from 2.08 ms. At 12 bits, poll 701 (at 752.15 ms) is the first after the 750 ms conversion, and the loop takes 759.34 ms. At 9 bits, poll 88 (at 96.24 ms) is the first after 93.75 ms, and the loop takes 103.43 ms. `loop` returns the 1/16 °C count as 16 bits, plus 0x10000 while the indicator is on. An LED write reaches `user_led_state` 1,000 ns later.

| Script | Stimulus | `loop` results | LED drive edges (ns) | Final time (ns) |
| --- | --- | --- | --- | --- |
| `test-over-temperature.yaml` | 25, 31.0625, 29, 27.5, -10.125 °C, 12 bits | 400, 66033, 66000, 440, 65374 (0xFF5E) | 1,535,560,000 (on), 3,054,240,000 (off) | 3,813,580,000 |
| `test-9-bit-resolution.yaml` | 25.0625, 25.5625 °C, 9 bits | 400, 408 (low 3 bits clear) | none | 223,740,000 |
| `test-power-on-85c.yaml` | scratchpad read before any conversion, 25 °C | `peek` 1360 (+85 °C), `loop` 400 | none | 783,340,000 |
| `test-open-dq.yaml` | 25 °C; DQ open, 31.0625 °C; DQ closed; `peek` | 400, 0x8000 (no presence, one 960 µs reset), 66033, `peek` 497 | 1,536,520,000 (on) | 1,543,640,000 |
| `test-wrong-family.yaml` (negative control) | firmware expects family 0x10 (DS18S20) | `setup` 3, `loop` 0x8000 | none | 6,000,000 |

## Measured result

`labwired test`, debug CLI, 2026-11-21. The first four scripts passed on the first run with the calculated values: 13, 6, 5 and 6 assertions.

- Logic analyzer: `user_led_drive` edges at 1,535,560,000 and 3,054,240,000 ns, `user_led_state` 1,000 ns later; the TH/TL hysteresis keeps the indicator on at 29 °C. 17,002 digital edges in the over-temperature run (0 dropped), 294 in the wrong-family run: 146 IO4 drive edges (one reset, 72 slots) and 148 line edges (the same plus the presence pulse).
- Device state: U4 ran 12 function commands in the over-temperature run (WRITE and READ SCRATCHPAD in `setup`, CONVERT T and READ SCRATCHPAD per `loop`), the last READ_SCRATCHPAD; none in the wrong-family run (READ ROM is the engine ROM layer).
- Console, over-temperature script: `ROM 28 01 00 00 00 00 00 29`, `DS18B20 12-bit, TH 30 C, TL 28 C`, `T=25.0000 C after 701 polls`, `T=31.0625 C after 701 polls HIGH`, `T=29.0000 C after 701 polls HIGH`, `T=27.5000 C after 701 polls`, `T=-10.1250 C after 701 polls`.
- Wall time, debug CLI, single runs: 2.74 s for 3.81 s of virtual time (over-temperature), 0.62 s, 0.26 s and 0.04 s for the other three.
- Open DQ (`test-open-dq.yaml`, added 2026-11-21 after #2138, firmware unchanged): 13 of 13 assertions pass. While DQ is open, `SNS_IO1` falls only for the 480 µs IO4 reset pulse (776,221,000 to 776,701,000 ns) and no presence pulse follows; `loop` returns 0x8000 and prints nothing, and the indicator keeps its state. After reconnection the 12-bit conversion takes 701 polls again, so U4 kept the resolution written in `setup`. Two prediction errors before the pass, both in the script, not in the model or firmware: the first version asserted `user_led_state` at the final time, but D1 follows the drive 1,000 ns after the run had ended (8 of 9 passed); the added `peek` call runs one more READ SCRATCHPAD (7 function commands, not 6). Wall time 1.37 s, debug CLI.
- Onboarding effort: 5 min 57 s from the fetch of the pinned source (14:01:37 UTC) to four passing scripts (14:07:35 UTC). That includes a 2 min 43 s cold debug CLI build. Reused: the SCD41 fixture's manifest and firmware skeleton and the 1-Wire bit-bang code of the `flex-generated-parts` fixture.

Negative controls (`/tmp` copies, not committed):

- 29.9375 °C instead of 31.0625 °C in the over-temperature script: 5 of 13 assertions fail. `loop` returns 479 and the drive gets no edge.
- 12 bits instead of 9 in the 9-bit script: 4 of 6 assertions fail. The first `loop` returns 401 and the run stops at `max_time_ns` (999,860,000 ns).
- Family 0x28 expected in the wrong-family script: 5 of 6 assertions fail. `setup` returns 0 and U4 runs 3 function commands.
- `probe_connected` kept at 1 in the open-DQ script: 9 of 13 assertions fail. The second `loop` converts and returns 66033, U4's presence pulse shows on `SNS_IO1` from 776,731,000 to 776,851,000 ns, and the third conversion runs the script into `max_time_ns`.

## Scope

This is functional portable behavior simulation of one board's sensor path. It does not run original ESP32-S3 firmware, does not model the ESP32-S3 GPIO or RMT peripherals, does not cover the full board, and does not establish chip parity.

## Rebuild

`sh build.sh` rebuilds `firmware.wasm` with `rustc --target wasm32-unknown-unknown`; run `chmod 644 firmware.wasm` after it.
