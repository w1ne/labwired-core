# Mixed-signal board coverage

Coverage is strict: a profile is covered only when an analog device is driven through the in-core MNA netlist, its solved voltage is asserted after board pin/ADC routing, and the ADC result is asserted. The Arduino Uno test also executes the ATmega328P ADC register sequence. Unit tests, bare-chip tests, analog examples without an end-to-end assertion, and digital-only board tests do not count.

This inventory mirrors every `configs/systems/*.yaml` profile (103 at last update). Several profiles are demos, fixtures, or variants, so each file is listed independently to avoid silently treating one board test as coverage for another profile.

| System profile | Strict analog coverage |
| --- | --- |
| `adafruit-feather-esp32-v2` | NO end-to-end analog coverage |
| `adxl345-sensor-lab` | NO end-to-end analog coverage |
| `apollo4p-evb` | NO end-to-end analog coverage |
| `arduino-mega-2560` | NO end-to-end analog coverage |
| `arduino-nano-33-ble` | NO end-to-end analog coverage |
| `arduino-nano` | NO end-to-end analog coverage |
| `arduino-uno-r4-minima` | NO end-to-end analog coverage |
| `arduino-uno` | [`cosim_avr_analog_diode::arduino_uno_diode_netlist_reaches_a0_and_firmware_adc0`](../crates/core/tests/cosim_avr_analog_diode.rs) |
| `arduino-zero` | NO end-to-end analog coverage |
| `arty-litex-vexriscv` | NO end-to-end analog coverage |
| `atmega1281` | NO end-to-end analog coverage |
| `atmega2561` | NO end-to-end analog coverage |
| `brd2709a` | NO end-to-end analog coverage |
| `calliope-mini-v1` | NO end-to-end analog coverage |
| `cc2538dk` | NO end-to-end analog coverage |
| `ci-fixture-l476-readonly` | NO end-to-end analog coverage |
| `ci-fixture-riscv-uart1` | NO end-to-end analog coverage |
| `ci-fixture-uart1` | NO end-to-end analog coverage |
| `ci-fixture-unknown-arch` | NO end-to-end analog coverage |
| `curiosity-pic32mz-ef` | NO end-to-end analog coverage |
| `dspic33fj32mc202-demo` | NO end-to-end analog coverage |
| `dspic33fj32mc204-demo` | NO end-to-end analog coverage |
| `ek-ra6m5` | NO end-to-end analog coverage |
| `ek-ra8m1` | NO end-to-end analog coverage |
| `ek-rx261` | NO end-to-end analog coverage |
| `esp32-wroom-32` | NO end-to-end analog coverage |
| `esp32-wroom-epaper` | NO end-to-end analog coverage |
| `esp32c3-ble-pong` | NO end-to-end analog coverage |
| `esp32c3-devkit` | NO end-to-end analog coverage |
| `esp32c3-epaper-workshop` | NO end-to-end analog coverage |
| `esp32c3-jtag-watch` | NO end-to-end analog coverage |
| `esp32c3-mlx90640-thermal` | NO end-to-end analog coverage |
| `esp32c3-nokia5110-workshop` | NO end-to-end analog coverage |
| `esp32c3-oled-128x32-workshop` | NO end-to-end analog coverage |
| `esp32c3-oled-demo` | NO end-to-end analog coverage |
| `esp32c3-tm1637-clock-workshop` | NO end-to-end analog coverage |
| `esp32c6-devkitc` | NO end-to-end analog coverage |
| `esp32s3-oled-demo` | NO end-to-end analog coverage |
| `esp32s3-zero` | NO end-to-end analog coverage |
| `esp32s3` | NO end-to-end analog coverage |
| `feather-f405` | NO end-to-end analog coverage |
| `frdm-k64f` | NO end-to-end analog coverage |
| `frdm-kw41z-lcd` | NO end-to-end analog coverage |
| `frdm-kw41z` | NO end-to-end analog coverage |
| `iolink-multiport-demo` | NO end-to-end analog coverage |
| `iolinki-g0-startup` | NO end-to-end analog coverage |
| `kv260-r5` | NO end-to-end analog coverage |
| `labwired-demo-board` | NO end-to-end analog coverage |
| `litex-vexriscv` | NO end-to-end analog coverage |
| `luckfox-pico-mini-a` | NO end-to-end analog coverage |
| `mb1355c` | NO end-to-end analog coverage |
| `metro-m4` | NO end-to-end analog coverage |
| `microbit-v1` | NO end-to-end analog coverage |
| `microbit-v2` | NO end-to-end analog coverage |
| `mimxrt595-evk` | NO end-to-end analog coverage |
| `mpfs-disco-kit` | NO end-to-end analog coverage |
| `nano-33-iot` | NO end-to-end analog coverage |
| `nrf52-dk` | NO end-to-end analog coverage |
| `nrf52840-dk` | NO end-to-end analog coverage |
| `nrf5340dk` | NO end-to-end analog coverage |
| `nrf54l15dk` | NO end-to-end analog coverage |
| `nrf54lm20dk` | NO end-to-end analog coverage |
| `nucleo-f103rb-epaper` | NO end-to-end analog coverage |
| `nucleo-f103rb` | [`cosim_pin_routing::nucleo_f103rb_ntc_netlist_reaches_pa0_adc1_end_to_end`](../crates/core/tests/cosim_pin_routing.rs) |
| `nucleo-f401re` | NO end-to-end analog coverage |
| `nucleo-f407` | NO end-to-end analog coverage |
| `nucleo-f767zi` | NO end-to-end analog coverage |
| `nucleo-g071rb` | NO end-to-end analog coverage |
| `nucleo-h563zi-demo` | NO end-to-end analog coverage |
| `nucleo-l073rz` | NO end-to-end analog coverage |
| `nucleo-l476rg` | NO end-to-end analog coverage |
| `nucleo-l552ze-q` | NO end-to-end analog coverage |
| `nucleo-u545re` | NO end-to-end analog coverage |
| `nucleo-u575zi` | NO end-to-end analog coverage |
| `nucleo-u5a5zj` | NO end-to-end analog coverage |
| `nucleo_g474re` | NO end-to-end analog coverage |
| `nucleo_wba52cg` | NO end-to-end analog coverage |
| `pic16f1829-curiosity` | NO end-to-end analog coverage |
| `pic16f877a-demo` | NO end-to-end analog coverage |
| `pic18f4520-demo` | NO end-to-end analog coverage |
| `pico-selftest` | NO end-to-end analog coverage |
| `pico` | NO end-to-end analog coverage |
| `pybadge` | NO end-to-end analog coverage |
| `rp2040-pico` | NO end-to-end analog coverage |
| `rp2350-zero` | NO end-to-end analog coverage |
| `s32k388-evb` | NO end-to-end analog coverage |
| `sam-e70-xplained` | NO end-to-end analog coverage |
| `sam4s-xplained` | NO end-to-end analog coverage |
| `seeed-xiao-nrf52840-sense` | NO end-to-end analog coverage |
| `simctl-selftest` | NO end-to-end analog coverage |
| `sk-am62` | NO end-to-end analog coverage |
| `smart-ring` | NO end-to-end analog coverage |
| `stm32f072b-disco` | NO end-to-end analog coverage |
| `stm32f103-bare` | NO end-to-end analog coverage |
| `stm32f103-integrated-test` | NO end-to-end analog coverage |
| `stm32f4-discovery` | NO end-to-end analog coverage |
| `stm32f401cdu6-blackpill` | NO end-to-end analog coverage |
| `stm32f401cdu6` | NO end-to-end analog coverage |
| `stm32f411ceu6-blackpill` | NO end-to-end analog coverage |
| `stm32f7-discovery` | NO end-to-end analog coverage |
| `stm32h735-smoke` | NO end-to-end analog coverage |
| `teensy-41` | NO end-to-end analog coverage |
| `vegaboard-ri5cy` | NO end-to-end analog coverage |

## Current covered board/device slices

- **Arduino Uno R3 / diode load line:** generic MNA diode model → PC0/ADC0 descriptor route → ATmega328P conversion. Example manifest: [analog-diode-system.yaml](../examples/arduino-uno-io/analog-diode-system.yaml).
- **NUCLEO-F103RB / NTC divider:** generic Beta NTC netlist → PA0/ADC1 route → STM32F103 ADC conversion. Example manifest: [analog-ntc-system.yaml](../examples/nucleo-f103rb/analog-ntc-system.yaml).

All rows marked `NO end-to-end analog coverage` remain gaps; the next round should take one of those profiles and preserve this strict criterion.

