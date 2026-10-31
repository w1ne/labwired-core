# SAM E70 Xplained

The **Microchip SAM E70 Xplained** evaluation kit (ATSAME70-XPLD) with an
**ATSAME70Q21B** (Cortex-M7, 300 MHz max): 2 MiB of flash at `0x0040_0000`,
384 KiB of SRAM at `0x2040_0000`, one active-low user LED (LED0, `PC8`), one
active-low user button (SW0, `PA11`) and the console on **USART1**
(`PA21` RXD1 / `PB4` TXD1), the EDBG virtual COM port.

This is the **USART+LED L1 slice**. Boot, the USART1 console and the PIO LED
path (with `PIO_ODSR` and pad read-back) are proven by a committed smoke
fixture and an `io-smoke.yaml`. It reuses the SAM4S PIO and PMC models, whose
register layouts the SAM E70 keeps, at the SAM E70 bases.

**Stock Zephyr boots.** Unmodified upstream Zephyr v3.7.0 `hello_world` and
`synchronization` for `sam_e70_xplained/same70q21b` run their own SoC bring-up
and print on USART1: the Cortex-M7 L1 cache set/way loops (sized from the
CCSIDR geometry the SCB reports), the PMC clock tree (main RC → 12 MHz
crystal, PLLA ×25 and the UPLL locked, MCK on PLLA /2) and the CHIPID check.
The synchronization sample also proves SysTick timeouts, a semaphore hand-off
and the PendSV context switch.

!!! warning "Not claimed yet"
    Clock frequencies are not modelled: the PMC tracks which sources are
    enabled (CKGR_MOR with its 0x37 key, PLLA/UPLL lock, PMC_SR derived from
    them; every source is stable at once and an MCK switch completes at once),
    but PLL multipliers and MCKR do not change the simulated core rate. The
    Cortex-M7 L1 caches report their 16 KiB + 16 KiB geometry and accept every
    maintenance operation, but hold no data: memory is always coherent, so a
    missing clean/invalidate around DMA is not caught. The PIO peripheral select
    (`ABCDSR1/2`) is stored only, and the bus-matrix `CCFG_SYSIO` select is a
    stub, so pad routing has no effect. EEFC and the RSTC/SUPC/RTT/WDT/RTC
    system controller are declared stubs: the watchdog never fires. The TCM,
    the MPU (Zephyr's regions are stored, not enforced) and the FPU are not
    exercised. GMAC, TRNG, TC0–3, SPI0/1,
    QSPI, TWIHS0–2, MCAN0/1, AFEC0/1, DACC, PWM0/1, HSMCI, SSC, USBHS and
    XDMAC are **not mapped**: firmware that touches them faults with a memory
    violation. There is no silicon capture: **SIM-DERIVED**.

!!! tip "Live status"
    The tables below are a maintained snapshot. Authoritative automation:

    - [Chip conformance scoreboard](../coverage/chip-conformance.md) — level · modelled peripherals · register-match

---

## Status at a glance

| Aspect | Status |
|--------|--------|
| Chip descriptor | [`configs/chips/same70q21b.yaml`](../../configs/chips/same70q21b.yaml) |
| Example system | [`configs/systems/sam-e70-xplained.yaml`](../../configs/systems/sam-e70-xplained.yaml) |
| Reference firmware | [`crates/firmware-sam-e70-xplained-smoke/`](../../crates/firmware-sam-e70-xplained-smoke/) |
| Example | [`examples/sam-e70-xplained/`](../../examples/sam-e70-xplained/) |
| Committed artifact | `tests/fixtures/sam-e70-xplained-smoke.elf` (`SAM E70 XPLAINED SMOKE`) |
| Stock Zephyr | `tests/fixtures/same70q21b-zephyr-hello.elf`, `same70q21b-zephyr-sync.elf` (Zephyr v3.7.0, [`crates/firmware-sam-e70-xplained-zephyr/build.sh`](../../crates/firmware-sam-e70-xplained-zephyr/build.sh)) |
| Build target | `thumbv7em-none-eabi` |
| Tier (snapshot) | **L1** — smoke (CPU + memory + PMC + PIO + USART1) + stock Zephyr hello_world/synchronization; SIM-DERIVED |

---

## Memory map and board wiring

| Region / signal | Base / pin | State |
|--------|------|-------|
| Flash | `0x0040_0000`, 2 MiB | modelled — the smoke links and runs here |
| SRAM | `0x2040_0000`, 384 KiB | modelled — stack at the top; the ITCM/DTCM split is not modelled |
| PMC | `0x400E_0600` | `sam4s_pmc` — PCER/PCDR/PCSR banks 0/1, SCER/SCSR, CKGR_MOR (keyed)/PLLAR/UCKR, PMC_SR derived (MOSCXTS/MOSCRCS/LOCKA/LOCKU; MCKRDY always set) |
| CHIPID | `0x400E_0940` | declarative — CIDR `0xA102_0E01` (rev B), EXID `0x2` |
| UART0/1 | `0x400E_0800` / `0x400E_0A00` | modelled (`sam` UART layout), not exercised |
| EEFC | `0x400E_0C00` | `type: stub` |
| PIOA–PIOE | `0x400E_0E00` + `0x200`×n | modelled (`sam4s_pio`) |
| RSTC / SUPC / RTT / WDT / RTC | `0x400E_1800` / `1810` / `1830` / `1850` / `1860` | `type: stub` |
| UART2–4 | `0x400E_1A00` + `0x200`×n | modelled (`sam` UART layout), not exercised |
| USART0–2 | `0x4002_4000` + `0x4000`×n | modelled (`sam` UART layout); USART1 is the console |
| Bus matrix | `0x4008_8000` | `type: stub` — `CCFG_SYSIO` (+0x114) has no effect |
| Cortex-M7 L1 caches | SCB `0xE000_ED78`–`ED84`, maintenance `0xE000_EF50` | `cortex_m7_cache` — 16 KiB I + 16 KiB D geometry in CLIDR/CTR/CCSIDR; maintenance ops are no-ops (no cache contents) |
| LED0 | `PC8` | modelled GPIO — active low |
| SW0 | `PA11` | modelled GPIO input — active low, PIO pull-up |
| USART1 VCP | `PA21` (RXD1, periph A) / `PB4` (TXD1, periph D) | the simulator captures the USART1 stream; the EDBG bridge is not modelled |

## Boot path

**Fast boot.** The engine loads the ELF and enters it through the vector table
at `0x0040_0000` (reset handler `0x0040_0400`, SP `0x2046_0000`). On silicon
GPNVM bit 1 selects the boot source (set = flash, clear = the SAM-BA boot
ROM); the engine always boots the flash image and does not model the ROM.

## Validation

```bash
# from the repository root
cargo build -p firmware-sam-e70-xplained-smoke --release --target thumbv7em-none-eabi
./target/release/labwired test --script examples/sam-e70-xplained/io-smoke.yaml --no-uart-stdout
# stock Zephyr v3.7.0 (committed fixtures)
./target/release/labwired test --script examples/sam-e70-xplained/zephyr-hello.yaml --no-uart-stdout
./target/release/labwired test --script examples/sam-e70-xplained/zephyr-sync.yaml --no-uart-stdout
```

See [`examples/sam-e70-xplained/VALIDATION.md`](../../examples/sam-e70-xplained/VALIDATION.md)
for the claims, runtime evidence and explicit non-claims.

## Sources

The SAM E70/S70/V70/V71 datasheet (Microchip DS60001527), the Zephyr
`sam_e70_xplained` board and `samx7x` SoC devicetree (peripheral bases,
interrupt numbers, memory sizes, LED/button/console wiring), and the Renode
v1.17.0 `sam_e70.repl` as a cross-check of the bases — detailed in
[`examples/sam-e70-xplained/REQUIRED_DOCS.md`](../../examples/sam-e70-xplained/REQUIRED_DOCS.md).
