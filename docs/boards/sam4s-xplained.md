# SAM4S Xplained

The **SAM4S Xplained** (Atmel/Microchip): an **ATSAM4S16C** (Cortex-M4, 120 MHz), 1 MiB of flash at `0x00400000`, 128 KiB of SRAM at `0x20000000`, a user LED on `PC10`, a user button on `PA5` (active low), and UART0 on `PA9`/`PA10`. Zephyr's board documentation identifies UART0 as connected to the onboard SEGGER J-Link OB and exposed as a virtual COM port. The simulator captures UART output directly; it does not model the USB bridge or establish physical acceptance.

This is the **first SAM4S part in the engine**. It is an **L1 (smoke)** target: boot, the PMC clock enables, the UART0 console and the PIO LED/button path (with `PIO_ODSR` read-back) are proven by a committed smoke fixture and an `io-smoke.yaml`. The SAM4S PIO is the older parallel-I/O block (`PIO_PER`/`PIO_OER`/`PIO_SODR`/`PIO_CODR`/`PIO_ODSR`/`PIO_PSR`), not the SAM D21 `PORT` block; it is modelled by the new `sam4s_pio` peripheral, and `sam4s_pmc` models the PMC clock enables and master-clock-ready status. No silicon capture exists: **SIM-DERIVED**, no executing-fidelity differential.

!!! tip "Live status"
    The tables below are a maintained snapshot. Authoritative automation:

    - [Chip conformance scoreboard](../coverage/chip-conformance.md) — level · modelled peripherals · register-match

---

## Status at a glance

| Aspect | Status |
|--------|--------|
| Chip descriptor | [`configs/chips/sam4s16c.yaml`](../../configs/chips/sam4s16c.yaml) |
| Example system | [`configs/systems/sam4s-xplained.yaml`](../../configs/systems/sam4s-xplained.yaml) |
| Reference firmware | [`crates/firmware-sam4s-xplained-smoke/`](../../crates/firmware-sam4s-xplained-smoke/) |
| Example | [`examples/sam4s-xplained/`](../../examples/sam4s-xplained/) |
| Committed artifact | `tests/fixtures/sam4s-xplained-smoke.elf` (`SAM4S XPLAINED SMOKE`) |
| Build target | `thumbv7em-none-eabi` |
| Tier (snapshot) | **L1** — smoke (CPU + memory + UART0 + PIO); SIM-DERIVED |

---

## Memory map and board wiring

| Region / signal | Base / pin | State |
|--------|------|-------|
| Flash | `0x0040_0000`, 1 MiB | modelled — the smoke links and runs here |
| SRAM | `0x2000_0000`, 128 KiB | modelled — data/stack |
| PMC | `0x400E_0400` | modelled (`sam4s_pmc`) — `PCER0`/`PCSR0` enables, `SR` MOSCXTS/MCKRDY |
| UART0 / UART1 | `0x400E_0600` / `0x400E_0800` | modelled — `sam` USART layout (SR/THR), UART0 is the console |
| USART0 / USART1 | `0x4002_4000` / `0x4002_8000` | modelled — `sam` layout |
| PIOA / PIOB / PIOC | `0x400E_0E00` / `0x400E_1000` / `0x400E_1200` | modelled (`sam4s_pio`) — PER/OER/SODR/CODR/ODSR/PSR/PDSR, pull-ups |
| LED | PC10 | modelled PIO — active high |
| Button | PA5 | modelled PIO — active low, internal pull-up enabled by the firmware |
| UART0 VCP / console | PA9 / PA10 | physical J-Link OB VCP documented by Zephyr; simulator captures the UART stream, without modeling USB |
| TC0/TC1, SPI0, TWI0/TWI1 | see descriptor | `type: stub` — zeros |
| EEFC, RSTC/SUPC/RTT/WDT/RTC, CHIPID | see descriptor | `type: stub` |

## Boot path

**Fast boot.** The engine loads the ELF and enters it at its entry point
(the flash base, `0x0040_0000`) with SP seeded at the top of SRAM. The SAM4S
ROM/bootloader path is not modelled.

## Validation

```bash
cd core
cargo build -p firmware-sam4s-xplained-smoke --release --target thumbv7em-none-eabi
./target/release/labwired test --script examples/sam4s-xplained/io-smoke.yaml --no-uart-stdout
```

See [`examples/sam4s-xplained/VALIDATION.md`](../../examples/sam4s-xplained/VALIDATION.md) for the claims, runtime evidence and explicit non-claims.

## Sources

The SAM4S datasheet (Atmel 11100) for the PIO/PMC/UART/PIO register maps, and
the Renode `sam4s`/`sam4s16c`/`sam4s_xplained` platforms for the addresses and
part sizes — detailed in
[`examples/sam4s-xplained/REQUIRED_DOCS.md`](../../examples/sam4s-xplained/REQUIRED_DOCS.md).
