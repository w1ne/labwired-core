# Mixed-signal board coverage

This inventory tracks every system profile in `configs/systems/*.yaml` (including demos,
fixtures, and multiple profiles for the same physical board). A profile is covered only when
a board-specific test solves an analog device/netlist, routes the solved signal to that
profile's ADC input, and asserts the converter result. Generic chip bus tests, analog output
traces without an ADC conversion, and device-only unit tests do not count.

Current inventory: **103 profiles; 14 have end-to-end analog coverage; 89 have no end-to-end analog coverage.** Each round selects an uncovered profile with a modeled ADC and a resolvable board pin or explicit ADC-channel route; profiles with a stub or unmodeled ADC remain listed as uncovered.

| System profile | End-to-end proof |
|---|---|
| `adafruit-feather-esp32-v2` | [`cosim_esp32_analog_diode.rs`](../crates/core/tests/cosim_esp32_analog_diode.rs)::`feather_esp32_diode_netlist_reaches_sens_sar_channel_and_one_shot_result` |
| `adxl345-sensor-lab` | [`cosim_pin_routing.rs`](../crates/core/tests/cosim_pin_routing.rs)::`adxl345_sensor_lab_diode_netlist_reaches_pa6_adc1_end_to_end` |
| `apollo4p-evb` | **NO E2E analog coverage** |
| `arduino-mega-2560` | [`cosim_pin_routing.rs`](../crates/core/tests/cosim_pin_routing.rs)::`arduino_mega_2560_ntc_netlist_reaches_adc8_through_mux5_end_to_end` |
| `arduino-nano-33-ble` | [`cosim_pin_routing.rs`](../crates/core/tests/cosim_pin_routing.rs)::`arduino_nano_33_ble_ntc_netlist_reaches_saadc_ain0_end_to_end` — explicit SAADC AIN0 route and two-channel EasyDMA scan |
| `arduino-nano` | [`cosim_pin_routing.rs`](../crates/core/tests/cosim_pin_routing.rs)::`arduino_nano_ntc_netlist_reaches_pc0_adc0_end_to_end` |
| `arduino-uno-r4-minima` | **NO E2E analog coverage** |
| `arduino-uno` | [`cosim_avr_analog_diode.rs`](../crates/core/tests/cosim_avr_analog_diode.rs)::`arduino_uno_diode_netlist_reaches_a0_and_firmware_adc0` |
| `arduino-zero` | **NO E2E analog coverage** |
| `arty-litex-vexriscv` | **NO E2E analog coverage** |
| `atmega1281` | [`cosim_pin_routing.rs`](../crates/core/tests/cosim_pin_routing.rs)::`atmega1281_ntc_netlist_reaches_adc0_end_to_end` |
| `atmega2561` | [`cosim_pin_routing.rs`](../crates/core/tests/cosim_pin_routing.rs)::`atmega2561_diode_netlist_reaches_adc0_end_to_end` — diode load line through explicit ADC0 route and firmware-visible AVR ADC conversion |
| `brd2709a` | [`cosim_pin_routing.rs`](../crates/core/tests/cosim_pin_routing.rs)::`brd2709a_diode_netlist_reaches_pd2_iadc0_end_to_end` |
| `calliope-mini-v1` | **NO E2E analog coverage** |
| `cc2538dk` | **NO E2E analog coverage** |
| `ci-fixture-l476-readonly` | **NO E2E analog coverage** |
| `ci-fixture-riscv-uart1` | **NO E2E analog coverage** |
| `ci-fixture-uart1` | **NO E2E analog coverage** |
| `ci-fixture-unknown-arch` | **NO E2E analog coverage** |
| `curiosity-pic32mz-ef` | **NO E2E analog coverage** |
| `dspic33fj32mc202-demo` | **NO E2E analog coverage** |
| `dspic33fj32mc204-demo` | **NO E2E analog coverage** |
| `ek-ra6m5` | [`cosim_pin_routing.rs`](../crates/core/tests/cosim_pin_routing.rs)::`ek_ra6m5_ntc_and_diode_netlists_reach_adc1_end_to_end` — Beta NTC and Shockley diode netlists route to ADC1 channels 0/2 and produce 12-bit scan results |
| `ek-ra8m1` | [`cosim_pin_routing.rs`](../crates/core/tests/cosim_pin_routing.rs)::`ek_ra8m1_ntc_and_diode_netlists_reach_adc1_end_to_end` — the same generic Beta NTC and Shockley diode netlists route to ADC1 channels 0/2 and produce 12-bit scan results |
| `ek-rx261` | **NO E2E analog coverage** |
| `esp32-wroom-32` | [`cosim_esp32_analog_diode.rs`](../crates/core/tests/cosim_esp32_analog_diode.rs)::`esp32_wroom_32_ntc_netlist_reaches_sens_sar_channel_and_one_shot_result` — Beta NTC divider reaches the profile's SENS SAR ADC1 channel 6 and one-shot result |
| `esp32-wroom-epaper` | **NO E2E analog coverage** |
| `esp32c3-ble-pong` | **NO E2E analog coverage** |
| `esp32c3-devkit` | **NO E2E analog coverage** |
| `esp32c3-epaper-workshop` | **NO E2E analog coverage** |
| `esp32c3-jtag-watch` | **NO E2E analog coverage** |
| `esp32c3-mlx90640-thermal` | **NO E2E analog coverage** |
| `esp32c3-nokia5110-workshop` | **NO E2E analog coverage** |
| `esp32c3-oled-128x32-workshop` | **NO E2E analog coverage** |
| `esp32c3-oled-demo` | **NO E2E analog coverage** |
| `esp32c3-tm1637-clock-workshop` | **NO E2E analog coverage** |
| `esp32c6-devkitc` | **NO E2E analog coverage** |
| `esp32s3-oled-demo` | **NO E2E analog coverage** |
| `esp32s3-zero` | **NO E2E analog coverage** |
| `esp32s3` | **NO E2E analog coverage** |
| `feather-f405` | **NO E2E analog coverage** |
| `frdm-k64f` | **NO E2E analog coverage** |
| `frdm-kw41z-lcd` | **NO E2E analog coverage** |
| `frdm-kw41z` | **NO E2E analog coverage** |
| `iolink-multiport-demo` | **NO E2E analog coverage** |
| `iolinki-g0-startup` | **NO E2E analog coverage** |
| `kv260-r5` | **NO E2E analog coverage** |
| `labwired-demo-board` | **NO E2E analog coverage** |
| `litex-vexriscv` | **NO E2E analog coverage** |
| `luckfox-pico-mini-a` | **NO E2E analog coverage** |
| `mb1355c` | **NO E2E analog coverage** |
| `metro-m4` | **NO E2E analog coverage** |
| `microbit-v1` | **NO E2E analog coverage** |
| `microbit-v2` | **NO E2E analog coverage** |
| `mimxrt595-evk` | **NO E2E analog coverage** |
| `mpfs-disco-kit` | **NO E2E analog coverage** |
| `nano-33-iot` | **NO E2E analog coverage** |
| `nrf52-dk` | **NO E2E analog coverage** |
| `nrf52840-dk` | **NO E2E analog coverage** |
| `nrf5340dk` | **NO E2E analog coverage** |
| `nrf54l15dk` | **NO E2E analog coverage** |
| `nrf54lm20dk` | **NO E2E analog coverage** |
| `nucleo-f103rb-epaper` | **NO E2E analog coverage** |
| `nucleo-f103rb` | [`cosim_pin_routing.rs`](../crates/core/tests/cosim_pin_routing.rs)::`nucleo_f103rb_ntc_netlist_reaches_pa0_adc1_end_to_end` |
| `nucleo-f401re` | [`cosim_pin_routing.rs`](../crates/core/tests/cosim_pin_routing.rs)::`nucleo_f401re_diode_netlist_reaches_pa0_adc1_end_to_end` |
| `nucleo-f407` | **NO E2E analog coverage** |
| `nucleo-f767zi` | **NO E2E analog coverage** |
| `nucleo-g071rb` | **NO E2E analog coverage** |
| `nucleo-h563zi-demo` | **NO E2E analog coverage** |
| `nucleo-l073rz` | **NO E2E analog coverage** |
| `nucleo-l476rg` | **NO E2E analog coverage** |
| `nucleo-l552ze-q` | **NO E2E analog coverage** |
| `nucleo-u545re` | **NO E2E analog coverage** |
| `nucleo-u575zi` | **NO E2E analog coverage** |
| `nucleo-u5a5zj` | **NO E2E analog coverage** |
| `nucleo_g474re` | **NO E2E analog coverage** |
| `nucleo_wba52cg` | **NO E2E analog coverage** |
| `pic16f1829-curiosity` | **NO E2E analog coverage** |
| `pic16f877a-demo` | **NO E2E analog coverage** |
| `pic18f4520-demo` | **NO E2E analog coverage** |
| `pico-selftest` | **NO E2E analog coverage** |
| `pico` | **NO E2E analog coverage** |
| `pybadge` | **NO E2E analog coverage** |
| `rp2040-pico` | **NO E2E analog coverage** |
| `rp2350-zero` | **NO E2E analog coverage** |
| `s32k388-evb` | **NO E2E analog coverage** |
| `sam-e70-xplained` | **NO E2E analog coverage** |
| `sam4s-xplained` | **NO E2E analog coverage** |
| `seeed-xiao-nrf52840-sense` | **NO E2E analog coverage** |
| `simctl-selftest` | **NO E2E analog coverage** |
| `sk-am62` | **NO E2E analog coverage** |
| `smart-ring` | **NO E2E analog coverage** |
| `stm32f072b-disco` | **NO E2E analog coverage** |
| `stm32f103-bare` | **NO E2E analog coverage** |
| `stm32f103-integrated-test` | **NO E2E analog coverage** |
| `stm32f4-discovery` | **NO E2E analog coverage** |
| `stm32f401cdu6-blackpill` | **NO E2E analog coverage** |
| `stm32f401cdu6` | **NO E2E analog coverage** |
| `stm32f411ceu6-blackpill` | **NO E2E analog coverage** |
| `stm32f7-discovery` | **NO E2E analog coverage** |
| `stm32h735-smoke` | **NO E2E analog coverage** |
| `teensy-41` | **NO E2E analog coverage** |
| `vegaboard-ri5cy` | **NO E2E analog coverage** |

The EK-RA6M5 and EK-RA8M1 proofs reuse the same generic Beta NTC and Shockley diode netlists and explicit RA ADC12 channel routing; both profiles lack pin-to-ADC aliases for these channels. The shared fixture is [examples/ek-ra6m5/analog-ntc-system.yaml](../examples/ek-ra6m5/analog-ntc-system.yaml).
