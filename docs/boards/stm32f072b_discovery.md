# Imported STM32F072B-DISCO onboarding smoke

The imported profile is bounded to the Cortex-M0, 128 KiB flash, 16 KiB SRAM,
F0 RCC, GPIOA, GPIOC, and USART1. Other peripherals in the original Renode
descriptor were removed because this engine does not model them for this
profile.

The exact imported-system firmware ran for 4,000,000 instructions. It enabled
the F0 GPIOA/GPIOC and USART1 clock gates, transmitted its smoke text through
USART1, read GPIOC ODR after setting the four LED output bits, and read PA0
high with its internal pull-up enabled. The JUnit report passed all three
smoke assertions. The GPIOC values are register readbacks; this run did not
model visible LED output. The PA0 readback does not exercise a pressed button
or an external button connection.

The run assumes an ideal power rail, so power-on and brown-out behavior are
outside this result. This is local LabWired execution against the imported
system manifest. It does not establish Renode equivalence, whole-board
behavior, unsupported peripheral behavior, or physical hardware confirmation.

Result JSON, UART output, snapshot, JUnit report, and build/run logs are under
[`docs/coverage/renode-onboarding/metrics/stm32f072b_discovery/`](../coverage/renode-onboarding/metrics/stm32f072b_discovery/).
