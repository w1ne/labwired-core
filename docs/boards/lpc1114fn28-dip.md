# LPC1114FN28 (DIP-28, breadboard)

The **NXP LPC1114FN28/102**, a Cortex-M0 in a DIP-28 package with 32 KiB of
flash at `0x0000_0000` and 4 KiB of SRAM at `0x1000_0000`. NXP sells it as a
bare part (there is no current NXP evaluation board for it), so the board
manifest is the minimal breadboard circuit the smoke assumes: an LED on
`PIO0_7` and a 3.3 V USB-UART adapter on `PIO1_7` (TXD) / `PIO1_6` (RXD).

This is the **UART+LED L1 slice** and the first NXP LPC11xx part. Boot, NXP's
CMSIS `SystemInit` clock sequence, the UART console and the GPIO path on the
LED pad (address-masked `DATA` writes and reads, plus edge interrupts) are
proven by a committed smoke fixture and an `io-smoke.yaml`. Two models are
new: `lpc11xx_gpio` and the `lpc11xx` UART layout (a 16C550 at a 4-byte
stride with `LCR.DLAB`, `FDR` and `TER`). SYSCON and IOCON are declarative
register banks generated from NXP's CMSIS-SVD.

!!! warning "Not claimed yet"
    The boot ROM (ISP, IAP and the vector-table checksum check) is not
    modelled: the engine boots the flash vector table directly. Clock
    frequencies are not modelled: `SYSPLLSTAT.LOCK` follows the PLL power-up
    bit, but the core runs at the fixed 48 MHz `cpu_hz` whatever
    `SYSPLLCTRL`/`MAINCLKSEL` select. IOCON is stored only, so pin functions,
    pulls and open drain have no effect (every pad is a GPIO with the reset
    pull-up). The UART interrupt sources and the start-logic wake-up
    interrupts are not modelled. I2C, WWDT, the four counter/timers, ADC, PMU,
    the flash controller and SSP0 are declared stubs. There is no silicon
    capture: **SIM-DERIVED**.

!!! tip "Live status"
    The tables below are a maintained snapshot. Authoritative automation:

    - [Chip conformance scoreboard](../coverage/chip-conformance.md) — level · modelled peripherals · register-match

---

## Status at a glance

| Aspect | Status |
|--------|--------|
| Chip descriptor | [`configs/chips/lpc1114fn28.yaml`](../../configs/chips/lpc1114fn28.yaml) |
| Example system | [`configs/systems/lpc1114fn28-dip.yaml`](../../configs/systems/lpc1114fn28-dip.yaml) |
| Reference firmware | [`crates/firmware-lpc1114fn28-smoke/`](../../crates/firmware-lpc1114fn28-smoke/) |
| Example | [`examples/lpc1114fn28-dip/`](../../examples/lpc1114fn28-dip/) |
| Committed artifact | `tests/fixtures/lpc1114fn28-smoke.elf` (`LPC1114FN28 SMOKE`) |
| Build target | `thumbv6m-none-eabi` |
| Tier (snapshot) | **L1** — smoke (Cortex-M0 + memory + SYSCON + UART + GPIO); SIM-DERIVED |

---

## Memory map and board wiring

| Region / signal | Base / pin | State |
|--------|------|-------|
| Flash | `0x0000_0000`, 32 KiB | modelled — vector table at the base |
| SRAM | `0x1000_0000`, 4 KiB | modelled — stack at the top |
| Boot ROM | `0x1FFF_0000`, 16 KiB | **not mapped** |
| I2C, WWDT, CT16B0/1, CT32B0/1, ADC, PMU, flash controller, SSP0 | `0x4000_0000` … `0x4004_0000` | `type: stub` |
| UART | `0x4000_8000`, IRQ 21 | modelled (`uart`, `profile: lpc11xx`) |
| IOCON | `0x4004_4000` | declarative, stored only |
| SYSCON | `0x4004_8000` | declarative; `SYSPLLSTAT.LOCK` follows `PDRUNCFG.SYSPLL_PD`; `DEVICE_ID` `0x1A40902B` |
| GPIO0–3 (`pio0`–`pio3`) | `0x5000_0000` + `0x1_0000`×n, IRQ 31–28 | modelled (`lpc11xx_gpio`) |
| LED | `PIO0_7` | modelled GPIO — active high (breadboard LED to GND); the 20 mA high-drive pad, LED1 on the mbed LPC1114FN28 |
| Console | `PIO1_7` TXD / `PIO1_6` RXD | the simulator captures the UART stream; the USB-UART adapter is not modelled |

The ports are named `pio0`–`pio3` rather than `gpio0`–`gpio3`: the bus treats
a `gpio0`/`gpio1` port as Nordic and runs its GPIOTE edge-service pass every
cycle, which nothing on this chip consumes (it doubled the per-step cost).

## Boot path

The engine loads SP/PC from the vector table at `0x0000_0000` (the CLI logs
`Initial PC: 0xc0, SP: 0x10001000`). On silicon the boot ROM runs first,
checks that vector entries 0–7 sum to zero and starts the image only then;
the ISP tools (lpc21isp, Flash Magic) write that checksum into entry 7 when
they program the part. The committed fixture is the raw `cortex-m-rt` image,
with entry 7 left at zero.

## Validation

```bash
# from the repository root
cargo build -p firmware-lpc1114fn28-smoke --release --target thumbv6m-none-eabi
./target/release/labwired test --script examples/lpc1114fn28-dip/io-smoke.yaml --no-uart-stdout
```

See [`examples/lpc1114fn28-dip/VALIDATION.md`](../../examples/lpc1114fn28-dip/VALIDATION.md)
for the claims, runtime evidence and explicit non-claims.

## Sources

NXP's CMSIS-SVD `LPC11xx_v6a.svd` (peripheral bases, IRQ numbers, register
offsets, access and reset values), the CMSIS `system_LPC11xx.c` and mbed-os
`serial_api.c`/`LPC1114.ld` (clock bring-up, UART divisor sequence, memory)
and the lpc21isp part table (memory sizes, `DEVICE_ID`) — detailed in
[`examples/lpc1114fn28-dip/REQUIRED_DOCS.md`](../../examples/lpc1114fn28-dip/REQUIRED_DOCS.md).
