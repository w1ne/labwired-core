# HeyPCB ESP32-S3 SCD41 on a Flex I2C bus

Portable Rust-to-Wasm firmware reads the Sensirion SCD41 CO2 sensor of the pinned **HeyPCB ESP32-S3 SCD41** board, revision A, through a Flex I2C bus. It sends the 16-bit commands of the SCD4x datasheet: `stop_periodic_measurement`, `get_serial_number`, `start_periodic_measurement`, then once per 5 s measurement interval `get_data_ready_status` and `read_measurement`. It checks the Sensirion CRC-8 of every word, converts CO2, temperature and humidity, prints each reading to a Flex console that stands for the board's native USB serial port, and drives the IO48 user LED as a high-CO2 indicator (on at 1000 ppm, off below 800 ppm). No ESP32-S3 CPU or I2C controller is emulated.

This board is a held-out workload. U4 is the unchanged catalog `scd41` device model (`configs/devices/scd41.yaml`). The board needed no Rust model code, no runtime or grammar change, no new manifest section, tool, importer, MCP command or board-specific code.

## Source

- Board: [HeyPCB/esp32-sensors-1 at `57a2122a`](https://github.com/HeyPCB/esp32-sensors-1/tree/57a2122a19ac1aa9c092e43fb5b123c461508752/boards/scd41), asset `boards/scd41/kicad/esp32_scd41`, license CERN-OHL-S-2.0. [source-map.json](source-map.json) records the Git blob IDs; each one was checked with `git hash-object` on the fetched file. A regression checks the revision and license against the sibling LDR fixture's source map.
- Bus: the pad nets come from the pinned PCB blob. U4 pad 8 is on `I2C_SCL` and pad 10 on `I2C_SDA`; U3 pad 17 (IO9) is on `I2C_SCL`, pad 12 (IO8) on `I2C_SDA`, pad 25 on IO48. R6 and R7 (4.7 kΩ) pull the two lines to 3V3. The README states "fixed address 0x62".
- Part: the BOM and README fit SCD41-D-R2 (LCSC C3659294). The schematic symbol is `Sensor_Gas:SCD40-D-R2`; the SCD40 and SCD41 share the address and the commands this firmware uses.
- The source README states that the board was not fabricated, assembled, or powered. The board folder contains no firmware.

## Model

`system.yaml` places the catalog `scd41` model at 0x62 on a 100 kHz bus and binds its `co2`, `temperature` and `humidity` inputs to three write ports. The model answers with the encodings of its descriptor: CO2 in ppm as the word, `T = -45 + 175 · word / 65535`, `RH = 100 · word / 65535`, each word followed by its CRC. The serial number is the model's constant `0x4c454f310041`.

Gap of the catalog model, explicit in `unsupported_checks`: the data-ready status is the constant `0x8006` (always ready). The part gives its first result 5 s after `start_periodic_measurement` and then one result every 5 s. The model also answers every command at once, where the datasheet gives 500 ms after `stop_periodic_measurement` and 1 ms after the read commands. The firmware waits those times itself.

## Expected values, calculated before the first run

A transfer takes one SCL period for each bit, START and STOP: 290 µs for a 2-byte command, 380 µs for a 3-byte read, 920 µs for a 9-byte read, 110 µs for a refused address. `setup` takes 502.79 ms. Each `loop` takes 5003.88 ms: the 5 s interval, two commands, two 1 ms waits and both reads. An LED write reaches `user_led_state` 1,000 ns later. `loop` returns CO2 in ppm × 10 plus the indicator state.

| Script | Stimulus | `loop` results | LED drive edges (ns) | Final time (ns) |
| --- | --- | --- | --- | --- |
| `test-co2-alarm.yaml` | 450, 1200, 900, 700, 700 ppm | 4500, 12001, 9001, 7000, 7000 | 10,510,550,000 (on), 20,518,310,000 (off) | 25,522,190,000 |
| `test-below-threshold.yaml` (negative control) | 450, 950 (-10 °C, 80 %RH), 999 ppm | 4500, 9500, 9990 | none | 15,514,430,000 |
| `test-wrong-address.yaml` (negative control) | address 0x61 | `setup` 1, `loop` 0 | none | 110,000 |
| `test-read-before-first-interval.yaml` (model-gap record) | poll 1 ms after the start | 4500 (silicon: 1, `SCD41 not ready`) | none | 507,670,000 |

The temperature words are 26214 at 25 °C and 13107 at -10 °C; the humidity words are 32768 at 50 %RH and 52428 at 80 %RH. All decode to the stimulus to 0.01.

## Measured result

`labwired test`, debug CLI, 2026-11-21. The first run failed. Both causes were defects in this fixture, not in LabWired:

- Firmware defect: `advance_ns` takes a `u32`, and the firmware passed the 5 s interval as 5,000,000,000 ns. The value wrapped to 705,032,704 ns. The alarm script gave LED edges at 1,920,615,408 and 3,338,440,816 ns and a final time of 4,047,353,520 ns, against the calculated 10,510,550,000, 20,518,310,000 and 25,522,190,000 ns. The firmware now waits in whole seconds and a remainder. The timing assertions caught it; the value assertions did not.
- Script defect: the model-gap script had a 100 ms `max_time_ns`, shorter than the 502.79 ms `setup`. It stopped with `max_time_ns` at 290,000 ns. The limit is now 1 s.

After the two fixes, all four scripts pass with the calculated values: 13, 9, 8 and 5 assertions.

- Logic analyzer: `user_led_drive` edges at 10,510,550,000 and 20,518,310,000 ns, `user_led_state` 1,000 ns later; hysteresis keeps the indicator on at 900 ppm.
- I2C: 24 acknowledged transfers to 0x62 in the alarm script, 16 in the below-threshold script, 8 in the model-gap script; one refused transfer to 0x61.
- Console, alarm script: `SCD41 serial 0x4c454f310041 at 0x62`, `CO2=450 ppm T=25.00 C RH=50.00 %`, `CO2=1200 ppm T=25.00 C RH=50.00 % HIGH`, `CO2=900 ppm ... HIGH`, then `CO2=700 ppm T=25.00 C RH=50.00 %` twice.
- Wall time, debug CLI, single runs: 4.85 s for 25.5 s of virtual time (alarm), 3.17 s for 15.5 s (below threshold), 0.11 s and 0.02 s for the other two.

Negative controls (`/tmp` copies, not committed):

- 1000 ppm instead of 999 ppm in the below-threshold script: 3 of 9 assertions fail. `loop` returns 10001 and the drive gets one edge at 15,514,430,000 ns.
- Port `co2_ppm` initial 451 instead of 450 in the alarm script: 2 of 13 assertions fail (`loop` 4510 and the console text).
- Port `co2_ppm` with unit `%` instead of `ppm`: exit 2, `config_error`, `I2C device 'source-u4-scd41' input 'co2' takes unit "ppm", but 'environment.co2' produces Some("%"); no implicit unit conversion`.

`test-read-before-first-interval.yaml` passes because the model is wrong in a known way. When the catalog model gets the 5 s cadence, this script must fail, and its expected result changes to 1.

## Onboarding effort

Measured once on 2026-11-21, from file times in one lane session: 10 min 5 s from the fetch of the pinned board files (06:27:03 UTC) to four passing scripts (06:37:08 UTC). This includes a 6 min 54 s debug CLI build and both fixture defects. The firmware and scripts reuse the SHT40 fixture as a template. No comparable time exists for a chip-specific model of this board, so these numbers support no speed claim.

A pass shows that this firmware satisfied the assertions against the declared model. It is not evidence for ESP32-S3 firmware, the I2C controller, SCD41 timing or accuracy, or chip parity.
