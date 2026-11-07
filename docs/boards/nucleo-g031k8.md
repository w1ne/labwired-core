# NUCLEO-G031K8

The NUCLEO-G031K8 is an ST Nucleo-32 board built around the STM32G031K8T6U.
Its narrow LabWired profile is in [`configs/systems/nucleo-g031k8.yaml`](../../configs/systems/nucleo-g031k8.yaml)
and covers boot, the ST-LINK VCP UART, and the user LED.

| Board signal | STM32G031 pin | LabWired path |
| --- | --- | --- |
| ST-LINK VCP TX/RX | PA2 / PA3 | USART2, AF1 |
| LD3 user LED | PC6 | GPIOC, active high |

The committed [smoke evidence](../../examples/nucleo-g031k8/evidence/) asserts
the USART2 marker and repeated PC6 transitions. This profile does not claim
coverage for the other MCU peripherals, ST-LINK USB transport, or unlisted
package pins. The profile declares no zero-answering stubs.

ST's live [board page](https://www.st.com/en/evaluation-tools/nucleo-g031k8.html)
and [eStore page](https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools/stm32-nucleo-boards/nucleo-g031k8.html)
listed the board Active and In Stock on 2026-11-04; details are in the
[vendor-status record](../../examples/nucleo-g031k8/evidence/vendor-status.md).
Board wiring is from [UM2591](https://www.st.com/resource/en/user_manual/dm00622380-stm32g0-nucleo-32-board-mb1455-stmicroelectronics.pdf)
and the [MB1455 G031K8 schematic](https://www.st.com/resource/en/schematic_pack/mb1455-g031k8-d01-schematic.pdf).
