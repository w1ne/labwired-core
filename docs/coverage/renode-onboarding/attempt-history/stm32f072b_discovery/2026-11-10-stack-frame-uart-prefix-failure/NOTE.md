The Cortex-M0 stack-frame smoke built and ran through 4,000,000 instructions.
The frame assertions all emitted `frame=1`, including the four sentinels,
PC/xPSR, EXC_RETURN, and aligned stack address. The sole failure was the YAML
UART substring omitting the `0x` prefixes printed before each register value.
The complete result, UART, JUnit, and logs are preserved here.
