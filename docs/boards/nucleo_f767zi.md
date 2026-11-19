# Imported NUCLEO-F767ZI onboarding smoke

The imported onboarding profile is bounded to Cortex-M7 execution, 2 MiB of
flash at `0x08000000`, the 384 KiB SRAM window used by Renode's STM32F746 CPU
map, RCC, GPIOI, and USART1. The MCU's broader silicon memory includes TCM and
backup SRAM that this profile does not map. The [Renode STM32F746 CPU map](https://github.com/renode/renode/blob/master/platforms/cpus/stm32f746.repl)
defines the imported flash and SRAM windows; ST's [STM32F767 datasheet](https://www.st.com/resource/en/datasheet/stm32f767zi.pdf)
describes the additional TCM and backup memory. Other peripherals in the
generated descriptor were removed because this smoke does not exercise them.

The exact imported-system smoke runs the Cortex-M7 firmware for 800,000
instructions. It checks the `OK` USART1 output and an edge on GPIOI pin 1 while
the firmware configures PI1 and sets it through BSRR. It then stops at the
configured instruction limit. The JUnit report records the assertion results.
The run assumes an ideal power rail; power-on and brown-out behavior are not
tested.

This is local LabWired CLI evidence against the imported system manifest. It
does not validate the whole STM32F767 platform, the unlisted peripherals,
Renode equivalence, or physical hardware.

Result JSON, UART output, GPIO traces, snapshot, JUnit report, and build/run
logs are under
[`docs/coverage/renode-onboarding/metrics/nucleo_f767zi/`](../coverage/renode-onboarding/metrics/nucleo_f767zi/).
