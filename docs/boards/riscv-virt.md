# RISC-V `virt` machine

The generic RISC-V **`virt`** machine — the platform convention shared by QEMU's `-M virt` and the upstream RISC-V platform definition: a 32-bit **RV32IMAC** CPU, one flat RAM window at `0x80000000`, an **NS16550** UART0 at `0x10000000`, a CLINT at `0x02000000` (10 MHz timebase) and a PLIC at `0x0C000000`.

This is an **L1 (smoke)** target: boot + the UART byte path are proven by a committed smoke fixture and an `io-smoke.yaml`. The CPU, memory map and NS16550 UART are modelled; the PLIC and CLINT are declared stubs (the engine has no platform-level interrupt-controller model yet), and the smoke prints both readbacks so the boundary is visible on the wire. No silicon capture exists: **SIM-DERIVED**, no executing-fidelity differential yet.

!!! tip "Live status"
    The tables below are a maintained snapshot. Authoritative automation:

    - [Chip conformance scoreboard](../coverage/chip-conformance.md) — level · modelled peripherals · register-match

---

## Status at a glance

| Aspect | Status |
|--------|--------|
| Chip descriptor | [`configs/chips/riscv-virt.yaml`](../../configs/chips/riscv-virt.yaml) |
| Example system | [`configs/systems/riscv-virt.yaml`](../../configs/systems/riscv-virt.yaml) |
| Reference firmware | [`crates/firmware-riscv-virt-smoke/`](../../crates/firmware-riscv-virt-smoke/) |
| Example | [`examples/riscv-virt/`](../../examples/riscv-virt/) |
| Committed artifact | `tests/fixtures/riscv-virt-smoke.elf` (`RISCV VIRT SMOKE`) |
| Build target | `riscv32imc-unknown-none-elf` |
| Tier (snapshot) | **L1** — smoke (CPU + memory + UART); SIM-DERIVED |

---

## Memory map

| Region | Base | Size | State |
|--------|------|------|-------|
| RAM (code window) | `0x8000_0000` | 1 MiB | modelled as `flash` — the loader places text here |
| RAM (data/stack window) | `0x8010_0000` | 7 MiB | modelled as `ram` |
| UART0 (NS16550) | `0x1000_0000` | 4 KiB | modelled — THR @ +0x00, LSR @ +0x05 |
| CLINT | `0x0200_0000` | 64 KiB | `type: stub` — answers zeros |
| PLIC | `0x0C00_0000` | 4 MiB | `type: stub` |

The machine's RAM size is a parameter (QEMU `-m`, upstream 2 GiB); this descriptor models an 8 MiB window, split for the loader. The CLINT timebase is available to firmware through the CPU's `time` CSR even though the MMIO window is a stub.

## Boot path

**Fast boot.** The engine loads the ELF and enters it at its entry point with SP seeded at the top of `ram` (0x80100000 + 7 MiB). The machine has no firmware/bootloader stage.

## Validation

```bash
cd core
cargo build -p firmware-riscv-virt-smoke --release --target riscv32imc-unknown-none-elf
./target/release/labwired test --script examples/riscv-virt/io-smoke.yaml --no-uart-stdout
```

See [`examples/riscv-virt/VALIDATION.md`](../../examples/riscv-virt/VALIDATION.md) for the claims, runtime evidence and explicit non-claims.

## Sources

The pinned upstream RISC-V `virt` platform for the memory map and interrupt wiring, the QEMU `virt` machine documentation for the same convention, and the PC16550D datasheet for the UART register map — detailed in [`examples/riscv-virt/REQUIRED_DOCS.md`](../../examples/riscv-virt/REQUIRED_DOCS.md).
