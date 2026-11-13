# Validation — Vega board (RI5CY)

## Measured imported profile

The Renode onboarding chip descriptor was narrowed to the modeled RV32IMC
execution profile, flash, SRAM, and LPUART0. Unmodeled clock, timer, interrupt,
and second-UART windows are omitted. The smoke runs against
`configs/systems/onboarding/vegaboard_ri5cy.yaml`.

| Capability | Evidence |
|---|---|
| RISC-V firmware boot | `cpu_state.type=risc_v`; 4,000,000 instructions executed |
| M extension | Firmware executes an explicit `mul` instruction and prints `mul=0000002a` |
| LPUART0 | Smoke asserts console text and reads back `STAT=0x00c00000` |
| Smoke assertions | 3/3 passed; stop reason `max_steps` |

Committed evidence is under
`docs/coverage/renode-onboarding/metrics/vegaboard_ri5cy/`, including the build
and run logs, JUnit assertions, UART capture, and result JSON. The aggregate
scoreboard is `docs/coverage/renode-onboarding-scoreboard.md`.

This proves only the listed instruction, memory, and UART smoke paths. It does
not test clock gating, additional UARTs, interrupt delivery, low-power timers,
board-level I/O, or silicon behavior. The separate full descriptor
`configs/chips/vegaboard-ri5cy.yaml` still declares additional peripherals;
they are outside this narrowed Renode onboarding profile.
