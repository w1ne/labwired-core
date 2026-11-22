# STM32F429 smoke attempt history

Attempt 1 used the F7 shared smoke binary, which polls the later USART ISR/TDR register layout. Against the imported F429 F4 USART model it executed 800,000 instructions but captured one UART byte, stalled polling TXE, and did not reach the GPIO assertion. The raw metrics, run log, result, JUnit, and UART log are preserved under `attempt-1/`. The profile was changed to use a dedicated F4 classic SR/DR smoke; the generated scoreboard reports the latest run only.

The 2026-11-11 PSP SysTick-to-PendSV probe booted but faulted on its first
read of `SYST_RVR` (`0xE000_E014`): the conservative STM32F429 profile had
omitted the core SysTick window. The full failed target-only run is preserved
under `2026-11-11-before-systick-window/`. The profile will include the existing
real SysTick model before the probe is rerun.

The next target-only run mapped the model, booted without a fault, and reached
the assertion, but the probe reported no SysTick/PendSV entries. Its setup had
left `PRIMASK` set after installing the temporary vector table. That run is
preserved under `2026-11-11-before-unmasking-pendsv/`; the probe now restores
the saved interrupt mask before requesting the exception.

The final target-only run passed all five smoke assertions after adding the
engine's real SysTick model to the bounded STM32F429 profile. It executed
800,000 instructions without a fault. UART reported
`systick pendsv psp tailchain frame=1 r0=1 pending=1 ipsr=0000000E ok`; the
current metrics and raw artifacts are under
`docs/coverage/renode-onboarding/metrics/stm32f429/`.
