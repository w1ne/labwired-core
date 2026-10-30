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

!!! warning "Not claimed yet"
    Clock generation is not modelled: the PMC reports the master clock ready
    and the smoke stays on the 12 MHz main RC. The PIO peripheral select
    (`ABCDSR1/2`) is stored only, and the bus-matrix `CCFG_SYSIO` select is a
    stub, so pad routing has no effect. EEFC and the RSTC/SUPC/RTT/WDT/RTC
    system controller are declared stubs: the watchdog never fires. The
    caches, TCM, MPU and FPU are not exercised. GMAC, TRNG, TC0–3, SPI0/1,
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
| Build target | `thumbv7em-none-eabi` |
| Tier (snapshot) | **L1** — smoke (CPU + memory + PMC + PIO + USART1); SIM-DERIVED |

---

## Memory map and board wiring

| Region / signal | Base / pin | State |
|--------|------|-------|
| Flash | `0x0040_0000`, 2 MiB | modelled — the smoke links and runs here |
| SRAM | `0x2040_0000`, 384 KiB | modelled — stack at the top; the ITCM/DTCM split is not modelled |
| PMC | `0x400E_0600` | `sam4s_pmc` — PCER/PCDR/PCSR banks 0/1, SCER/SCSR, MCKRDY always set |
| UART0/1 | `0x400E_0800` / `0x400E_0A00` | modelled (`sam` UART layout), not exercised |
| EEFC | `0x400E_0C00` | `type: stub` |
| PIOA–PIOE | `0x400E_0E00` + `0x200`×n | modelled (`sam4s_pio`) |
| RSTC / SUPC / RTT / WDT / RTC | `0x400E_1800` / `1810` / `1830` / `1850` / `1860` | `type: stub` |
| UART2–4 | `0x400E_1A00` + `0x200`×n | modelled (`sam` UART layout), not exercised |
| USART0–2 | `0x4002_4000` + `0x4000`×n | modelled (`sam` UART layout); USART1 is the console |
| Bus matrix | `0x4008_8000` | `type: stub` — `CCFG_SYSIO` (+0x114) has no effect |
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
```

See [`examples/sam-e70-xplained/VALIDATION.md`](../../examples/sam-e70-xplained/VALIDATION.md)
for the claims, runtime evidence and explicit non-claims.

## Sources

The SAM E70/S70/V70/V71 datasheet (Microchip DS60001527), the Zephyr
`sam_e70_xplained` board and `samx7x` SoC devicetree (peripheral bases,
interrupt numbers, memory sizes, LED/button/console wiring), and the Renode
v1.17.0 `sam_e70.repl` as a cross-check of the bases — detailed in
[`examples/sam-e70-xplained/REQUIRED_DOCS.md`](../../examples/sam-e70-xplained/REQUIRED_DOCS.md).
