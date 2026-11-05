# Mixed-signal board coverage

This inventory tracks every system profile in `configs/systems/*.yaml` (including demos,
fixtures, and multiple profiles for the same physical board). A profile is covered only when
a board-specific test solves an analog device/netlist, routes it to a modeled board endpoint,
and asserts the result. Endpoints include an ADC conversion or a voltage-thresholded digital
input on profiles without an ADC. Generic chip bus tests, analog output traces without a board
endpoint, and device-only unit tests do not count.

Current inventory: **103 profiles; 38 have end-to-end analog coverage; 65 have no end-to-end analog coverage.** Each round selects the next uncovered profile and extends shared device, netlist, or routing support where needed. Coverage through GPIO thresholds does not imply ADC support.

| System profile | End-to-end proof |
|---|---|
| `adafruit-feather-esp32-v2` | [`cosim_esp32_analog_diode.rs`](../crates/core/tests/cosim_esp32_analog_diode.rs)::`feather_esp32_diode_netlist_reaches_sens_sar_channel_and_one_shot_result` |
| `adxl345-sensor-lab` | [`cosim_pin_routing.rs`](../crates/core/tests/cosim_pin_routing.rs)::`adxl345_sensor_lab_diode_netlist_reaches_pa6_adc1_end_to_end` |
| `apollo4p-evb` | [`cosim_apollo4_analog.rs`](../crates/core/tests/cosim_apollo4_analog.rs)::`apollo4p_evb_ntc_and_diode_netlists_reach_adc_slots_and_scan_fifo` — generic Beta NTC and unrelated Shockley diode are solved into ADC SE0/SE1 and asserted through the scan FIFO; adjacent SE2 is checked undriven |
| `arduino-mega-2560` | [`cosim_pin_routing.rs`](../crates/core/tests/cosim_pin_routing.rs)::`arduino_mega_2560_ntc_netlist_reaches_adc8_through_mux5_end_to_end` |
| `arduino-nano-33-ble` | [`cosim_pin_routing.rs`](../crates/core/tests/cosim_pin_routing.rs)::`arduino_nano_33_ble_ntc_netlist_reaches_saadc_ain0_end_to_end` — explicit SAADC AIN0 route and two-channel EasyDMA scan |
| `arduino-nano` | [`cosim_pin_routing.rs`](../crates/core/tests/cosim_pin_routing.rs)::`arduino_nano_ntc_netlist_reaches_pc0_adc0_end_to_end` |
| `arduino-uno-r4-minima` | [`cosim_ra4m1_analog.rs`](../crates/core/tests/cosim_ra4m1_analog.rs)::`uno_r4_minima_ntc_and_diode_netlists_reach_a0_a1_and_adc14` — generic Beta NTC and Shockley diode netlists resolve through A0/A1 pin mappings and convert on ADC14 at 12 and 14 bits |
| `arduino-uno` | [`cosim_avr_analog_diode.rs`](../crates/core/tests/cosim_avr_analog_diode.rs)::`arduino_uno_diode_netlist_reaches_a0_and_firmware_adc0` |
| `arduino-zero` | [`cosim_samd21_analog.rs`](../crates/core/tests/cosim_samd21_analog.rs)::`arduino_zero_ntc_and_diode_netlists_reach_distinct_samd21_adc_pads` — generic Beta NTC and unrelated Shockley diode netlists route through PA02/AIN0 and PB08/AIN2 into asserted 12/10/8-bit conversions; undriven AIN3 stays at zero |
| `arty-litex-vexriscv` | [`cosim_arty_litex_analog.rs`](../crates/core/tests/cosim_arty_litex_analog.rs)::`arty_litex_ntc_and_diode_netlists_drive_cas_gpio_inputs_through_thresholds` — generic Beta NTC divider to SW0 and unrelated Shockley diode load line to BTN0; asserts solved voltages, CAS readbacks, and Schmitt hysteresis. This is GPIO threshold coverage; the profile has no mapped ADC. |
| `atmega1281` | [`cosim_pin_routing.rs`](../crates/core/tests/cosim_pin_routing.rs)::`atmega1281_ntc_netlist_reaches_adc0_end_to_end` |
| `atmega2561` | [`cosim_pin_routing.rs`](../crates/core/tests/cosim_pin_routing.rs)::`atmega2561_diode_netlist_reaches_adc0_end_to_end` — diode load line through explicit ADC0 route and firmware-visible AVR ADC conversion |
| `brd2709a` | [`cosim_pin_routing.rs`](../crates/core/tests/cosim_pin_routing.rs)::`brd2709a_diode_netlist_reaches_pd2_iadc0_end_to_end` |
| `calliope-mini-v1` | [`cosim_calliope_mini_analog.rs`](../crates/core/tests/cosim_calliope_mini_analog.rs)::`calliope_mini_ntc_and_diode_netlists_reach_distinct_adc_channels` — generic Beta NTC and unrelated Shockley diode netlists reach nRF51 ADC channels 0/1 and produce asserted 10-bit firmware-visible results; no analog pad aliases or silicon ADC validation are claimed |
| `cc2538dk` | [`cosim_cc2538_analog.rs`](../crates/core/tests/cosim_cc2538_analog.rs)::`cc2538dk_ntc_and_diode_netlists_drive_gpio_inputs_through_thresholds` — generic Beta NTC and Shockley diode netlists route to PC4/PC5 and assert GPIO DATA levels, threshold crossings, and retained hysteresis; this is GPIO threshold coverage, not ADC support |
| `curiosity-pic32mz-ef` | [`cosim_curiosity_pic32mz_analog.rs`](../crates/core/tests/cosim_curiosity_pic32mz_analog.rs)::`curiosity_pic32mz_generic_ntc_and_diode_reach_gpio_thresholds` — generic Beta NTC and Shockley diode netlists reach bounded GPIOB inputs and assert solved voltages, PORTB threshold crossings, and hysteresis. The thresholds are simulation assumptions; this is not silicon validation or ADC support. |
| `ci-fixture-l476-readonly` | [`cosim_nucleo_l476_analog.rs`](../crates/core/tests/cosim_nucleo_l476_analog.rs)::`nucleo_l476rg_profiles_route_ntc_and_diode_to_gpio_thresholds` — same generic NTC and Shockley diode solve and reach GPIOA input thresholds on the read-only L476 fixture |
| `ci-fixture-riscv-uart1` | [`cosim_ci_fixture_riscv_analog.rs`](../crates/core/tests/cosim_ci_fixture_riscv_analog.rs)::`ci_fixture_riscv_ntc_and_diode_netlists_reach_gpio_thresholds` — generic Beta NTC and unrelated Shockley diode netlists reach separate synthetic GPIOA inputs; asserts solved voltages, input levels, and Schmitt hysteresis. This CI-only endpoint is simulation-only, with no hardware claim. |
| `ci-fixture-uart1` | [`cosim_ci_fixture_uart1_analog.rs`](../crates/core/tests/cosim_ci_fixture_uart1_analog.rs)::`ci_fixture_uart1_ntc_and_diode_netlists_reach_gpio_thresholds` — generic Beta NTC and unrelated Shockley diode netlists reach separate synthetic GPIOA inputs; asserts solved voltages, input levels, and Schmitt hysteresis. This CI-only endpoint is simulation-only, with no hardware claim. |
| `ci-fixture-unknown-arch` | [`cosim_ci_fixture_unknown_arch_analog.rs`](../crates/core/tests/cosim_ci_fixture_unknown_arch_analog.rs)::`ci_fixture_unknown_arch_ntc_and_diode_netlists_reach_gpio_thresholds` — generic Beta NTC and Shockley diode netlists route through separate synthetic GPIO inputs; asserts solved voltages, GPIO levels, and Schmitt hysteresis. This is direct SystemBus fixture coverage only; World still rejects the fixture's unknown architecture, so it cannot execute firmware. |
| `dspic33fj32mc202-demo` | [`cosim_dspic33fj32mc202_analog.rs`](../crates/core/tests/cosim_dspic33fj32mc202_analog.rs)::`dspic33fj32mc202_ntc_and_diode_netlists_reach_gpio_thresholds` — generic Beta NTC and Shockley diode netlists reach separate GPIOB inputs; asserts solved voltages, PORTB levels, and Schmitt hysteresis. GPIO thresholds are simulation assumptions; this is not ADC support or silicon validation. |
| `dspic33fj32mc204-demo` | [`cosim_dspic33fj32mc204_analog.rs`](../crates/core/tests/cosim_dspic33fj32mc204_analog.rs)::`dspic33fj32mc204_ntc_and_diode_netlists_reach_gpio_thresholds` — generic Beta NTC and Shockley diode netlists reach separate GPIOB inputs; asserts solved voltages, PORTB levels, and Schmitt hysteresis. GPIO thresholds are simulation assumptions; this is not ADC support or silicon validation. |
| `ek-ra6m5` | [`cosim_pin_routing.rs`](../crates/core/tests/cosim_pin_routing.rs)::`ek_ra6m5_ntc_and_diode_netlists_reach_adc1_end_to_end` — Beta NTC and Shockley diode netlists route to ADC1 channels 0/2 and produce 12-bit scan results |
| `ek-ra8m1` | [`cosim_pin_routing.rs`](../crates/core/tests/cosim_pin_routing.rs)::`ek_ra8m1_ntc_and_diode_netlists_reach_adc1_end_to_end` — the same generic Beta NTC and Shockley diode netlists route to ADC1 channels 0/2 and produce 12-bit scan results |
| `ek-rx261` | [`cosim_ek_rx261_analog.rs`](../crates/core/tests/cosim_ek_rx261_analog.rs)::`ek_rx261_ntc_and_diode_netlists_reach_simulated_gpio_thresholds` — generic NTC and unrelated Shockley diode netlists reach two input-only simulation endpoints; asserts solved voltages, threshold levels, and hysteresis. This is direct SystemBus coverage only: PORT/MPC registers and firmware GPIO access remain unsupported, and thresholds are simulation assumptions. |
| `esp32-wroom-32` | [`cosim_esp32_analog_diode.rs`](../crates/core/tests/cosim_esp32_analog_diode.rs)::`esp32_wroom_32_ntc_netlist_reaches_sens_sar_channel_and_one_shot_result` — Beta NTC divider reaches the profile's SENS SAR ADC1 channel 6 and one-shot result |
| `esp32-wroom-epaper` | [`cosim_esp32_analog_diode.rs`](../crates/core/tests/cosim_esp32_analog_diode.rs)::`esp32_wroom_epaper_ntc_and_diode_reach_distinct_sens_adc_channels` — generic Beta NTC and unrelated Shockley diode netlists reach SENS SAR ADC1 channels 6/7; independent voltage equations, channel isolation, and 12-bit one-shot results are asserted |
| `esp32c3-ble-pong` | [`cosim_esp32c3_analog.rs`](../crates/core/tests/cosim_esp32c3_analog.rs)::`esp32c3_ble_pong_ntc_and_diode_netlists_reach_adc1_channels` — generic Beta NTC and unrelated Shockley diode netlists route through the explicit ADC1_CH0/CH2 endpoints; solved voltages, channel isolation, the profile's existing GP2Y0A21 input on ADC1_CH1, and firmware-visible APB_SARADC one-shot results are asserted |
| `esp32c3-devkit` | [`cosim_esp32c3_analog.rs`](../crates/core/tests/cosim_esp32c3_analog.rs)::`esp32c3_devkit_ntc_and_diode_netlists_reach_adc1_channels` — generic Beta NTC and unrelated Shockley diode netlists solve to explicit ADC1_CH0/CH2 routes; the independent voltages, channel isolation, undriven CH1/CH3, and APB_SARADC one-shot results are asserted |
| `esp32c3-epaper-workshop` | [`cosim_esp32c3_analog.rs`](../crates/core/tests/cosim_esp32c3_analog.rs)::`esp32c3_epaper_workshop_ntc_and_diode_netlists_reach_adc1_channels` — generic Beta NTC and unrelated Shockley diode netlists solve to ADC1_CH0/CH2 on this e-paper profile; independent voltages, channel isolation, undriven CH1/CH3, and APB_SARADC one-shot results are asserted |
| `esp32c3-jtag-watch` | [`cosim_esp32c3_analog.rs`](../crates/core/tests/cosim_esp32c3_analog.rs)::`esp32c3_jtag_watch_ntc_and_diode_netlists_reach_adc1_channels` — generic Beta NTC and unrelated Shockley diode netlists solve to ADC1_CH0/CH2; independent voltages, channel isolation, undriven CH1/CH3, and APB_SARADC one-shot results are asserted |
| `esp32c3-mlx90640-thermal` | [`cosim_esp32c3_analog.rs`](../crates/core/tests/cosim_esp32c3_analog.rs)::`esp32c3_mlx90640_thermal_ntc_and_diode_netlists_reach_adc1_channels` — preserves the profile's MLX90640 I²C device while generic Beta NTC and unrelated Shockley diode netlists solve to ADC1_CH0/CH2; independent voltages, channel isolation, undriven CH1/CH3, and APB_SARADC one-shot results are asserted |
| `esp32c3-nokia5110-workshop` | [`cosim_esp32c3_analog.rs`](../crates/core/tests/cosim_esp32c3_analog.rs)::`esp32c3_nokia5110_workshop_ntc_and_diode_netlists_reach_adc1_channels` — preserves the profile's PCD8544 SPI display while generic Beta NTC and unrelated Shockley diode netlists solve to ADC1_CH0/CH2; independent voltages, channel isolation, undriven CH1/CH3, and APB_SARADC one-shot results are asserted |
| `esp32c3-oled-128x32-workshop` | [`cosim_esp32c3_analog.rs`](../crates/core/tests/cosim_esp32c3_analog.rs)::`esp32c3_oled_128x32_workshop_ntc_and_diode_netlists_reach_adc1_channels` — preserves the profile's SSD1306 I²C display while generic Beta NTC and unrelated Shockley diode netlists solve to ADC1_CH0/CH2; independent voltages, channel isolation, undriven CH1/CH3, and APB_SARADC one-shot results are asserted |
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
| `nucleo-l476rg` | [`cosim_nucleo_l476_analog.rs`](../crates/core/tests/cosim_nucleo_l476_analog.rs)::`nucleo_l476rg_profiles_route_ntc_and_diode_to_gpio_thresholds` — generic NTC divider and Shockley diode solve onto PA0/PA1; asserts solved voltages, GPIO input levels, and Schmitt hysteresis. This is GPIO threshold coverage, not ADC support. |
| `nucleo-l552ze-q` | [`cosim_nucleo_l552_analog.rs`](../crates/core/tests/cosim_nucleo_l552_analog.rs)::`nucleo_l552ze_q_ntc_and_diode_netlists_reach_gpio_thresholds` — generic Beta NTC and Shockley diode netlists solve onto PA0/PA1; asserts voltages, GPIOA_IDR levels, and Schmitt hysteresis. This is GPIO threshold coverage; ADC1 remains a stub. |
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

The EK-RA6M5 and EK-RA8M1 proofs reuse the same generic Beta NTC and Shockley diode netlists and explicit RA ADC12 channel routing; both profiles lack pin-to-ADC aliases for these channels. The shared fixture is [examples/ek-ra6m5/analog-ntc-system.yaml](../examples/ek-ra6m5/analog-ntc-system.yaml). Apollo4 Plus uses the same generic analog adapter with an explicit channel route; its model currently covers single scans of external slots and FIFO output.

EK-RX261 uses the reusable input-only `input_pin_bank` endpoint because its PORT/MPC register map remains a stub. It demonstrates the shared MNA-to-thresholded-input route for two unrelated devices, but does not model silicon GPIO thresholds, analog pad muxing, ADC conversion, or firmware-visible port registers.
