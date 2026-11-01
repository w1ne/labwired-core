# EK-RA6M5

The **Renesas EK-RA6M5** evaluation kit with an **R7FA6M5BH3CFC** (RA6M5,
Cortex-M33 with TrustZone, 200 MHz max): 2 MiB of code flash at `0x0000_0000`,
512 KiB of SRAM at `0x2000_0000`, three active-high user LEDs (`P006`–`P008`),
two active-low switches (`P005`, `P004`) and the console on **SCI0**
`P411`/`P410`, the J-Link OB virtual COM port.

This is the **UART+LED L1 slice**. Boot, the SCI0 console and the GPIO LED path
(with `PCNTR1` PODR read-back) are proven by a committed smoke fixture and an
`io-smoke.yaml`. It reuses the RA4M1 PORT, PFS and SCI models at the RA6M5
bases.

**Stock Zephyr boots.** Unmodified upstream Zephyr v4.0.0 `samples/hello_world`
and `samples/synchronization` for `ek_ra6m5` run to their banners
(`tests/fixtures/r7fa6m5bh-zephyr-{hello,sync}.elf`, rebuilt byte-identical by
[`crates/firmware-ek-ra6m5-zephyr/build.sh`](../../crates/firmware-ek-ra6m5-zephyr/build.sh);
Zephyr v3.7.0 has no `ek_ra6m5` board). What that needed:

- **The FSP clock bring-up.** `ra6_sysc` is a full `R_SYSTEM` register file
  (0x700 bytes, `R7FA6M5BH.h`): every register reads back, `OSCSF` is derived
  from the HOCO/MOSC/PLL/PLL2 stop bits, and the USB/OCTA/CANFD `xxxCKCR`
  switches complete the CKSREQ/CKSRDY handshake. It shares its model with
  the RA8M1's `ra8_sysc`, with a per-family layout.
- **SCI0 in FIFO mode.** FSP's `r_sci_uart` opens SCI0 with `FCR.FM` and
  waits for the `RFRST`/`TFRST` reset bits to read back 0; Zephyr's
  `poll_out` waits for `FDR.T` ≤ 8 and writes `FTDRL`. The `sci` UART layout
  now has `FCR`, `FDR`, `FTDRL` and `FRDRL`.
- **AGT through the ICU event link** (stock Zephyr v4.1.0
  `samples/drivers/counter/alarm`, `tests/fixtures/r7fa6m5bh-zephyr-counter-alarm.elf`;
  v4.0.0 has no RA6 `agt` nodes). The ICU at `0x4000_6000` is `ra_icu`: an
  ELC event sets `IELSRn.IR` of every line linked to it and drives NVIC
  line n as a level that drops when firmware clears `IR`. AGT0–5 are
  `ra_agt` (16-bit, timer mode; events `0x040 + 3n` INT / `+1` compare A /
  `+2` compare B, FSP ra6m5 `bsp_elc.h`). The sample's four AGT0 compare
  alarms (Now: 2/6/14/30 s) arrive on IELSR84, then the 65536-tick request
  is refused. Same models as the EK-RA8M1, at the RA6M5 bases.
- **SCI0 interrupts through the ICU** (stock Zephyr v4.0.0
  `tests/drivers/uart/uart_basic_api`,
  `tests/fixtures/r7fa6m5bh-zephyr-uart-basic-api.elf`: 6/6 + 1/1). SCI0
  raises RXI/TXI/TEI as ELC events `0x180`/`0x181`/`0x182` (FSP ra6m5
  `bsp_elc.h`), which Zephyr links to NVIC lines 0-2. `SCR` is stored, and
  its TIE/RIE/TE/RE/TEIE enables drive the same interrupt engine as the
  RA8M1's SCI_B: TXI when a byte moves into the timed shift register
  (one 10-bit frame at 115200 baud of the 200 MHz guest clock), TEI when
  the last frame ends, RXI once per received character a frame apart. In
  FIFO mode `SSR` reads as `SSR_FIFO` (TDFE, TEND, RDF at `FCR.RTRG`, DR
  below it). The FSP UART suite drives fifo_fill on TXI/TEI and fifo_read
  and the pending suite on RXI; without the events fifo_fill fails and
  fifo_read hangs.
- **Stock Zephyr shell** (v4.0.0 `samples/subsys/shell/shell_module`,
  `tests/fixtures/r7fa6m5bh-zephyr-shell.elf`, the sample of Renode's
  `shell_module.robot`). Its serial backend is interrupt driven, so the
  prompt, the echo and every reply leave on SCI0 TXI/TEI and every typed
  byte arrives on RXI. `examples/ek-ra6m5/zephyr-shell.yaml` types commands,
  arguments, TAB completion and history recall; without the sci0
  `elc_event` the image prints nothing.
- **Declared stubs** for `R_FCACHE` (the FCACHEIV invalidate completes on a
  0 read), the C-cache (`R_CACHE`) and the TrustZone attribution
  (`R_PSCU`/`R_CPSCU`), which Zephyr's `soc_early_init_hook` and
  `bsp_clock_init` write.

!!! warning "Not claimed yet"
    No clock rate is derived from `SCKSCR`/`SCKDIVCR`/`PLLCCR`: oscillators
    settle at once and `cpu_hz` stays the reset MOCO 8 MHz after Zephyr
    switches ICLK to the 200 MHz PLL (the SysTick-based Zephyr timebase
    counts cycles, so it is unaffected). Module stop (MSTP) is a declared
    stub. The ICU `IRQCRn`/NMI pin half and `DTCE` are stored only.
    SCI0 raises no ERI and no receive error flags, and `SMR`/`BRR`/`SEMR`
    are not decoded (`baud` comes from the chip YAML). The AGT pulse output, event counter and
    measurement modes are not modelled. The PFS
    peripheral mux is stored only. The SCI FIFO has no TX occupancy (TX
    drains at once, so `FDR.T` reads 0); `DR` is set at once rather than
    15 ETU after the last stop bit, and RXI comes per character rather
    than at `RTRG` or the `DR` timeout. TrustZone,
    flash programming, DMAC/DTC, ELC, RTC, IWDT/WDT, SPI,
    USBFS/USBHS, ETHERC and CANFD are declared stubs; ADC0/ADC1 convert
    software-started scans (stock Zephyr `adc_api` 6/6), IIC0-2 are I2C
    masters (stock Zephyr `samples/sensor/bme280` reads a BME280 on IIC1)
    and GPT0-9 generate saw-wave PWM (stock Zephyr `pwm_api` 2/2, the
    GTIOC1A waveform checked on P405). SCI1–9 and the
    remaining blocks are **not mapped**: firmware that touches them faults
    with a memory violation. Option-setting memory is not mapped either; the
    loader warns and drops Zephyr's OFS segments. There is no silicon
    capture: **SIM-DERIVED**.

!!! tip "Live status"
    The tables below are a maintained snapshot. Authoritative automation:

    - [Chip conformance scoreboard](../coverage/chip-conformance.md) — level · modelled peripherals · register-match

---

## Status at a glance

| Aspect | Status |
|--------|--------|
| Chip descriptor | [`configs/chips/r7fa6m5bh.yaml`](../../configs/chips/r7fa6m5bh.yaml) |
| Example system | [`configs/systems/ek-ra6m5.yaml`](../../configs/systems/ek-ra6m5.yaml) |
| Reference firmware | [`crates/firmware-ek-ra6m5-smoke/`](../../crates/firmware-ek-ra6m5-smoke/); stock Zephyr v4.0.0 / v4.1.0 via [`crates/firmware-ek-ra6m5-zephyr/`](../../crates/firmware-ek-ra6m5-zephyr/) |
| Example | [`examples/ek-ra6m5/`](../../examples/ek-ra6m5/) |
| Committed artifacts | `tests/fixtures/ek-ra6m5-smoke.elf` (`EK-RA6M5 SMOKE`); `tests/fixtures/r7fa6m5bh-zephyr-hello.elf`, `r7fa6m5bh-zephyr-sync.elf`, `r7fa6m5bh-zephyr-counter-alarm.elf`, `r7fa6m5bh-zephyr-uart-basic-api.elf`, `r7fa6m5bh-zephyr-shell.elf` |
| Build target | `thumbv8m.main-none-eabi` |
| Tier (snapshot) | **L1** — smoke (CPU + memory + PORT/PFS + SCI0); SIM-DERIVED |

---

## Memory map and board wiring

| Region / signal | Base / pin | State |
|--------|------|-------|
| Code flash | `0x0000_0000`, 2 MiB | modelled — the smoke links and runs here |
| Data flash | `0x0800_0000`, 8 KiB | not mapped |
| Option-setting memory | `0x0100_A100`+ | not mapped — OFS0/OFS1 reset defaults are assumed; Zephyr's OFS segments are dropped with a loader warning |
| SRAM | `0x2000_0000`, 512 KiB | modelled — stack at the top |
| SYSTEM | `0x4001_E000` | `ra6_sysc` — every `R_SYSTEM` register stored (0x700 bytes); `OSCSF` derived from HOCOCR/MOSCCR/PLLCR/PLL2CR; CKSREQ→CKSRDY on USBCKCR/OCTACKCR/CANFDCKCR |
| FCACHE | `0x4001_C000` | `type: stub` — FCACHEIV reads 0, FLWT dropped |
| C-cache | `0x4000_7000` | `type: stub` — no cache modelled |
| CPSCU / PSCU | `0x4000_8000` / `0x400E_0000` | `type: stub` — no security attribution |
| MSTP | `0x4008_4000` | `type: stub` — module stop not modelled |
| ICU | `0x4000_6000` | `ra_icu` — IELSR0–95 link ELC events to NVIC lines 0–95 (`IR` level, W0C); IRQCRn/NMI stored only |
| AGT0–5 | `0x400E_8000` + `0x100`×n | `ra_agt` — 16-bit timer mode, underflow and compare A/B as ELC events (`0x040 + 3n`) |
| PORT0–B | `0x4008_0000` + `0x20`×n | modelled (`ra_port`) — PCNTR1/2/3; 16-bit aliases upper half first (PODR +0x0, PDR +0x2, as `R7FA6M5BH.h`) |
| PFS / PWPR | `0x4008_0800` / `0x4008_0D03` | modelled (`ra_pfs`) — PCR pull-up, write protection, PDR/PODR stores set the port's `PCNTR1` (stock Zephyr blinky); PmnPFS read-back of PDR/PODR and PIDR not modelled; PMR/PSEL route a pad to the GPT (GTIOC1A/B on P405/P406) and are stored only for other peripherals |
| SCI0 (console) | `0x4011_8000` | modelled — classic SCI, async; FIFO mode (`FCR`/`FDR`/`FTDRL`/`FRDRL`, `SSR_FIFO`); RXI/TXI/TEI as ICU events |
| SCI1–9 | `0x4011_8100` + `0x100`×n | not mapped |
| LEDs | LED1 blue P006, LED2 green P007, LED3 red P008 | modelled GPIO — active high |
| Switches | S1 P005, S2 P004 | modelled GPIO input — active low, PFS pull-up |
| SCI0 VCP | P411 (TXD0) / P410 (RXD0) | the simulator captures the SCI0 stream; the J-Link OB bridge is not modelled |
| ADC0 / ADC1 | `0x4017_0000` / `0x4017_0200` | modelled (`ra_adc12`) — software-started single/continuous scans, `ADDRn`, addition/averaging, scan end as ELC events 0x160 / 0x166; nominal 1 µs per channel; group scan, hardware triggers and the temperature sensor not modelled |
| IIC0 / IIC1 / IIC2 | `0x4009_F000` + `0x100`×n | modelled (`ra_iic`) — RIIC master transmit/receive, 7-bit addresses, start/restart/stop, address NACK, RXI/TXI/TEI/ERI as ELC events 0x073 + 5n .. +3, wire time from `ICBRL`/`ICBRH`/`CKS`; slave mode, 10-bit addresses, arbitration loss, timeout and SMBus not modelled |
| GPT0-9 | `0x4016_9000` + `0x100`×n | modelled (`ra_gpt`) — saw-wave PWM counting PCLKD / 2^`TPCS`, cycle-end buffer transfers (`GTPBR`, `GTCCRC`/`GTCCRE`), compare A/B and overflow as ELC events 0x0C0 + 9n (+0/+1/+6), GTIOCnA/B from `GTIOR` and `GTUDDTYC` 0%/100% on the pads `gtioc_pads` names; triangle/one-shot modes, input capture, hardware sources, dead time and POEG not modelled |
| DMAC/DTC, ELC, RTC, IWDT, WDT, USBFS, CANFD, USBHS, ETHERC/EDMAC, SPI0/1, FACI | see descriptor | `type: stub` |

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
# stock Zephyr v4.0.0 (committed fixtures)
./target/release/labwired test --script examples/ek-ra6m5/zephyr-hello.yaml --no-uart-stdout
./target/release/labwired test --script examples/ek-ra6m5/zephyr-sync.yaml --no-uart-stdout
# stock Zephyr v4.1.0 counter alarm (AGT0 compare match through ICU IELSR84)
./target/release/labwired test --script examples/ek-ra6m5/zephyr-counter-alarm.yaml --no-uart-stdout
# stock Zephyr v4.0.0 uart_basic_api (SCI0 RXI/TXI/TEI through ICU IELSR0-2)
./target/release/labwired test --script examples/ek-ra6m5/zephyr-uart-basic-api.yaml --no-uart-stdout
# stock Zephyr v4.0.0 shell (interrupt-driven serial backend on SCI0)
./target/release/labwired test --script examples/ek-ra6m5/zephyr-shell.yaml --no-uart-stdout
```

See [`examples/ek-ra6m5/VALIDATION.md`](../../examples/ek-ra6m5/VALIDATION.md)
for the claims, runtime evidence and explicit non-claims.

## Sources

The Renesas FSP CMSIS header `R7FA6M5BH.h` (peripheral bases and register
offsets), the Zephyr `ek_ra6m5` board and RA6M5 SoC devicetree (memory sizes,
LED/switch/console wiring, SCI0 clock gate `MSTPB31`), and the Renode v1.17.0
`renesas-r7fa6m5b.repl` as a cross-check of the bus map — detailed in
[`examples/ek-ra6m5/REQUIRED_DOCS.md`](../../examples/ek-ra6m5/REQUIRED_DOCS.md).
