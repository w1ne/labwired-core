# STM32F429 smoke attempt history

Attempt 1 used the F7 shared smoke binary, which polls the later USART ISR/TDR register layout. Against the imported F429 F4 USART model it executed 800,000 instructions but captured one UART byte, stalled polling TXE, and did not reach the GPIO assertion. The raw metrics, run log, result, JUnit, and UART log are preserved under `attempt-1/`. The profile was changed to use a dedicated F4 classic SR/DR smoke; the generated scoreboard reports the latest run only.
