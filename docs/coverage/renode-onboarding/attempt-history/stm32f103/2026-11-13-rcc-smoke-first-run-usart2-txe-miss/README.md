# STM32F103 RCC smoke first run

The target booted and reached `MaxSteps`. The new RCC assertions all passed:
reset APB enable values, enabled peripheral bits, APB2 reserved-bit masking,
GPIOB reads while IOPBEN was clear, a dropped GPIOB write while unclocked, and
restored GPIOB access after re-enabling the clock.

The overall smoke failed one existing assertion: USART2 TXEIE did not appear in
NVIC pending during the firmware's 64-cycle observation window (`TXEIRQ=0`,
`REARM=1`). The remaining 81 assertions passed. The full per-target metrics,
logs, UART output, JUnit, result, and snapshot from this attempt are preserved
in this directory. The follow-up increases that bounded observation window to
128 cycles before rerunning only `stm32f103`.
