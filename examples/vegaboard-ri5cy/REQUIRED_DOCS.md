# Required Documents — Vega board (RI5CY)

Authoritative sources used by this example (retrieved 2026-10-23):

| Doc | Title | Used for |
| :--- | :--- | :--- |
| Pinned upstream RI5CY platform | the platform definition | The upstream device map; this LabWired profile uses only its RV32IMC execution subset, 1 MiB flash at 0x0, 192 KiB SRAM at 0x20000000, and LPUART0 at 0x40042000 |
| NXP LPUART (Kinetis) register map | the UART | DATA @ +0x0C (write transmits), STAT @ +0x04 with TDRE (bit 23) / TC (bit 22); the engine's `nxp_lpuart` model handles the polled console path |
| RI5CY (PULP) core documentation | the CPU | RV32IMC — the smoke is built for `riscv32imc-unknown-none-elf` and executes an explicit `mul` instruction |

The imported chip descriptor deliberately omits the SCG/PCC blocks, second
LPUART, INTMUX, and low-power timers because this profile does not model them.
The smoke makes no claim about those devices or board-level I/O.
