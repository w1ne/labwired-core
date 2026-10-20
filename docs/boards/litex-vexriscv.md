# LiteX VexRiscv SoC

The LiteX **VexRiscv** soft-core SoC: a 32-bit RISC-V CPU (`rv32im_zicsr_zifencei` — no compressed, no atomic extension) in a LiteX SoC, as built for FPGA carrier boards such as the Digilent Arty A7. LabWired's target runs a bare-metal ELF from the SoC's integrated SRAM and captures the LiteX UART console.

This is an **L1 (smoke)** target: boot, the UART byte path and the timer one-shot are proven by a committed smoke fixture and an `io-smoke.yaml`. The CPU, memory map, UART and the LiteX countdown timer are modelled; everything else is *declared, not faked* — unmodelled CSR windows are `type: stub` and answer zeros, and the smoke prints one of those readbacks (`spi=00000000`) so the omission is visible on the wire. No silicon capture exists: **SIM-DERIVED**, no executing-fidelity differential yet.

!!! tip "Live status"
    The tables below are a maintained snapshot. Authoritative automation:

    - [Chip conformance scoreboard](../coverage/chip-conformance.md) — level · modelled peripherals · register-match

---

## Status at a glance

| Aspect | Status |
|--------|--------|
| Chip descriptor | [`configs/chips/litex-vexriscv.yaml`](../../configs/chips/litex-vexriscv.yaml) |
| Example system | [`configs/systems/litex-vexriscv.yaml`](../../configs/systems/litex-vexriscv.yaml) |
| Reference firmware | [`crates/firmware-litex-vexriscv-smoke/`](../../crates/firmware-litex-vexriscv-smoke/) |
| Example | [`examples/litex-vexriscv/`](../../examples/litex-vexriscv/) |
| Committed artifact | `tests/fixtures/litex-vexriscv-smoke.elf` (`LITEX VEXRISCV SMOKE`) |
| Build target | `riscv32im-unknown-none-elf` |
| Tier (snapshot) | **L1** — smoke (CPU + memory + UART + timer one-shot); SIM-DERIVED |

---

## Memory map

| Region | Base | Size | State |
|--------|------|------|-------|
| Integrated ROM | `0x0000_0000` | 256 KiB | mapped; contents (LiteX BIOS) **not modelled** |
| Integrated SRAM | `0x1000_0000` | 256 KiB | modelled — the smoke links and runs here |
| SPI-flash window | `0x2000_0000` | 32 MiB | mapped; reads `0xFF` untouched, no flash controller behind it |
| Main RAM (DDR) | `0x4000_0000` | 256 MiB on the SoC | **not mapped** — the engine allocates memory regions densely and a 256 MiB window is not useful to the smoke |
| CSR space | `0x6000_0000` | — | modelled blocks below; the `0xE000_0000` shadow window is documented, not modelled |

## Modelled surface

| Block | Address | State |
|-------|---------|-------|
| LiteX UART | `0x6000_1800` | modelled — `rxtx` @ +0x00, `txfull` @ +0x04 (polled console path) |
| LiteX timer | `0x6000_2800` | modelled — 32-bit countdown (`load`/`reload`/`en`/`update_value`/`value`) + zero-event `ev_status`/`ev_pending`/`ev_enable` |
| LiteX SPI-flash controller | `0x6000_5000` | `type: stub` — answers zeros |
| LiteX Ethernet MAC | `0x6000_7800` | `type: stub` |
| LiteX MMCM | `0xE000_4800` | `type: stub` |

The timer counts down at the SoC clock and advances from the bus's elapsed
cycles; `reload = 0` is a one-shot, `reload > 0` re-arms every `reload + 1`
cycles. IRQ delivery into the RISC-V core is not wired yet, so firmware uses
the status/pending registers (the smoke does exactly that).

The CSR base is a gateware parameter, not a chip constant: LiteX builds in the wild place the CSR region at `0x6000_0000`, `0xE000_0000` or `0xF000_0000`. This descriptor pins the `0x6000_0000`-based map its CSR addresses are written against.

## Boot path

**Fast boot.** The engine loads the ELF and enters it at its entry point with SP seeded at the top of SRAM. The real board boots the LiteX BIOS out of the integrated ROM, which then loads an application image from SPI flash; that BIOS and the flash load are **not modelled**, and this target does not claim them.

## Validation

```bash
cd core
cargo build -p firmware-litex-vexriscv-smoke --release --target riscv32im-unknown-none-elf
./target/release/labwired test --script examples/litex-vexriscv/io-smoke.yaml --no-uart-stdout
```

See [`examples/litex-vexriscv/VALIDATION.md`](../../examples/litex-vexriscv/VALIDATION.md) for the claims, the runtime evidence, and the explicit non-claims.

## Sources

LiteX documentation and source (`litex/soc/cores/cpu/vexriscv/core.py`, `litex/soc/cores/uart.py`), the public `litex-hub/zephyr-on-litex-vexriscv` gateware, and VexRiscv core documentation — detailed in [`examples/litex-vexriscv/REQUIRED_DOCS.md`](../../examples/litex-vexriscv/REQUIRED_DOCS.md).
