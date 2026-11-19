# Vega RI5CY — Renode onboarding evidence

The original imported chip descriptor mislabeled the target as ARM and listed
clock/timer blocks that the engine does not model. The onboarding profile now
declares RISC-V RV32IMC, flash, SRAM, and the modeled LPUART0 only.

## Smoke result

The imported system ran the RISC-V smoke for 4,000,000 instructions. The result
JSON reports `cpu_state.type=risc_v`; all 3/3 smoke assertions passed, including
an explicit `mul` instruction returning `0x2a` and the LPUART status readback.
This is a narrow smoke result, not a claim about the full Vega platform.

## Modeled surface

| Peripheral ID | Base address | Type |
|---|---:|---|
| `lpuart0` | `0x40042000` | `nxp_lpuart` |

Flash (`0x0`, 1 MiB), SRAM (`0x20000000`, 192 KiB), and the RV32IMC
execution subset are also configured. Clock generators, additional UARTs,
interrupt muxes, low-power timers, and board I/O are omitted from this
onboarding profile.

Per-target logs and the full result are in
`docs/coverage/renode-onboarding/metrics/vegaboard_ri5cy/`; aggregate counts are
in `docs/coverage/renode-onboarding-scoreboard.md`.
