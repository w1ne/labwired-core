# Digilent Arty A7-35T — LiteX VexRiscv SoC

The **Digilent Arty A7-35T** (Artix-7 XC7A35T) running a **LiteX VexRiscv** soft-core SoC: a 32-bit RISC-V CPU (`rv32im_zicsr_zifencei`), 100 MHz system clock, integrated SRAM, and a Control-and-Status CSR for the board's user I/O. LabWired runs a bare-metal ELF from the SoC's SRAM and captures the LiteX UART console.

This is an **L1 (smoke)** target on the `litex-vexriscv` SoC family: boot, the UART byte path, the timer one-shot, and the LED/switch/button CSR path are proven by a committed smoke fixture and an `io-smoke.yaml`. The CPU, memory map, UART, timer and CAS block are modelled; the SPI-flash controller, Ethernet MAC and MMCM are declared stubs. No bitstream capture exists: **SIM-DERIVED**, no executing-fidelity differential yet.

!!! tip "Live status"
    The tables below are a maintained snapshot. Authoritative automation:

    - [Chip conformance scoreboard](../coverage/chip-conformance.md) — level · modelled peripherals · register-match

---

## Status at a glance

| Aspect | Status |
|--------|--------|
| Chip descriptor | [`configs/chips/arty-litex-vexriscv.yaml`](../../configs/chips/arty-litex-vexriscv.yaml) |
| Example system | [`configs/systems/arty-litex-vexriscv.yaml`](../../configs/systems/arty-litex-vexriscv.yaml) |
| Reference firmware | [`crates/firmware-arty-litex-smoke/`](../../crates/firmware-arty-litex-smoke/) |
| Example | [`examples/arty-litex-vexriscv/`](../../examples/arty-litex-vexriscv/) |
| Committed artifact | `tests/fixtures/arty-litex-smoke.elf` (`ARTY LITEX SMOKE`) |
| Build target | `riscv32im-unknown-none-elf` |
| Tier (snapshot) | **L1** — smoke (CPU + memory + UART + timer + CAS); SIM-DERIVED |

---

## Memory map

| Region | Base | Size | State |
|--------|------|------|-------|
| Integrated ROM | `0x0000_0000` | 256 KiB | mapped; contents (LiteX BIOS) **not modelled** |
| Integrated SRAM | `0x1000_0000` | 256 KiB | modelled — the smoke links and runs here |
| SPI-flash window | `0x2000_0000` | 32 MiB | mapped; reads `0xFF` untouched, no flash controller behind it |
| Main RAM (DDR) | `0x4000_0000` | 256 MiB on the SoC | **not mapped** |
| CSR space | `0x6000_0000` | — | modelled blocks below; the `0xE000_0000` shadow is documented, not modelled |

## Modelled surface

| Block | Address | State |
|-------|---------|-------|
| LiteX UART | `0x6000_1800` | modelled — `rxtx` @ +0x00, `txfull` @ +0x04 |
| LiteX timer | `0x6000_2800` | modelled — countdown + zero-event status/pending |
| Control-and-Status (CAS) | `0x6000_6800` | modelled — `Leds` @0x00 (R/W), `Switches` @0x04, `ButtonsStatus` @0x08, `ButtonsPending` @0x0C (W1C), `ButtonsEnabled` @0x10 |
| LiteX SPI-flash controller | `0x6000_5000` | `type: stub` — answers zeros |
| LiteX Ethernet MAC | `0x6000_7800` | `type: stub` |
| LiteX MMCM | `0xE000_4800` | `type: stub` |

## Board I/O

| Board control | CAS bits | `board_io` |
| :--- | :--- | :--- |
| LD4–LD7 (user LEDs) | `Leds` 0–3 | output, active high |
| SW0–SW3 (slide switches) | `Switches` 0–3 | input, active low |
| BTN0–BTN3 (push-buttons) | `ButtonsStatus` 0–3; presses latch in `ButtonsPending` | input, active low (`buttons_active_low: true`) |

## Boot path

**Fast boot.** The engine loads the ELF and enters it at its entry point with SP seeded at the top of SRAM. The real board boots the LiteX BIOS out of the integrated ROM and loads an application from SPI flash; that BIOS and the flash load are **not modelled**.

## Validation

```bash
cd core
cargo build -p firmware-arty-litex-smoke --release --target riscv32im-unknown-none-elf
./target/release/labwired test --script examples/arty-litex-vexriscv/io-smoke.yaml --no-uart-stdout
```

See [`examples/arty-litex-vexriscv/VALIDATION.md`](../../examples/arty-litex-vexriscv/VALIDATION.md) for the claims, runtime evidence and explicit non-claims.

## Sources

Digilent Arty A7 reference manual, LiteX documentation and source (`litex/soc/cores/uart.py`, `timer.py`), the public VexRiscv SoC gateware for the Arty and the generated `csr.h` for the Control-and-Status block — detailed in [`examples/arty-litex-vexriscv/REQUIRED_DOCS.md`](../../examples/arty-litex-vexriscv/REQUIRED_DOCS.md).
