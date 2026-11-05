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
SYSTEM window (`ra8_sysc`). PORT0–B reuse the RA `ra_port` model.

**Stock Zephyr v3.7.0** `samples/hello_world` and `samples/synchronization`
for board `ek_ra8m1` also boot, unmodified, from committed fixtures rebuilt
byte-identical by
[`crates/firmware-ek-ra8m1-zephyr/build.sh`](../../crates/firmware-ek-ra8m1-zephyr/build.sh).
That needed three things:

- **Armv8.1-M scalar instructions.** Zephyr builds for
  `-mcpu=cortex-m85+nomve`, and GCC 13 emits a `DLS`/`LE` low-overhead loop
  (`z_arm_interrupt_init`) and `CSINC` (`cbprintf`). The decoder now handles
  `DLS`/`WLS`/`LE` and `CSEL`/`CSINC`/`CSINV`/`CSNEG`. Before this, `DLS`/`LE`
  fell into a legacy fallback that skipped them, so such a loop silently ran
  its body once.
- **The FSP clock bring-up.** `ra8_sysc` is now a full `R_SYSTEM` register
  file: `OSCSF` follows the HOCO/MOSC/PLL/PLL2 stop bits, and every
  peripheral-clock switch register (`SCICKCR`, `SPICKCR`, … `I3CCKCR`)
  answers the CKSREQ→CKSRDY handshake. `bsp_clock_init` starts the main
  oscillator and both PLLs and moves ICLK to PLL1P.
- **The ICU IELSR window.** `R_ICU_Type` runs 0x6480 bytes from
  `0x4000_6000`, with `WUPEN`/`IELSR` at `0x4000_C000`+. Zephyr's SCI_B driver
  clears `IELSRn.IR` at init. Only the first 2 KiB were mapped, so that store
  hit a BusFault.

**Stock Zephyr v4.0.0** `samples/drivers/counter/alarm` runs too (Zephyr
v3.7.0 has no Renesas AGT counter driver). Its four AGT0 compare-match
alarms (2, 4, 8, 16 s; `Now: 2/6/14/30`) reach the core only through the
ICU: the AGT raises ELC event 0x047 (`AGT0_COMPARE_A`), `IELSR84` links it
to NVIC line 84 and sets `IR`, and the line follows `IR` as a level until
the handler's `R_BSP_IrqStatusClear` clears it (`ra_icu`, `ra_agt`). The
v4.0.0 board also opens SCI3 (the mikroBUS UART) at boot, so SCI3 is now
mapped. This is the AGT-period evidence of Renode's
`Renesas_RA8M1.robot` "Should Run Periodically Blink LED", on stock Zephyr
instead of an FSP ELF.

**Stock Zephyr v3.7.0** `tests/drivers/uart/uart_basic_api` passes too,
7/7 across its two suites, on the interrupt-driven SCI_B driver. Every SCI
interrupt is an ICU event: SCI9's RXI/TXI/TEI (ELC events 0x163-0x165) are
linked to NVIC lines 24-26 by `IELSRn` (`elc_event` on sci3/sci9). TXI
follows the TDRE edge and TEI the TEND edge of a timed shift register (10-bit
frames at 115200 baud in the 480 MHz guest clock), and RXI is raised once per
character, a frame apart. The driver's default receive FIFO reads `FRSR.R`.
This is the stock-Zephyr counterpart of Renode's `Renesas_RA8M1.robot`
"Should Read And Write On UART", which drives the same SCI_B interrupts from
an FSP ELF.

The stock Zephyr v3.7.0 shell (`samples/subsys/shell/shell_module`) runs on
the same interrupt-driven SCI9: its serial backend is
`CONFIG_SHELL_BACKEND_SERIAL_INTERRUPT_DRIVEN`, so the prompt, the echo and
every reply leave on TXI/TEI and every typed byte arrives on RXI. The example
types commands, arguments, TAB completion and history recall
(`examples/ek-ra8m1/zephyr-shell.yaml`); without SCI9's `elc_event` the
image prints nothing at all.

!!! warning "Not claimed yet"
    The core decodes Armv8-M Mainline plus the scalar Armv8.1-M `DLS`/`WLS`/
    `LE` and `CSEL` family. **Helium/MVE**, the tail-predicated loops, PACBTI
    and `BF*` are not decoded, so firmware built with `-mcpu=cortex-m85`
    and MVE enabled will stop with a decode error.
    **TrustZone** is not modelled (no security state, non-secure aliases not
    mapped). Clock generation is register-level only: oscillators and PLLs
    settle at once, and no rate is derived from the selected sources
    (`cpu_hz` stays 8 MHz after Zephyr moves ICLK to the 480 MHz PLL1P).
    Module stop (MSTP) and the ICU `IRQCRn`/NMI half are declared stubs;
    the AGT and SCI3/SCI9 raise ELC events, the PFS peripheral mux is stored
    only, and SCI_B is async only: the FIFO trigger levels (`RTRG`, the
    `FRSR.DR` timeout), ERI and the error flags, and the `CCR2` baud decode
    are not modelled (RXI comes per character; the frame time is YAML
    `baud`). The AGT models timer mode only. DMAC/DTC,
    FCACHE, FACI, ELC, RTC, IWDT/WDT, SPI, USBFS/USBHS,
    ETHERC and CANFD are declared stubs; ADC0/ADC1 convert software-started
    scans (stock Zephyr `adc_api` 6/6), IIC0/IIC1 are I2C masters (stock
    Zephyr `samples/sensor/bme280` reads a BME280 on IIC1), GPT0-13
    generate saw-wave PWM (stock Zephyr `pwm_api` 2/2, the GTIOC7A waveform
    checked on P603) and DAC0/DAC1 drive their DAn pins (stock Zephyr
    `dac_api` 1/1, and `dac_loopback` 1/1 with DA0 jumpered to AN000).
    SCI0–2, SCI4–8 and the remaining
    blocks are **not mapped**: firmware that touches them faults with a memory violation. There
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
| Reference firmware | [`crates/firmware-ek-ra8m1-smoke/`](../../crates/firmware-ek-ra8m1-smoke/), stock Zephyr via [`crates/firmware-ek-ra8m1-zephyr/`](../../crates/firmware-ek-ra8m1-zephyr/) |
| Example | [`examples/ek-ra8m1/`](../../examples/ek-ra8m1/) |
| Committed artifact | `tests/fixtures/ek-ra8m1-smoke.elf` (`EK-RA8M1 SMOKE`), `tests/fixtures/r7fa8m1ah-zephyr-{hello,sync}.elf` (stock Zephyr v3.7.0), `tests/fixtures/r7fa8m1ah-zephyr-counter-alarm.elf` (stock Zephyr v4.0.0), `tests/fixtures/r7fa8m1ah-zephyr-{uart-basic-api,shell}.elf` (stock Zephyr v3.7.0) |
| Build target | `thumbv8m.main-none-eabi` |
| Tier (snapshot) | **L1** — smoke (CPU + memory + PORT/PFS + SCI9) plus stock Zephyr hello_world/synchronization, the AGT counter alarm, uart_basic_api and the shell (SCI_B interrupts) through the ICU; SIM-DERIVED |

---

## Memory map and board wiring

| Region / signal | Base / pin | State |
|--------|------|-------|
| ITCM | `0x0000_0000`, 64 KiB | mapped RAM (secure alias only) |
| Code flash | `0x0200_0000`, 2 MiB | modelled — the smoke links and runs here |
| Option-setting memory | `0x0300_A100`+ | not mapped — OFS0/OFS1 reset defaults are assumed; the Zephyr ELF's OFS segments are dropped with a loader warning |
| DTCM | `0x2000_0000`, 64 KiB | mapped RAM (secure alias only) |
| SRAM | `0x2200_0000`, 896 KiB | modelled — stack at the top |
| Data flash | `0x2700_0000`, 12 KiB | mapped, erased (reads `0xFF`); no FACI programming |
| SYSTEM | `0x4001_E000` | `ra8_sysc` — every `R_SYSTEM` register stored; `OSCSF` derived from HOCOCR/MOSCCR/PLLCR/PLL2CR; CKSREQ→CKSRDY on every xxxCKCR; no rates derived |
| MSTP | `0x4020_3000` | `type: stub` — module stop not modelled |
| ICU | `0x4000_6000` (IRQCR/NMI) and `0x4000_C000` (WUPEN/IELSR) | IRQCR/NMI `type: stub`; IELSR modelled (`ra_icu`) — ELC event → `IR` → NVIC line level |
| AGT0 / AGT1 | `0x4022_1000` / `0x4022_1100` | modelled (`ra_agt`) — 16-bit timer mode, LOCO/PCLKB sources, underflow + compare A/B as ELC events 0x046–0x04B |
| PORT0–B | `0x4040_0000` + `0x20`×n | modelled (`ra_port`) — PCNTR1/2/3 |
| PFS / PWPR / PWPRS | `0x4040_0800` / `0x4040_0D0C` / `0x4040_0D14` | modelled (`ra8_pfs`) — PCR pull-up, write protection, PDR/PODR stores set the port's `PCNTR1` (stock Zephyr blinky); PmnPFS read-back of PDR/PODR and PIDR not modelled; PMR/PSEL route a pad to the GPT (GTIOC7A/B on P603/P602) and are stored only for other peripherals |
| SCI9 (console) | `0x4035_8900` | modelled — SCI_B, async; RXI/TXI/TEI as ELC events 0x163-0x165; `FRSR.R` |
| SCI3 (mikroBUS) | `0x4035_8300` | modelled — SCI_B, async; RXI/TXI/TEI as ELC events 0x139-0x13B; `FRSR.R` |
| SCI0–2, SCI4–8 | `0x4035_8000` + `0x100`×n | not mapped |
| LEDs | LED1 blue P600, LED2 green P414, LED3 red P107 | modelled GPIO — active high |
| Switches | S1 P009, S2 P008 | modelled GPIO input — active low, PFS pull-up |
| SCI9 VCP | PA14 (TXD9) / PA15 (RXD9) | the simulator captures the SCI9 stream; the J-Link OB bridge is not modelled |
| ADC0 / ADC1 | `0x4033_2000` / `0x4033_2200` | modelled (`ra_adc12`) — software-started single/continuous scans, `ADDRn`, addition/averaging, scan end as ELC events 0x1AE / 0x1B4; nominal 1 µs per channel; group scan, hardware triggers and the temperature sensor not modelled |
| DAC0 / DAC1 | `0x4033_3000` | modelled (`ra_dac12`) — `DADRn` (12 bits, right- or left-justified per `DADPR.DPSEL`) and the control registers read back; DAn = code × 3.3 V / 4096 while `DACR.DAOEn` = 1, at once; an ADC channel's `analog_links` (a board jumper) converts that level; conversion time, amplifier wait, reference select and D/A-A/D sync not modelled |
| IIC0 / IIC1 | `0x4025_E000` / `0x4025_E100` | modelled (`ra_iic`) — RIIC master transmit/receive, 7-bit addresses, start/restart/stop, address NACK, RXI/TXI/TEI/ERI as ELC events 0x05C-0x05F / 0x061-0x064, wire time from `ICBRL`/`ICBRH`/`CKS`; slave mode, 10-bit addresses, arbitration loss, timeout and SMBus not modelled |
| GPT0-13 | `0x4032_2000` + `0x100`×n | modelled (`ra_gpt`) — saw-wave PWM counting PCLKD / 2^`TPCS`, cycle-end buffer transfers (`GTPBR`, `GTCCRC`/`GTCCRE`), compare A/B and overflow as ELC events 0x0A1 + 9n (+0/+1/+6), GTIOCnA/B from `GTIOR` and `GTUDDTYC` 0%/100% on the pads `gtioc_pads` names; triangle/one-shot modes, input capture, hardware sources, dead time and POEG not modelled |
| DMAC/DTC, FCACHE, FACI, ELC, RTC, IWDT, WDT, USBFS, USBHS, ETHERC/EDMAC, SPI0/1, CANFD | see descriptor | `type: stub` |

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
# stock Zephyr v3.7.0 (fixtures committed; rebuild with crates/firmware-ek-ra8m1-zephyr/build.sh)
./target/release/labwired test --script examples/ek-ra8m1/zephyr-hello.yaml --no-uart-stdout
./target/release/labwired test --script examples/ek-ra8m1/zephyr-sync.yaml --no-uart-stdout
# stock Zephyr v4.0.0 counter alarm (AGT0 through the ICU)
./target/release/labwired test --script examples/ek-ra8m1/zephyr-counter-alarm.yaml --no-uart-stdout
# stock Zephyr v3.7.0 uart_basic_api (SCI_B interrupts through the ICU)
./target/release/labwired test --script examples/ek-ra8m1/zephyr-uart-basic-api.yaml --no-uart-stdout
# stock Zephyr v3.7.0 shell (interrupt-driven serial backend)
./target/release/labwired test --script examples/ek-ra8m1/zephyr-shell.yaml --no-uart-stdout
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
