# NUCLEO-L4A6ZG onboarding smoke

This focused profile boots bare-metal firmware for the STM32L4A6ZGT6 and
exercises the active-high LD3 user LED on PB14. The profile declares only the
modeled RCC and GPIOB paths needed by that test; it does not claim the board's
VCP, USB OTG, buttons, or the MCU's other peripherals.
The focused memory map exposes the 256 KiB SRAM1 region only; SRAM2 is omitted.

## Run

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-l4a6zg \
  --crate firmware-nucleo-l4a6zg-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-l4a6zg/onboarding-smoke.yaml \
  --system configs/systems/nucleo-l4a6zg.yaml \
  --out-dir examples/nucleo-l4a6zg/evidence
```

The committed local simulator log and generated run artifacts are under
[`evidence/`](evidence/). This is a local deterministic simulation smoke, not a
hosted `labwired_verify` result or a physical-board capture.

## Pin map and scope

| Board signal | MCU pin | Model path |
|---|---|---|
| Red user LED LD3 | PB14 | RCC + GPIOB output |

ST's UM2179 Rev 10 §7.6 documents LD3 on PB14 and says the user LEDs turn on
when the I/O is high. The smoke profile uses that board-wired LED only. Other
header pins and board peripherals remain undeclared here.

The board and MCU do not currently have entries in LabWired Part Knowledge.
The pin map and model profile here are sourced from ST's own board manual,
datasheet, and reference manual; they are not presented as a Part Knowledge
cross-check.

## Sources

- ST [NUCLEO-L4A6ZG product page](https://www.st.com/en/evaluation-tools/nucleo-l4a6zg.html)
  and [live vendor status receipt](evidence/vendor-status.md).
- ST [NUCLEO-L4A6ZG data brief](https://www.st.com/resource/en/data_brief/nucleo-l4a6zg.pdf):
  the mounted STM32L4A6ZGT6 device.
- ST [UM2179 Rev 10, STM32 Nucleo-144 boards (MB1312)](https://www.st.com/resource/en/user_manual/um2179-stm32-nucleo144-boards-mb1312-stmicroelectronics.pdf),
  §§7.6 and 7.13: LD3/PB14 connection, active-high LED behavior, default solder
  bridge configuration.
- ST [STM32L4A6ZG datasheet](https://www.st.com/resource/en/datasheet/stm32l4a6zg.pdf)
  and [RM0351 Rev 10 reference manual](https://www.st.com/resource/en/reference_manual/dm00083560-stm32l47xxx-stm32l48xxx-stm32l49xxx-and-stm32l4axxx-advanced-armbased-32bit-mcus-stmicroelectronics.pdf):
  chip memory and STM32L4 RCC/GPIO register layout.
