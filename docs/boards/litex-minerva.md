# LiteX Minerva SoC

The LiteX **Minerva** platform: a 32-bit RISC-V **RV32I** soft core (no multiply/divide, no compressed instructions) in a LiteX SoC, with the CSR space at `0xE0000000`, integrated memory at `0x0`/`0x10000000`, a ROM at `0x20020000` and a 1 MiB boot-flash window at `0x20228000`.

This is an **L1 (smoke)** target: boot + the UART byte path are proven by a committed smoke fixture and an `io-smoke.yaml`. The CPU, memory map and LiteX UART are modelled; the timer (this platform uses the **8-bit-CSR** `LiteX_Timer` variant, not the 32-bit CSR timer modelled for the VexRiscv SoC) and the Ethernet MAC are declared stubs, and the smoke prints the timer readback so the boundary is visible on the wire. No silicon capture exists: **SIM-DERIVED**, no executing-fidelity differential yet.

!!! tip "Live status"
    The tables below are a maintained snapshot. Authoritative automation:

    - [Chip conformance scoreboard](../coverage/chip-conformance.md) — level · modelled peripherals · register-match

---

## Status at a glance

| Aspect | Status |
|--------|--------|
| Chip descriptor | [`configs/chips/litex-minerva.yaml`](../../configs/chips/litex-minerva.yaml) |
| Example system | [`configs/systems/litex-minerva.yaml`](../../configs/systems/litex-minerva.yaml) |
| Reference firmware | [`crates/firmware-litex-minerva-smoke/`](../../crates/firmware-litex-minerva-smoke/) |
| Example | [`examples/litex-minerva/`](../../examples/litex-minerva/) |
| Committed artifact | `tests/fixtures/litex-minerva-smoke.elf` (`MINERVA LITEX SMOKE`) |
| Build target | `riscv32i-unknown-none-elf` |
| Tier (snapshot) | **L1** — smoke (CPU + memory + UART); SIM-DERIVED |

---

## Memory map

| Region | Base | Size | State |
|--------|------|------|-------|
| Integrated memory | `0x0000_0000` | 256 KiB | mapped as a `memory_regions` window |
| Integrated SRAM | `0x1000_0000` | 256 KiB | modelled — the smoke links and runs here |
| ROM | `0x2002_0000` | 32 KiB | mapped; contents (LiteX BIOS) **not modelled** |
| Boot flash | `0x2022_8000` | 1 MiB | declared as `flash` (reads `0xFF` untouched) |
| Main RAM (DDR) | `0x4000_0000` | 256 MiB on the SoC | **not mapped** |
| CSR space | `0xE000_0000` | — | modelled block below |

## Modelled surface

| Block | Address | State |
|-------|---------|-------|
| LiteX UART | `0xE000_1800` | modelled — `rxtx` @ +0x00, `txfull` @ +0x04 |
| LiteX timer (8-bit CSR variant) | `0xE000_2800` | `type: stub` — answers zeros |
| LiteX Ethernet MAC | `0xE000_3800` | `type: stub` |

## Boot path

**Fast boot.** The engine loads the ELF and enters it at its entry point with SP seeded at the top of SRAM. The real board boots the LiteX BIOS from the ROM and loads an application from boot flash; that BIOS and the flash load are **not modelled**.

## Validation

```bash
cd core
cargo build -p firmware-litex-minerva-smoke --release --target riscv32i-unknown-none-elf
./target/release/labwired test --script examples/litex-minerva/io-smoke.yaml --no-uart-stdout
```

See [`examples/litex-minerva/VALIDATION.md`](../../examples/litex-minerva/VALIDATION.md) for the claims, runtime evidence and explicit non-claims.

## Sources

LiteX documentation and source (`litex/soc/cores/uart.py`), the pinned upstream LiteX Minerva platform for the SoC map, and the Minerva core documentation — detailed in [`examples/litex-minerva/REQUIRED_DOCS.md`](../../examples/litex-minerva/REQUIRED_DOCS.md).
