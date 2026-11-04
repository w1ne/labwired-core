# NUCLEO-L010RB

This narrow Nucleo-64 profile boots an STM32L010RBT6 image and toggles the
board's active-high user LED LD2 on PA5 (Arduino D13). The descriptor exposes
only the reset-clock RCC and GPIOA paths used by this smoke. It makes no claim
about the button, ST-LINK VCP, or other MCU peripherals.

Run from the repository root:

```sh
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id nucleo-l010rb \
  --crate firmware-nucleo-l010rb-smoke \
  --target thumbv6m-none-eabi \
  --script examples/nucleo-l010rb/onboarding-smoke.yaml \
  --system configs/systems/nucleo-l010rb.yaml \
  --out-dir examples/nucleo-l010rb/evidence
```

The committed build and simulator output is under [`evidence/`](evidence/).
This is local engine evidence, not a physical-board capture.
The hosted LabWired board catalog currently has no L010RB target, so this lane
does not claim a hosted `labwired_verify` `model_verified` verdict.

## Modeled board path

| Board signal | MCU pin | Modeled path |
| --- | --- | --- |
| LD2 green user LED / Arduino D13 | PA5 | GPIOA output, active high |

The profile declares no zero-answering peripheral stubs. Firmware that accesses
an omitted MCU peripheral remains unsupported by this profile.

## Sources

- ST's [NUCLEO-L010RB product page](https://www.st.com/en/evaluation-tools/nucleo-l010rb.html)
  identifies the board and its STM32L010RB MCU.
- ST's [STM32L010RB product page](https://www.st.com/en/microcontrollers-microprocessors/stm32l010rb.html)
  lists the MCU memory and Cortex-M0+ family details.
- ST [UM1724 Rev 17](https://www.st.com/resource/en/user_manual/um1724-stm32-nucleo64-boards-mb1136-stmicroelectronics.pdf)
  identifies NUCLEO-L010RB as an MB1136 board and maps D13/LD2 to PA5; a high
  GPIO level lights the user LED.
- ST [RM0451](https://www.st.com/resource/en/reference_manual/dm00443854-ultra-low-power-stm32l0x0-advanced-arm-based-32-bit-mcus-stmicroelectronics.pdf)
  documents the STM32L0x0 RCC and GPIO register map.
- Live vendor status and orderability are recorded in
  [`evidence/vendor-status.md`](evidence/vendor-status.md).
