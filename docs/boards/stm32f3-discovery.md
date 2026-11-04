# STM32F3DISCOVERY — LD3 GPIO smoke

The board uses an STM32F303VCT6. This LabWired profile models the reset-clock
gate for GPIOE and the GPIOE output used by user LED LD3 on PE9. ST's board
manual identifies that LED mapping. [UM1570](https://www.st.com/resource/en/user_manual/um1570-discovery-kit-with-stm32f303vc-mcu-stmicroelectronics.pdf)

## Live product status

ST listed STM32F3DISCOVERY as **Active** and in volume production. Its eStore
listing showed **In Stock** with **Add to cart** enabled when checked on
2026-11-04. ST also listed STM32F303VCT6 as **Active** and **In Stock**.
See the [vendor evidence](../../examples/stm32f3-discovery/evidence/vendor-status.md).

## Smoke evidence

The local onboarding CLI smoke passed both assertions: at least eight GPIOE.9
transitions and the expected stop at the instruction limit. The recorded run
contains 11 PE9 transitions across 2,200 steps:
[`result.json`](../../examples/stm32f3-discovery/evidence/simulation-output/result.json).
This is a simulator smoke result, not a physical-board capture.

## Model boundary

The profile omits the other seven user LEDs, user button, USB device, ST-LINK,
VCP, gyroscope, and e-compass. It maps the 40 KiB main SRAM region and omits
the separate 8 KiB CCM bank. The smoke result also records that no power net is
routed to the MCU; power-on and brown-out behavior are therefore not
simulated. No physical ST-LINK or VCP path is represented.

The reproducible smoke and full limitations are in
[`examples/stm32f3-discovery/`](../../examples/stm32f3-discovery/).
