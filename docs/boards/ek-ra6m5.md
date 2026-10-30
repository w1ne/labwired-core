# EK-RA6M5

The **Renesas EK-RA6M5** evaluation kit with an **R7FA6M5BH3CFC** (RA6M5,
Cortex-M33 with TrustZone, 200 MHz max): 2 MiB of code flash at `0x0000_0000`,
512 KiB of SRAM at `0x2000_0000`, three active-high user LEDs (`P006`–`P008`),
two active-low switches (`P005`, `P004`) and the console on **SCI0**
`P411`/`P410`, the J-Link OB virtual COM port.

This is the **UART+LED L1 slice**. Boot, the SCI0 console and the GPIO LED path
(with `PCNTR1` PODR read-back) are proven by a committed smoke fixture and an
`io-smoke.yaml`. It reuses the RA4M1 PORT, PFS, SYSTEM and SCI models at the
RA6M5 bases; there is no new peripheral model in this slice.

!!! warning "Not claimed yet"
    Clock generation is not modelled: only `HOCOCR`/`OSCSF` exist in the SYSTEM
    window, and the smoke stays on the reset MOCO. Module stop (MSTP) and ICU
    event linking are declared stubs, and the PFS peripheral mux is stored
    only. TrustZone, flash programming, DMAC/DTC, ELC, RTC, IWDT/WDT, AGT, GPT,
    IIC, SPI, ADC12, USBFS/USBHS, ETHERC and CANFD are declared stubs. SCI1–9
    and the remaining blocks are **not mapped**: firmware that touches them
    faults with a memory violation. There is no silicon capture:
    **SIM-DERIVED**.

!!! tip "Live status"
    The tables below are a maintained snapshot. Authoritative automation:

    - [Chip conformance scoreboard](../coverage/chip-conformance.md) — level · modelled peripherals · register-match

---

## Status at a glance

| Aspect | Status |
|--------|--------|
| Chip descriptor | [`configs/chips/r7fa6m5bh.yaml`](../../configs/chips/r7fa6m5bh.yaml) |
| Example system | [`configs/systems/ek-ra6m5.yaml`](../../configs/systems/ek-ra6m5.yaml) |
| Reference firmware | [`crates/firmware-ek-ra6m5-smoke/`](../../crates/firmware-ek-ra6m5-smoke/) |
| Example | [`examples/ek-ra6m5/`](../../examples/ek-ra6m5/) |
| Committed artifact | `tests/fixtures/ek-ra6m5-smoke.elf` (`EK-RA6M5 SMOKE`) |
| Build target | `thumbv8m.main-none-eabi` |
| Tier (snapshot) | **L1** — smoke (CPU + memory + PORT/PFS + SCI0); SIM-DERIVED |

---

## Memory map and board wiring

| Region / signal | Base / pin | State |
|--------|------|-------|
| Code flash | `0x0000_0000`, 2 MiB | modelled — the smoke links and runs here |
| Data flash | `0x0800_0000`, 8 KiB | not mapped |
| Option-setting memory | `0x0100_A100`+ | not mapped — OFS0/OFS1 reset defaults are assumed |
| SRAM | `0x2000_0000`, 512 KiB | modelled — stack at the top |
| SYSTEM | `0x4001_E000` | `ra_sysc` — HOCOCR/OSCSF only; SCKDIVCR/SCKSCR/PRCR read 0, writes dropped |
| MSTP | `0x4008_4000` | `type: stub` — module stop not modelled |
| ICU | `0x4000_6000` | `type: stub` — IELSR event linking not modelled |
| PORT0–B | `0x4008_0000` + `0x20`×n | modelled (`ra_port`) — PCNTR1/2/3 |
| PFS / PWPR | `0x4008_0800` / `0x4008_0D03` | modelled (`ra_pfs`) — PCR pull-up and write protection; PSEL/PMR stored only |
| SCI0 (console) | `0x4011_8000` | modelled — classic SCI, async, polled |
| SCI1–9 | `0x4011_8100` + `0x100`×n | not mapped |
| LEDs | LED1 blue P006, LED2 green P007, LED3 red P008 | modelled GPIO — active high |
| Switches | S1 P005, S2 P004 | modelled GPIO input — active low, PFS pull-up |
| SCI0 VCP | P411 (TXD0) / P410 (RXD0) | the simulator captures the SCI0 stream; the J-Link OB bridge is not modelled |
| DMAC/DTC, ELC, RTC, IWDT, WDT, USBFS, IIC0–2, CANFD, AGT0–5, USBHS, ETHERC/EDMAC, SPI0/1, GPT, ADC12, FACI | see descriptor | `type: stub` |

## Boot path

**Fast boot.** The engine loads the ELF and enters it through the vector table
at `0x0000_0000` (reset handler `0x800`, SP `0x2008_0000`). On silicon the
RA6M5 boots secure from the same table; the option-setting memory is a separate
region that this image does not program.

## Validation

```bash
# from the repository root
cargo build -p firmware-ek-ra6m5-smoke --release --target thumbv8m.main-none-eabi
./target/release/labwired test --script examples/ek-ra6m5/io-smoke.yaml --no-uart-stdout
```

See [`examples/ek-ra6m5/VALIDATION.md`](../../examples/ek-ra6m5/VALIDATION.md)
for the claims, runtime evidence and explicit non-claims.

## Sources

The Renesas FSP CMSIS header `R7FA6M5BH.h` (peripheral bases and register
offsets), the Zephyr `ek_ra6m5` board and RA6M5 SoC devicetree (memory sizes,
LED/switch/console wiring, SCI0 clock gate `MSTPB31`), and the Renode v1.17.0
`renesas-r7fa6m5b.repl` as a cross-check of the bus map — detailed in
[`examples/ek-ra6m5/REQUIRED_DOCS.md`](../../examples/ek-ra6m5/REQUIRED_DOCS.md).
