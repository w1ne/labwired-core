# CVR clear smoke attempt 1

The imported STM32F072B-DISCO system booted and executed 4,000,000 instructions to `max_steps`. The new CVR-clear behavior itself matched expectations: countdown started, no exception was pending immediately after the CVR write, the next timer period pended SysTick, and exception 15 entered once after PRIMASK was restored.

The run was recorded as a smoke failure because the emitted UART line had two spaces after `started=1`, while the ordered assertion expected one. This is an assertion formatting mismatch, not a simulator behavior failure. Raw runner output, metrics, JUnit, result, snapshot, and UART log are preserved beside this note.
