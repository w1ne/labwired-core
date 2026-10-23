# Vega board (RI5CY)

The PULP **Vega** board's SoC: a 32-bit RISC-V **RI5CY** core (RV32IMC), 1 MiB of flash at `0x0`, 192 KiB of SRAM at `0x20000000`, an **NXP LPUART0** console at `0x40042000` (external IRQ 17), a second LPUART at `0x40043000`, low-power timers at `0x40032000`/`0x40033000`/`0x4102B000`, and the SCG/PCC clock blocks.

This is an **L1 (smoke)** target: boot + the UART byte path are proven by a committed smoke fixture and an `io-smoke.yaml`. The CPU, memory map and LPUART0/1 are modelled; the SCG/PCC clock blocks, INTMUX and low-power timers are declared stubs, and the smoke prints the SCG/PCC readbacks so the boundary is visible on the wire. No silicon capture exists: **SIM-DERIVED**, no executing-fidelity differential yet.

!!! tip "Live status"
    The tables below are a maintained snapshot. Authoritative automation:

    - [Chip conformance scoreboard](../coverage/chip-conformance.md) — level · modelled peripherals · register-match

---

## Status at a glance

| Aspect | Status |
|--------|--------|
| Chip descriptor | [`configs/chips/vegaboard-ri5cy.yaml`](../../configs/chips/vegaboard-ri5cy.yaml) |
| Example system | [`configs/systems/vegaboard-ri5cy.yaml`](../../configs/systems/vegaboard-ri5cy.yaml) |
| Reference firmware | [`crates/firmware-vegaboard-ri5cy-smoke/`](../../crates/firmware-vegaboard-ri5cy-smoke/) |
| Example | [`examples/vegaboard-ri5cy/`](../../examples/vegaboard-ri5cy/) |
| Committed artifact | `tests/fixtures/vegaboard-ri5cy-smoke.elf` (`VEGABOARD RI5CY SMOKE`) |
| Build target | `riscv32imc-unknown-none-elf` |
| Tier (snapshot) | **L1** — smoke (CPU + memory + UART); SIM-DERIVED |

---

## Memory map

| Region | Base | Size | State |
|--------|------|------|-------|
| Flash | `0x0000_0000` | 1 MiB | modelled — the smoke links and runs here |
| SRAM | `0x2000_0000` | 192 KiB | modelled — data/stack |
| LPUART0 | `0x4004_2000` | 4 KiB | modelled — DATA @ +0x0C, STAT @ +0x04 |
| LPUART1 | `0x4004_3000` | 4 KiB | modelled |
| LPTMR0/1/2 | `0x4003_2000` / `0x4003_3000` / `0x4102_B000` | 4 KiB | `type: stub` — zeros |
| INTMUX | `0x4004_F000` | 4 KiB | `type: stub` |
| SCG CSR | `0x4002_C010` | 4 B | `type: stub` |
| PCC0 / PCC1 | `0x4002_B000` / `0x4102_7000` | 0x204 / 0x208 | `type: stub` |

## Boot path

**Fast boot.** The engine loads the ELF and enters it at its entry point (the
flash base, `0x0`) with SP seeded at the top of SRAM. The board's real boot
flow is not modelled.

## Validation

```bash
cd core
cargo build -p firmware-vegaboard-ri5cy-smoke --release --target riscv32imc-unknown-none-elf
./target/release/labwired test --script examples/vegaboard-ri5cy/io-smoke.yaml --no-uart-stdout
```

See [`examples/vegaboard-ri5cy/VALIDATION.md`](../../examples/vegaboard-ri5cy/VALIDATION.md) for the claims, runtime evidence and explicit non-claims.

## Sources

The pinned upstream RI5CY platform and Vega board overlay for the SoC map and
clock blocks, and the NXP LPUART (Kinetis) register map for the UART — detailed
in [`examples/vegaboard-ri5cy/REQUIRED_DOCS.md`](../../examples/vegaboard-ri5cy/REQUIRED_DOCS.md).
