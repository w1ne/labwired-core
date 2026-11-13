# Failed measurement: deferred IRQ record overlap

The exact imported `stm32f072b_discovery` target booted and executed 4,000,000
instructions. The smoke failed because the new NMI/IRQ6 assembly handlers used
overlapping record addresses. The UART showed the intended IRQ6 remained pending
while IRQ5 was active (`pending_irq5=0x00000040`) and then tail-chained after
both NMI entries, but the overlapping output fields made the assertion fail.
The recorded zero JUnit result is not a platform/model failure. The firmware
record layout is corrected in the follow-up measurement.
