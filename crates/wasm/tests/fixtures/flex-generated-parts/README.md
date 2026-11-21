# Generated parts, catalog data and sinks in one Flex device

An aquarium controller whose every part runs without per-part engine code:

| Part | How it runs |
| --- | --- |
| Heater relay module | `parts:` pack `gen:relay-module-active-low`, a `gpio_device` generated from catalog facts and carried inline in `system.yaml` |
| DS18B20 water probe | catalog descriptor `ds18b20` on the generic `onewire_device` primitive, a co-simulation model on a bit-banged 1-Wire line |
| LCD1602 I2C | catalog descriptor `hd44780_pcf8574`, a `display` with `strobed_nibbles` framing on the Flex I2C bus |
| WS2812 ring | catalog `neopixel` as a `sinks:` entry: every edge at its exact time, the frames kept in `frame_history` |
| TM1637 display | catalog `tm1637-7seg` as a `sinks:` entry: a listen-only `gpio_device` whose digits are device state |

`firmware.rs` reads the probe (reset, SKIP ROM, CONVERT T, poll, READ SCRATCHPAD with its CRC), switches the heater below 20.00 °C, and shows the reading on the LCD, the TM1637 and the ring. The scripts check the result with `device_state` assertions, which read what each device holds (`result.json`'s `devices`).

| Script | Water | Result | Heater | LCD | TM1637 |
| --- | --- | --- | --- | --- | --- |
| `test-cold-water.yaml` | 18.5 °C | 296 | on | `Water 18.50C` / `Heater ON` | `1850` |
| `test-warm-water.yaml` | 25.25 °C | 404 | off | `Water 25.25C` / `Heater off` | `2525` |

## Scope

Functional behaviour only. Not modelled: pull-ups and line electrical behaviour, DS18B20 parasite power and accuracy, HD44780 CGRAM and timing, WS2812 supply current, relay coil and pump physics. See `assumptions` and `unsupported_checks` in `system.yaml`.

## Reproduce

```sh
sh crates/wasm/tests/fixtures/flex-generated-parts/build.sh
cargo test -p labwired-core --features flex-script --lib -- flex_parts_tests
cargo run -p labwired-cli --bin labwired -- test --no-key \
  --script crates/wasm/tests/fixtures/flex-generated-parts/test-cold-water.yaml \
  --output-dir /tmp/generated-parts
```
