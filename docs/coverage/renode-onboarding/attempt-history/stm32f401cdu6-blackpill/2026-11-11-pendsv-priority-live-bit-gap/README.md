# F401 PendSV pending-bit diagnostic

The instrumented probe reported flags `0x1ef`: PendSV entered once at vector
14 after SysTick returned, priorities and vector setup were correct, but
ICSR.PENDSVSET did not remain visible after the SCB handed the request to the
CPU pending bitmap. Cortex-M already kept this state live for SysTick. The core
and SCB were updated to share PendSV pending state; the following target-only
run passed with flags `0x1ff`.

This failed local CLI run is retained as diagnostic history, not the current
scoreboard measurement.
