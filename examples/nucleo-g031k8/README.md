# NUCLEO-G031K8

This Nucleo-32 profile boots the STM32G031K8T6U and exercises two real board
paths: USART2 output through the ST-LINK virtual COM port and the active-high
LD3 user LED on PC6. The chip descriptor is intentionally narrow; it exposes
only reset-clock RCC, GPIOA, GPIOC, and USART2. Other peripherals and package
pins are outside this profile.

## Reproduce

From the repository root:

```bash
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id nucleo-g031k8 \
  --crate firmware-nucleo-g031k8-smoke \
  --target thumbv6m-none-eabi \
  --script examples/nucleo-g031k8/onboarding-smoke.yaml \
  --system configs/systems/nucleo-g031k8.yaml \
  --out-dir examples/nucleo-g031k8/evidence
```

The smoke requires `NUCLEO_G031K8_OK` on USART2 and at least 16 PC6 GPIO
transitions. Its build logs, simulator result, and onboarding metrics are
committed under [`evidence/`](evidence/). This is local engine evidence, not a
physical-board capture or a hosted `labwired_verify` verdict.

## Board connections and limits

| Board signal | MCU pin | Modeled path |
| --- | --- | --- |
| ST-LINK VCP TX/RX | PA2 / PA3 | USART2, AF1 |
| LD3 green user LED | PC6 | Active-high GPIOC output |

ST UM2591 Rev 2 identifies the default PC6 LED solder-bridge configuration
and PA2/PA3 USART2 VCP route. The ST-LINK USB bridge and its transport are not
simulated. No zero-answering peripheral stubs are declared; firmware accesses
outside this profile remain unsupported.

## Vendor status

Live ST product and ordering status checked 2026-11-04 UTC is recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Board sources

- [ST UM2591 Rev 2, STM32G0 Nucleo-32 board (MB1455)](https://www.st.com/resource/en/user_manual/dm00622380-stm32g0-nucleo-32-board-mb1455-stmicroelectronics.pdf)
- [ST MB1455-G031K8-D01 schematic, 5 Feb 2026](https://www.st.com/resource/en/schematic_pack/mb1455-g031k8-d01-schematic.pdf)
- [ST STM32G031K8 product page and datasheet](https://www.st.com/en/microcontrollers-microprocessors/stm32g031k8.html)
- [ST RM0444, STM32G0x1 reference manual](https://www.st.com/resource/en/reference_manual/rm0444-stm32g0x1-advanced-armbased-32bit-mcus-stmicroelectronics.pdf)
