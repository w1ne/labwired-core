# STM32F072B-DISCO SysTick-to-PendSV assertion ordering

The target built and executed 4,000,000 instructions. UART and the device
summary showed the new behavior checks passing:
`deferred=1 parent=0x0000000F child=0x0000000E frame_reused=1 r0_live=1
pending_clear=1 cleanup=1`. The overall smoke failed because the expected
`uart_ordered` list placed this line before the pre-existing SysTick checks,
while firmware prints it after `systick icsr set_readback`. The run logs,
UART, JUnit, simulator result, snapshot and generated failure metrics are
preserved here. The assertion is reordered to match the observed firmware
sequence; no behavior expectation was weakened.
