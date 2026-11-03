# NUCLEO-WBA55CG onboarding smoke

This board pairs the STM32WBA55CG with the Nucleo-64 user LEDs, buttons, and
ST-LINK V3E debugger. This narrow profile models the three user LED GPIOs, the
three user button inputs, and the default ST-LINK Virtual COM Port (VCP1) on
USART1. The firmware configures USART1 on PA8/PB12, prints
`NUCLEO_WBA55CG_OK`, and toggles LD1 on PB4.

The STM32WBA55CG radio, Bluetooth LE and 802.15.4 stacks, second VCP (LPUART1,
which is not connected by default), USB/ST-LINK behavior, and other unlisted
peripherals are outside this profile. UART output is captured from the modeled
USART1; this smoke does not simulate USB enumeration or the ST-LINK bridge.

Run from the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-wba55cg \
  --crate firmware-nucleo-wba55cg-smoke \
  --target thumbv8m.main-none-eabi \
  --script examples/nucleo-wba55cg/onboarding-smoke.yaml \
  --system configs/systems/nucleo-wba55cg-smoke.yaml \
  --out-dir examples/nucleo-wba55cg/evidence
```

The committed smoke evidence is under [`evidence/`](evidence/). The live ST
status and availability check is in [`evidence/vendor-status.md`](evidence/vendor-status.md).

## Pin map

| Signal | MCU pin | Board connection |
| --- | --- | --- |
| USART1 RX | PA8, AF7 | ST-LINK VCP1 |
| USART1 TX | PB12, AF7 | ST-LINK VCP1 |
| LD1 blue | PB4 | Active-low user LED |
| LD2 green | PA9 | Active-low user LED |
| LD3 red | PB8 | Active-low user LED |
| B1 | PC13 | User button |
| B2 | PB6 | User button |
| B3 | PB7 | User button |

## Sources

- ST [UM3301 Rev 2](https://www.st.com/resource/en/user_manual/um3301-stm32wba-nucleo64-board-mb1801-and-mb1803-stmicroelectronics.pdf), §§7.4.4, 7.5, 7.6: default VCP1, LED and button pin mappings.
- ST [STM32WBA5xxx datasheet DS14127](https://www.st.com/resource/en/datasheet/stm32wba55cg.pdf): WBA55 memory, alternate functions and peripheral register details.
- ST [RM0493](https://www.st.com/resource/en/reference_manual/rm0493-stm32wba52xx-and-stm32wba54xx-advanced-armbased-32bit-mcus-stmicroelectronics.pdf): WBA RCC and GPIO register layout.
