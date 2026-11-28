# mb1355c

This example was synthesized by Foundry as a board onboarding starter.

## Scope

- App-core boot path
- RCC/GPIO/UART baseline
- Board LED and user button mapping
- BLE example selection reference only

## Recommended ST Example

- STM32CubeWB BLE_p2pServer
- STM32CubeWB BLE_LLD_Pressbutton

## Required Source Confirmation

- Confirm MCU part number and package
- Confirm VCP UART instance on ST-LINK
- Confirm LED and button GPIO mappings from schematic
- Confirm whether BLE scope is documentation-only or full simulator behavior

## Auto-Resolved Source Docs

- `board_user_manual` required: https://www.st.com/resource/en/user_manual/um2819-stm32wb-nucleo64-board-mb1355-stmicroelectronics.pdf
- `board_schematic` required: https://www.st.com/resource/en/schematic_pack/mb1355-wb55rg-d01_schematic.pdf
- `mcu_datasheet` required: https://www.st.com/resource/en/datasheet/stm32wb55rg.pdf
- `reference_manual` required: https://www.st.com/resource/en/reference_manual/rm0434-stm32wb55xx-stm32wb35xx-advanced-armbased-32bit-mcus-stmicroelectronics.pdf
- `vendor_examples` required: https://github.com/STMicroelectronics/STM32CubeWB
- `vendor_example` required: https://github.com/STMicroelectronics/STM32CubeWB/tree/master/Projects/P-NUCLEO-WB55.Nucleo/Applications/BLE/BLE_p2pServer
- `vendor_example` recommended: https://github.com/STMicroelectronics/STM32CubeWB/tree/master/Projects/P-NUCLEO-WB55.Nucleo/Applications/BLE/BLE_LLD_Pressbutton

## Analog inputs

`analog-ntc-diode-pa0-pa1-system.yaml` drives a generic Beta-parameter NTC
divider into `board.analog.pa0_volts` (ADC1_IN5) and a Shockley diode load
line into `board.analog.pa1_volts` (ADC1_IN6). The descriptor's
`analog_pins:` map is the VFQFPN68 ADC1 set from ST STM32_open_pin_data
(PC0-PC3 = IN1-IN4, PA0-PA7 = IN5-IN12, PC4/PC5 = IN13/IN14,
PA8/PA9 = IN15/IN16), so a deck can route to any of these pads. The test is
`crates/core/tests/analog/cosim_stm32wb_g4_l0_analog.rs`
(`cargo test -p labwired-core --test analog cosim_stm32wb_g4_l0`).
