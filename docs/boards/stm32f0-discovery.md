# STM32F0DISCOVERY — LD3 GPIO smoke

The STM32F0DISCOVERY uses an STM32F051R8T6. This LabWired system profile
models the RCC gate for GPIOC and the GPIOC output used by green user LED LD3
on PC9, as documented in ST's
[UM1525](https://www.st.com/resource/en/user_manual/um1525-stm32f0discovery-discovery-kit-for-stm32-f0-microcontrollers-stmicroelectronics.pdf).

## Live product status

ST listed both the STM32F0DISCOVERY kit and STM32F051R8 MCU as **Active** and
in volume production on 2026-11-04. ST's eStore listing showed the board **In
Stock**, at **$32.73 for quantity 2**, with **Add to Cart** enabled. See the
[vendor evidence](../../examples/stm32f0-discovery/evidence/vendor-status.md).

## Smoke evidence

The local onboarding CLI smoke requires at least eight GPIOC.9 output edges
and stops at the instruction limit. Its committed run is recorded in
[`result.json`](../../examples/stm32f0-discovery/evidence/simulation-output/result.json).
This is a simulator smoke result, not a physical-board capture.

## Model boundary

The chip descriptor includes only RCC, GPIOC, and the 64 KiB flash / 8 KiB
SRAM required by the firmware. The board profile maps only LD3 on PC9. It
omits LD4, buttons, ST-LINK/V2, USB, other GPIO ports, timers, USART, ADC, and
all other STM32F051 peripherals. The Part Knowledge catalog has no exact
STM32F051R8 record yet; this smoke uses a narrow register profile sourced from
ST's UM1525 and RM0091.

The reproducible smoke is in
[`examples/stm32f0-discovery/`](../../examples/stm32f0-discovery/).
