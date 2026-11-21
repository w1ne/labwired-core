# STM32F4-Discovery SysTick profile missing

The first target-only attempt for the Cortex-M4 SysTick-to-PendSV tail-chain
smoke booted and executed 4,000,000 instructions, but faulted on a precise
BusFault when firmware read `SYST_RVR` at `0xE000E014`. The imported
`stm32f4_discovery` chip profile did not map the architected SysTick block.
The smoke consequently emitted no tail-chain UART record and failed its UART
assertion. The runner also could not regenerate the scoreboard because the
invoked Python did not have PyYAML installed; rerunning with `/usr/bin/python3`
uses the system installation used by this lane.
