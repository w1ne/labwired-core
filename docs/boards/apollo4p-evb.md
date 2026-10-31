# Apollo4 Plus EVB

The **Ambiq Apollo4 Plus EVB** (ordering code AMAP4PEVB) with an
**AMAP42KP-KBR** Apollo4 Plus SoC (Cortex-M4F, up to 192 MHz): 2 MiB of MRAM
at `0x0` (the secure bootloader owns `0x0`–`0x17FFF`, applications start at
`0x0001_8000`), 384 KiB TCM plus 2.375 MiB of extended/shared SRAM contiguous
from `0x1000_0000`, three active-low user LEDs (`GPIO30`, `GPIO90`,
`GPIO97`), two active-low buttons (`GPIO18`, `GPIO19`) and the console on
**UART0** `GPIO60`/`GPIO47` through the on-board J-Link VCOM.

This is the **UART+LED L1 slice**, the first Ambiq part. Boot from the
post-bootloader vector table, the PWRCTRL power-domain handshake, the UART0
console and the GPIO LED path (single-bit `WTC`/`WTS` strobes with `RD`
read-back) are proven by a committed smoke fixture and an `io-smoke.yaml`.
**Unmodified upstream Zephyr v3.7.0 `samples/hello_world`** boots through
the AmbiqSuite HAL (`zephyr-hello.yaml`): its low-power init, the CPU
block's DAXI flush and the boot ROM's delay helper, which LabWired supplies
as a clean-room table at `0x0800_0000` (`crates/core/roms/apollo4`, not an
Ambiq dump).

!!! warning "Not claimed yet"
    The secure bootloader is not modelled (the engine fast-boots the
    application vector table), and the boot ROM only as the HAL's helper
    table: its MRAM helpers return an error, and `br_util_delay_cycles`
    runs at 2 cycles per iteration instead of 3. Cache/DAXI buffering is
    not modelled. Power domains have no effect on
    their peripherals. The PL011 UART model does not store `CR`, so the
    Ambiq `CLKEN`/`CLKSEL` UART clock selection is written but not checked.
    Non-GPIO pad functions (`FNCSEL` ≠ 3) are not routed, and GPIO interrupts
    are not raised. CLKGEN, MCUCTRL, RSTGEN, IOM0–7, MSPI0–2, ADC, timers,
    STIMER, RTC, WDT, security, crypto, GPU/display, SDIO, USB and audio are
    declared stubs. There is no silicon capture: **SIM-DERIVED**.

!!! tip "Live status"
    The tables below are a maintained snapshot. Authoritative automation:

    - [Chip conformance scoreboard](../coverage/chip-conformance.md) — level · modelled peripherals · register-match

---

## Status at a glance

| Aspect | Status |
|--------|--------|
| Chip descriptor | [`configs/chips/amap42kp.yaml`](../../configs/chips/amap42kp.yaml) |
| Example system | [`configs/systems/apollo4p-evb.yaml`](../../configs/systems/apollo4p-evb.yaml) |
| Reference firmware | [`crates/firmware-apollo4p-evb-smoke/`](../../crates/firmware-apollo4p-evb-smoke/) |
| Example | [`examples/apollo4p-evb/`](../../examples/apollo4p-evb/) |
| Committed artifact | `tests/fixtures/apollo4p-evb-smoke.elf` (`APOLLO4P EVB SMOKE`) |
| Build target | `thumbv7em-none-eabi` |
| Tier (snapshot) | **L1** — smoke (CPU + memory + PWRCTRL + UART0 + GPIO) plus a stock Zephyr boot (boot-ROM helper table, CPU/DAXI block); SIM-DERIVED |

---

## Memory map and board wiring

| Region / signal | Base / pin | State |
|--------|------|-------|
| MRAM (boot ROM handover / SBL) | `0x0000_0000`–`0x0001_7FFF` | not modelled — no SBL image; the engine enters the application vector table |
| MRAM (application) | `0x0001_8000`, to 2 MiB | modelled — the smoke links and runs here (`reset_vector_offset: 0x18000`) |
| RAM | `0x1000_0000`, 2816 KiB (TCM 384 KiB first) | modelled — the smoke's stack is at the top of the TCM (`0x1006_0000`) |
| PWRCTRL | `0x4002_1000` | modelled (`apollo4_pwrctrl`) — MCUPERFREQ ack, DEVPWREN → DEVPWRSTATUS |
| GPIO | `0x4001_0000` | modelled (`apollo4_gpio`) — PADKEY, PINCFG, RD/WT/WTS/WTC/EN banks |
| UART0 (console) | `0x4001_C000` | modelled — PL011 layout, IRQ 15 |
| UART1–3 | `0x4001_D000`–`0x4001_F000` | modelled — same layout, unexercised |
| LEDs | LED0 GPIO30, LED1 GPIO90, LED2 GPIO97 | modelled GPIO — active low, push-pull |
| Buttons | BTN0 GPIO18, BTN1 GPIO19 | modelled GPIO input — active low, PULLCFG pull-up |
| UART0 VCOM | GPIO60 (TX) / GPIO47 (RX), FNCSEL 4 | simulator captures the UART0 stream; the J-Link bridge is not modelled |
| CLKGEN / MCUCTRL / RSTGEN / IOM / MSPI / ADC / timers / RTC / WDT / security / crypto / GPU / display / SDIO / USB / audio | see descriptor | `type: stub` |

## Boot path

**Fast boot past the bootloader.** On silicon the boot ROM runs the secure
bootloader from MRAM `0x0`, which validates and starts the application at
`0x0001_8000`. The engine loads the ELF (whose lowest load address is
`0x0001_8000`), finds no valid vector table at the flash base, and enters the
one at `flash_base + reset_vector_offset` — SP `0x1006_0000`, PC `0x18400`
from the firmware's own table. The SBL is not modelled; the boot ROM is only the clean-room HAL helper table.

## Validation

```bash
# from the repository root
cargo build -p firmware-apollo4p-evb-smoke --release --target thumbv7em-none-eabi
./target/release/labwired test --script examples/apollo4p-evb/io-smoke.yaml --no-uart-stdout
./target/release/labwired test --script examples/apollo4p-evb/zephyr-hello.yaml --no-uart-stdout
```

See [`examples/apollo4p-evb/VALIDATION.md`](../../examples/apollo4p-evb/VALIDATION.md)
for the claims, runtime evidence and explicit non-claims.

## Sources

Ambiq's CMSIS device header `apollo4p.h` and `am_reg_base_addresses.h`
(register maps, bit fields, memory map), the Zephyr `ambiq_apollo4p.dtsi`
SoC tree (application flash offset, IRQs) and the Zephyr `apollo4p_evb`
board port (LED/button/console wiring) — detailed in
[`examples/apollo4p-evb/REQUIRED_DOCS.md`](../../examples/apollo4p-evb/REQUIRED_DOCS.md).
