# MIMXRT595-EVK

The **NXP MIMXRT595-EVK** evaluation board with an **i.MX RT595S**
(MIMXRT595SFFOC). Only the Cortex-M33 is modelled: the image runs XIP from
the FlexSPI0 NOR window at `0x0800_0000` (the FlexSPI configuration block at
`0x0800_0400`, the vector table at `0x0800_1000`), data lives in the 5 MiB
on-chip SRAM at `0x2000_0000`, the RGB LED is on `P0_14`/`P1_0`/`P3_17` and
the console is **FLEXCOMM0** in USART mode.

This is the **FLEXCOMM USART+LED L1 slice** and the first i.MX RT500 part.
Boot, the FLEXCOMM0 console and the high-speed GPIO path on the RGB LED
(through `SET`, the byte and word pin arrays, `NOT`, `MPIN` under `MASK` and
`CLR`, with `PIN` read-back) plus the SW1/SW2 inputs are proven by a committed
smoke fixture and an `io-smoke.yaml`. Three models are new: the LPC-style
FLEXCOMM USART (`lpc_flexcomm_usart`), the LPC-style GPIO (`lpc_gpio`) and the
CLKCTL/RSTCTL/IOPCTL register banks (`rt5xx_ctl` / `rt5xx_iopctl`). The RT500
peripherals are LPC-family blocks, not the i.MX RT1xxx LPUART/GPIO/IOMUXC, so
nothing existing fitted.

**Stock Zephyr.** Unmodified Zephyr v3.7.0 `samples/hello_world` and
`samples/synchronization` for `mimxrt595_evk/mimxrt595s/cm33` boot and run
(`examples/mimxrt595-evk/zephyr-hello.yaml`, `zephyr-sync.yaml`). The Zephyr
board runs **secure**, so the chip maps the IDAU secure aliases (FlexSPI
`0x1800_0000`, SRAM `0x3000_0000`, peripherals `0x5000_0000`, boot ROM
`0x1300_0000`) as same-backing mirrors. Zephyr's RT5xx clock bring-up runs
against the CLKCTL0/1 status bits (`rt5xx_clkctl0/1`), the FRO trim read goes
through a clean-room boot-ROM OTP API window (`crates/core/roms/rt500`, every
fuse blank), FlexSPI0 is re-clocked through the `imxrt_flexspi` controller
model, and the OS event timer (`rt5xx_ostimer`) is the system clock:
`k_busy_wait` polls its Gray-coded count and `k_msleep` wakes on its match
interrupt.

!!! warning "Not claimed yet"
    The boot ROM's boot flow is not run and the FCB is not parsed: the engine
    boots from `flash.base + 0x1000` (`reset_vector_offset`); only the OTP
    API entry points exist in the ROM window, and every fuse reads 0. The
    FlexSPI device is a generic JEDEC NOR (no octal-DDR commands) and the
    CACHE64 is not modelled; the NOR window is a flat 8 MiB array (the EVK's
    part is 64 MiB). CLKCTL/RSTCTL/IOPCTL store their words but have no
    effect: clock gates, resets, clock selects and pad functions change
    nothing, every clock reports ready at once, and every word resets to 0
    rather than the silicon reset value. There is no TrustZone: the secure
    aliases are plain mirrors and no SAU/IDAU attribution is checked. The
    USART has no baud timing; GPIO pin interrupts and PINT are stored only.
    SYSCTL0/1, PMC, WWDT0, INPUTMUX, CACHE64, PINT and the non-secure SCS
    bank are declared stubs. FLEXCOMM1–11/13–16, CTIMER, MRT, DMA, SDIO,
    USB, ADC, I3C, the display/GPU, PowerQuad, the OTP controller and the
    HiFi4 DSP are **not mapped**: firmware that touches them faults with a
    memory violation. There is no silicon capture: **SIM-DERIVED**.

!!! tip "Live status"
    The tables below are a maintained snapshot. Authoritative automation:

    - [Chip conformance scoreboard](../coverage/chip-conformance.md) — level · modelled peripherals · register-match

---

## Status at a glance

| Aspect | Status |
|--------|--------|
| Chip descriptor | [`configs/chips/mimxrt595s.yaml`](../../configs/chips/mimxrt595s.yaml) |
| Example system | [`configs/systems/mimxrt595-evk.yaml`](../../configs/systems/mimxrt595-evk.yaml) |
| Reference firmware | [`crates/firmware-mimxrt595-evk-smoke/`](../../crates/firmware-mimxrt595-evk-smoke/) |
| Example | [`examples/mimxrt595-evk/`](../../examples/mimxrt595-evk/) |
| Committed artifact | `tests/fixtures/mimxrt595-evk-smoke.elf` (`MIMXRT595 EVK SMOKE`) |
| Stock Zephyr fixtures | `tests/fixtures/mimxrt595-zephyr-hello.elf`, `mimxrt595-zephyr-sync.elf` (Zephyr v3.7.0; [`crates/firmware-mimxrt595-evk-zephyr/build.sh`](../../crates/firmware-mimxrt595-evk-zephyr/build.sh)) |
| Build target | `thumbv8m.main-none-eabi` |
| Tier (snapshot) | **L1** — smoke (Cortex-M33 + memory + GPIO + FLEXCOMM0 USART) plus stock Zephyr hello_world/synchronization; SIM-DERIVED |

---

## Memory map and board wiring

| Region / signal | Base / pin | State |
|--------|------|-------|
| SRAM code-bus alias | `0x0000_0000` | **not mapped** |
| Boot ROM | `0x0300_0000` (first 64 KiB) | clean-room OTP API window (`roms/rt500`); the rest of the ROM is not modelled |
| Secure aliases | `0x1300_0000` ROM, `0x1800_0000` FlexSPI, `0x3000_0000` SRAM, `0x5000_0000` peripherals | same-backing mirrors of the non-secure address (`memory_regions[].alias_of`) |
| FlexSPI0 NOR (XIP) | `0x0800_0000`, 8 MiB mapped | modelled as flat flash memory — FCB at `0x0800_0400`, vector table at `0x0800_1000` |
| SRAM (data bus) | `0x2000_0000`, 5 MiB | modelled — the smoke's data/stack in `0x2028_0000`–`0x204F_FFFF` |
| RSTCTL0, CLKCTL0 | `0x4000_0000`, `0x4000_1000` | modelled storage (`rt5xx_ctl` / `rt5xx_clkctl0`, SET/CLR aliases; CLKCTL0 also reports the clock-ready/REQFLAG/PFD/FRO-tuner status) |
| SYSCTL0 | `0x4000_2000` | `type: stub` |
| IOPCTL | `0x4000_4000` | modelled storage (`rt5xx_iopctl`) |
| WWDT0 | `0x4000_E000` | `type: stub` |
| RSTCTL1, CLKCTL1 | `0x4002_0000`, `0x4002_1000` | modelled storage (`rt5xx_ctl` / `rt5xx_clkctl1`, with REQFLAG/PFD status) |
| SYSCTL1, PINT, INPUTMUX, CACHE64_CTRL0 | `0x4002_2000`, `0x4002_5000`, `0x4002_6000`, `0x4003_3000` | `type: stub` |
| GPIO (ports 0–6) | `0x4010_0000` | modelled (`lpc_gpio`) |
| FLEXCOMM0 / USART0 | `0x4010_6000`, IRQ 14 | modelled (`lpc_flexcomm_usart`, USART view only); the console |
| OS event timer | `0x4011_3000`, IRQ 41 | modelled (`rt5xx_ostimer`, 1 MHz, Gray-coded count, match IRQ); the Zephyr system clock |
| FLEXCOMM12 / USART | `0x4020_D000`, IRQ 64 | modelled (`lpc_flexcomm_usart`); the Arduino-header serial, not wired |
| FlexSPI0 controller | `0x4013_4000` | modelled (`imxrt_flexspi`, the RT1060 FlexSPI model) |
| PMC | `0x4013_5000` | `type: stub` |
| Non-secure SCS bank | `0xE002_E000` | `type: stub` (`SCB_NS->CPACR` in `SystemInit`) |
| RGB LED | red `P0_14`, green `P1_0`, blue `P3_17` | modelled GPIO — on when high (SDK `LOGIC_LED_ON` = 1) |
| SW1 / SW2 | `P0_25` / `P0_10` | modelled GPIO inputs, board buttons released = 1 (polarity assumed, see VALIDATION) |
| FLEXCOMM0 console | TXD `P0_1`, RXD `P0_2` | the simulator captures the FLEXCOMM0 stream; the LPC-Link2 VCOM bridge is not modelled |

## Boot path

**Fast boot.** On silicon the boot ROM reads the FlexSPI configuration block
(tag `FCFB`) at `0x0800_0400`, sets up the FlexSPI controller and starts the
image through the vector table at `0x0800_1000`. The engine does not run the
boot ROM or parse the FCB: the first words of the window are erased, so the
chip's `reset_vector_offset: 0x1000` makes reset load SP/PC from
`0x0800_1000`, where the SDK linker file and the smoke put the vector table
(reset handler `0x0800_1800`, SP `0x2050_0000`).

## Validation

```bash
# from the repository root
cargo build -p firmware-mimxrt595-evk-smoke --release --target thumbv8m.main-none-eabi
./target/release/labwired test --script examples/mimxrt595-evk/io-smoke.yaml --no-uart-stdout
./target/release/labwired test --script examples/mimxrt595-evk/zephyr-hello.yaml --no-uart-stdout
./target/release/labwired test --script examples/mimxrt595-evk/zephyr-sync.yaml --no-uart-stdout
```

See [`examples/mimxrt595-evk/VALIDATION.md`](../../examples/mimxrt595-evk/VALIDATION.md)
for the claims, runtime evidence and explicit non-claims.

## Sources

The MCUXpresso SDK `MIMXRT595S_cm33.h` device header (peripheral bases,
interrupt numbers, register layouts), the SDK `evkmimxrt595` flash linker
file, `board.h` and `system.c`/`system.h` (memory map, LED/switch/console
pins, FRO clocks), and the Renode v1.17.0 `imxrt500.repl` platform as a
cross-check — detailed in
[`examples/mimxrt595-evk/REQUIRED_DOCS.md`](../../examples/mimxrt595-evk/REQUIRED_DOCS.md).
