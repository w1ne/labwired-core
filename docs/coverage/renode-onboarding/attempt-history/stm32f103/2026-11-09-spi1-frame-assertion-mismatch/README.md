# STM32F103 SPI1 assertion update needed

The second `--only stm32f103` run booted 50,000,000 instructions. Firmware
reported the documented MAX31855 power-on frame `0x01901600` and passed both
SPI1 TXE and RXNE IRQ checks. The smoke still expected `0x03201600` from the
removed 50 °C stimulus, so its UART assertion failed. The next run updates the
assertion to the observed reset frame. This directory retains the complete
metrics, UART output, result, snapshot, JUnit report, and build/run logs.
