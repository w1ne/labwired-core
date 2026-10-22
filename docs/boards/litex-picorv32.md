# LiteX PicoRV32 SoC

The LiteX **PicoRV32** platform: a 32-bit RISC-V **RV32IMC** soft core (the M and C extensions are present) in a LiteX SoC with the CSR space at `0x60000000` (and the `0xE0000000` shadow), integrated memory at `0x0`, SRAM at `0x10000000`, and a 32 MiB SPI-flash window at `0x20000000`.

This is an **L1 (smoke)** target: boot + the UART byte path are proven by a committed smoke fixture and an `io-smoke.yaml`. The CPU, memory map and LiteX UART are modelled; the timer (this platform uses the **8-bit-CSR** `LiteX_Timer` variant at `0xE0002000`), the SPI-flash controller and the Ethernet MAC are declared stubs, and the smoke prints the timer readback so the boundary is visible on the wire. No silicon capture exists: **SIM-DERIVED**, no executing-fidelity differential yet.

!!! tip "Live status"
    The tables below are a maintained snapshot. Authoritative automation:

    - [Chip conformance scoreboard](../coverage/chip-conformance.md) — level · modelled peripherals · register-match

---

## Status at a glance

| Aspect | Status |
|--------|--------|
| Chip descriptor | [`configs/chips/litex-picorv32.yaml`](../../configs/chips/litex-picorv32.yaml) |
| Example system | [`configs/systems/litex-picorv32.yaml`](../../configs/systems/litex-picorv32.yaml) |
| Reference firmware | [`crates/firmware-litex-picorv32-smoke/`](../../crates/firmware-litex-picorv32-smoke/) |
| Example | [`examples/litex-picorv32/`](../../examples/litex-picorv32/) |
| Committed artifact | `tests/fixtures/litex-picorv32-smoke.elf` (`PICORV32 LITEX SMOKE`) |
| Build target | `riscv32imc-unknown-none-elf` |
| Tier (snapshot) | **L1** — smoke (CPU + memory + UART); SIM-DERIVED |

---

## Memory map

| Region | Base | Size | State |
|--------|------|------|-------|
| Integrated memory | `0x0000_0000` | 256 KiB | mapped as a `memory_regions` window |
| Integrated SRAM | `0x1000_0000` | 256 KiB | modelled — the smoke links and runs here |
| SPI-flash window | `0x2000_0000` | 32 MiB | declared as `flash` (reads `0xFF` untouched) |
| Main RAM (DDR) | `0x4000_0000` | 256 MiB on the SoC | **not mapped** |
| CSR space | `0x6000_0000` | — | modelled block below; the `0xE000_0000` shadow is documented, not modelled |

## Modelled surface

| Block | Address | State |
|-------|---------|-------|
| LiteX UART | `0x6000_1800` | modelled — `rxtx` @ +0x00, `txfull` @ +0x04 |
| LiteX timer (8-bit CSR variant) | `0xE000_2000` | `type: stub` — answers zeros |
| LiteX SPI-flash controller | `0x6000_5000` | `type: stub` |
| LiteX Ethernet MAC | `0x6000_7800` | `type: stub` |

## Boot path

**Fast boot.** The engine loads the ELF and enters it at its entry point with SP seeded at the top of SRAM. The real board boots the LiteX BIOS out of the integrated memory window and loads an application from SPI flash; that BIOS and the flash load are **not modelled**.

## Validation

```bash
cd core
cargo build -p firmware-litex-picorv32-smoke --release --target riscv32imc-unknown-none-elf
./target/release/labwired test --script examples/litex-picorv32/io-smoke.yaml --no-uart-stdout
```

See [`examples/litex-picorv32/VALIDATION.md`](../../examples/litex-picorv32/VALIDATION.md) for the claims, runtime evidence and explicit non-claims.

## Sources

LiteX documentation and source (`litex/soc/cores/uart.py`), the pinned upstream LiteX PicoRV32 platform for the SoC map, and the PicoRV32 core documentation — detailed in [`examples/litex-picorv32/REQUIRED_DOCS.md`](../../examples/litex-picorv32/REQUIRED_DOCS.md).
