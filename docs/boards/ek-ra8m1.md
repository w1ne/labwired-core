# EK-RA8M1

The **Renesas EK-RA8M1** evaluation kit with an **R7FA8M1AHECBD** (RA8M1,
Arm Cortex-M85 with Helium and TrustZone, 480 MHz max): 2 MiB of code flash at
`0x0200_0000`, 896 KiB of user SRAM at `0x2200_0000`, three active-high user
LEDs (`P600`, `P414`, `P107`), two active-low switches (`P009`, `P008`) and
the console on **SCI9** `PA14`/`PA15`, the J-Link OB virtual COM port.

This is the **UART+LED L1 slice** and the first RA8 part. Boot, the SCI9
console and the GPIO LED path (with `PCNTR1` PODR read-back) are proven by a
committed smoke fixture and an `io-smoke.yaml`. The RA8 SCI is the 32-bit
**SCI_B** block, not the classic SCI of RA4M1/RA6M5, so this slice adds a
`sci_b` UART layout, an RA8 PFS (`ra8_pfs`: `PWPR`/`PWPRS` moved) and an RA8
SYSTEM window (`ra8_sysc`: the SCICKCR clock-switch handshake). PORT0–B reuse
the RA `ra_port` model.

!!! warning "Not claimed yet"
    The core runs Armv8-M Mainline code only: **Helium/MVE**, the Armv8.1-M
    low-overhead branches and PACBTI are not decoded, so firmware built with
    `-mcpu=cortex-m85` auto-vectorisation will stop with a decode error.
    **TrustZone** is not modelled (no security state, non-secure aliases not
    mapped). Clock generation is not modelled beyond the SCICKCR handshake; no
    rate is derived from it. Module stop (MSTP) and ICU event linking are
    declared stubs, the PFS peripheral mux is stored only, and SCI_B runs
    polled async only (no FIFO, interrupts or wire timing). DMAC/DTC, FCACHE,
    FACI, ELC, RTC, IWDT/WDT, AGT, GPT, IIC, SPI, ADC12, USBFS/USBHS, ETHERC
    and CANFD are declared stubs. SCI0–8 and the remaining blocks are **not
    mapped**: firmware that touches them faults with a memory violation. There
    is no silicon capture: **SIM-DERIVED**.

!!! tip "Live status"
    The tables below are a maintained snapshot. Authoritative automation:

    - [Chip conformance scoreboard](../coverage/chip-conformance.md) — level · modelled peripherals · register-match

---

## Status at a glance

| Aspect | Status |
|--------|--------|
| Chip descriptor | [`configs/chips/r7fa8m1ah.yaml`](../../configs/chips/r7fa8m1ah.yaml) |
| Example system | [`configs/systems/ek-ra8m1.yaml`](../../configs/systems/ek-ra8m1.yaml) |
| Reference firmware | [`crates/firmware-ek-ra8m1-smoke/`](../../crates/firmware-ek-ra8m1-smoke/) |
| Example | [`examples/ek-ra8m1/`](../../examples/ek-ra8m1/) |
| Committed artifact | `tests/fixtures/ek-ra8m1-smoke.elf` (`EK-RA8M1 SMOKE`) |
| Build target | `thumbv8m.main-none-eabi` |
| Tier (snapshot) | **L1** — smoke (CPU + memory + PORT/PFS + SCI9); SIM-DERIVED |

---

## Memory map and board wiring

| Region / signal | Base / pin | State |
|--------|------|-------|
| ITCM | `0x0000_0000`, 64 KiB | mapped RAM (secure alias only) |
| Code flash | `0x0200_0000`, 2 MiB | modelled — the smoke links and runs here |
| Option-setting memory | `0x0300_A100`+ | not mapped — OFS0/OFS1 reset defaults are assumed |
| DTCM | `0x2000_0000`, 64 KiB | mapped RAM (secure alias only) |
| SRAM | `0x2200_0000`, 896 KiB | modelled — stack at the top |
| Data flash | `0x2700_0000`, 12 KiB | mapped, erased (reads `0xFF`); no FACI programming |
| SYSTEM | `0x4001_E000` | `ra8_sysc` — HOCOCR/OSCSF and SCICKDIVCR/SCICKCR (CKSREQ→CKSRDY); the rest reads 0, writes dropped |
| MSTP | `0x4020_3000` | `type: stub` — module stop not modelled |
| ICU | `0x4000_6000` | `type: stub` — IELSR event linking not modelled |
| PORT0–B | `0x4040_0000` + `0x20`×n | modelled (`ra_port`) — PCNTR1/2/3 |
| PFS / PWPR / PWPRS | `0x4040_0800` / `0x4040_0D0C` / `0x4040_0D14` | modelled (`ra8_pfs`) — PCR pull-up and write protection; PSEL/PMR stored only |
| SCI9 (console) | `0x4035_8900` | modelled — SCI_B, async, polled |
| SCI0–8 | `0x4035_8000` + `0x100`×n | not mapped |
| LEDs | LED1 blue P600, LED2 green P414, LED3 red P107 | modelled GPIO — active high |
| Switches | S1 P009, S2 P008 | modelled GPIO input — active low, PFS pull-up |
| SCI9 VCP | PA14 (TXD9) / PA15 (RXD9) | the simulator captures the SCI9 stream; the J-Link OB bridge is not modelled |
| DMAC/DTC, FCACHE, FACI, ELC, RTC, IWDT, WDT, AGT, USBFS, IIC0–2, GPT, ADC12, USBHS, ETHERC/EDMAC, SPI0/1, CANFD | see descriptor | `type: stub` |

## Boot path

**Fast boot.** The engine loads the ELF and enters it through the vector table
at `0x0200_0000` (reset handler `0x0200_0800`, SP `0x220E_0000`). On silicon
the RA8M1 boots secure from the same table; the option-setting memory is a
separate region that this image does not program.

## Validation

```bash
# from the repository root
cargo build -p firmware-ek-ra8m1-smoke --release --target thumbv8m.main-none-eabi
./target/release/labwired test --script examples/ek-ra8m1/io-smoke.yaml --no-uart-stdout
```

See [`examples/ek-ra8m1/VALIDATION.md`](../../examples/ek-ra8m1/VALIDATION.md)
for the claims, runtime evidence and explicit non-claims.

## Sources

The Renesas FSP CMSIS header `R7FA8M1AH.h` (peripheral bases and register
layouts, including `R_SCI_B0_Type`), FSP `ra8m1/bsp_feature.h`,
`bsp_clocks.c`, `bsp_module_stop.h` and `r_sci_b_uart.c` (reset clocks, the
SCICKCR handshake, the SCI9 stop bit, the SCI_B reset values and open order),
the Zephyr `ek_ra8m1` board and RA8 SoC devicetree (memory sizes,
LED/switch/console wiring), and the Renode v1.17.0 `renesas-r7fa8m1a.repl` /
`RenesasRA8M1_SCI.cs` as a cross-check — detailed in
[`examples/ek-ra8m1/REQUIRED_DOCS.md`](../../examples/ek-ra8m1/REQUIRED_DOCS.md).
