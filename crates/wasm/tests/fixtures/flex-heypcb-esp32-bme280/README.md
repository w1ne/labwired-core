# HeyPCB ESP32-S3 BME280 over a Flex I2C bus

Portable Rust-to-Wasm firmware reads the BME280 of the pinned **HeyPCB ESP32-S3 BME280** board, revision A, through a Flex I2C bus. The sensor is the catalog `bme280` device model, the model that the detailed path attaches to an MCU's I2C controller. The firmware drives the IO48 user LED as an over-temperature indicator. No ESP32-S3 CPU or I2C controller is emulated.

## Source

- Board: [HeyPCB/esp32-sensors-1 at `57a2122a19ac1aa9c092e43fb5b123c461508752`](https://github.com/HeyPCB/esp32-sensors-1/tree/57a2122a19ac1aa9c092e43fb5b123c461508752/boards/bme280), asset `boards/bme280/kicad/esp32_bme280`, license CERN-OHL-S-2.0. [source-map.json](source-map.json) records the Git blob IDs. A regression checks the revision and license against the sibling LDR fixture's source map.
- Bus: the pad nets come from the pinned PCB blob. U4 pad 3 is on `I2C_SDA` and pad 4 on `I2C_SCL`. Pad 2 (CSB) is on 3V3, which selects I2C. Pad 5 (SDO) is on GND, which selects address 0x76. `I2C_SDA` goes to U3 pad 12 (IO8) and `I2C_SCL` to U3 pad 17 (IO9). R6 and R7 (4.7 kΩ) pull both lines to 3V3.
- The source README states that the board was not fabricated, assembled, or powered. The board folder contains no firmware.

The Library lane runs ESP32-S3 target firmware for the same asset (`heypcb:HeyPCB/esp32-sensors-1:boards/bme280/kicad/esp32_bme280`) on the detailed path. This fixture does not replace that run and proves nothing about it.

## Model

```yaml
i2c_buses:
  - name: sensor_i2c
    frequency_hz: 100000
    devices:
      - id: source-u4-bme280
        type: bme280
        config: { i2c_address: 0x76 }
        inputs:
          temperature: environment.temperature
          humidity: environment.humidity
          pressure: environment.pressure
```

`i2c_buses` is a generic key of the Flex manifest. The device `type` is a catalog type, built by the factory that `external_devices` uses. No code in this repository names the board.

The firmware ([firmware.rs](firmware.rs), 234 lines, new for this fixture) calls `labwired.i2c_transfer`. `setup` reads the chip ID (0xD0), reads the calibration (0x88, 0xA1, 0xE1), and writes `ctrl_hum`, `ctrl_meas` and `config`. `loop` reads the eight data bytes at 0xF7 and compensates temperature, pressure and humidity with the fixed-point reference arithmetic of the Bosch BME280 datasheet. It returns the temperature in 0.01 °C plus 10,000. The indicator goes on at 30.00 °C and off below 28.00 °C.

Assumptions and limits, also carried in `run_report`:

- The firmware is new portable behavior firmware. It is not firmware from the pinned source and not an ESP32-S3 binary.
- The bus speed is 100 kHz. The pinned source states no bus speed.
- A transfer takes one SCL period for each bit, START and STOP. Firmware instructions take no virtual time.
- The path exchanges whole bytes with the device model. It does not model the SCL and SDA waveforms on IO9 and IO8, the R6 and R7 pull-ups, bus capacitance, or clock stretching. `run_report.i2c_buses[].not_modeled` lists these.
- The BME280 model has no measurement time, standby time or IIR filter. A sample follows its input at once.
- The model uses the catalog's shipped calibration coefficients, not those of a physical part.

## Measured result

`labwired test`, debug CLI, all three scripts exit 0:

| Script | Ambient set by the test bench | Firmware result | I2C transfers | Final time |
| --- | --- | --- | --- | --- |
| `test-ambient-and-alarm.yaml` | 25 → 31.5 → 29 → 27 °C | 12500, 13150, 12900, 12700 (25.00, 31.50, 29.00, 27.00 °C) | 12 acknowledged at 0x76, 0 refused | 15,140,000 ns |
| `test-pressure-humidity.yaml` | -10 °C, 80 %RH, 900 hPa | 9000 (-10.00 °C), 90000 Pa, 8000 (80.00 %RH) | 8 acknowledged | 7,060,000 ns |
| `test-wrong-address.yaml` (negative control) | 31.5 °C | `setup` returns 1, `loop` returns 0 | 1 refused at 0x77, 0 acknowledged | 110,000 ns |

- The firmware computes each value from the raw sample and the calibration bytes it read over the bus. Each result equals the value that the test bench set.
- Logic analyzer, first script: `user_led_drive` rises at 8,080,000 ns, the end of the transfer that reads 31.5 °C, and falls at 14,140,000 ns, the end of the transfer that reads 27 °C. At 29 °C the indicator stays on. `user_led_state` follows 1,000 ns later.
- Transfer time: `setup` is 7 transfers and 504 bit times, 5,040,000 ns at 100 kHz. One `loop` is 102 bit times plus a 1 ms period, 2,020,000 ns.
- `flex-trace.json` holds each transfer with its bytes. The first is a write of `[0xD0]` and a read of `[0x60]`.
- Sensor removed (unit test): with no device on the bus, a probe of 0x76 is refused.
- 200 °C on `ambient_c` stops the run with `model_error`: the BME280 model takes -40 to 85 °C.

A pass shows that this firmware satisfied the assertions against the declared model. It is not evidence for ESP32-S3 firmware, the I2C controller, the board's bus wiring under electrical load, or chip parity.
