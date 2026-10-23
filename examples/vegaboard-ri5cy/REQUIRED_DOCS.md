# Required Documents — Vega board (RI5CY)

Authoritative sources used by this example (retrieved 2026-10-23):

| Doc | Title | Used for |
| :--- | :--- | :--- |
| Pinned upstream RI5CY platform | the platform definition | RI5CY CPU (RV32IMC, privileged 1.10), 1 MiB flash at 0x0, 192 KiB SRAM at 0x20000000, LPUART0 at 0x40042000 (external IRQ 17), LPUART1 at 0x40043000, LPTMR0/1/2, INTMUX at 0x4004F000 |
| Pinned Vega board overlay | the board definition | SCG CSR at 0x4002C010, PCC0 at 0x4002B000, PCC1 at 0x41027000, and the clock-status tags |
| NXP LPUART (Kinetis) register map | the UART | DATA @ +0x0C (write transmits), STAT @ +0x04 with TDRE (bit 23) / TC (bit 22); the engine's `lpuart` UART profile models it |
| RI5CY (PULP) core documentation | the CPU | RV32IMC — multiply and compressed instructions; the smoke is built for `riscv32imc-unknown-none-elf` and issues a real multiply |

Open items: the SCG/PCC clock blocks, INTMUX and the low-power timers are
declared stubs; the platform declares no core clock (the descriptor models
50 MHz); no board-level user I/O is defined by this platform.
