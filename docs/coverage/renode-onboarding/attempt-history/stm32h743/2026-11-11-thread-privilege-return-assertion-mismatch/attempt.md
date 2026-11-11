# STM32H743 first Thread privilege return assertion run

- The imported system booted and executed 4,000,000 instructions to `max_steps`.
- Firmware UART showed the new result:
  `cortex-m7 cpacr handler fp=1 ipsr=0000000B thread_unprivileged=00000001 nocp=00000000 result=000000C3`.
- The smoke script still expected the previous UART line, so its assertion
  failed on a stale expected string. The complete metrics, logs, JUnit result,
  UART output, simulator result and snapshot are preserved here.
