# Waveshare RP2350-Zero

Compact **RP2350A** USB-C module (Waveshare RP2350-Zero): dual Cortex-M33,
520 KB SRAM, 4 MB flash, castellated 18.00 × 23.50 mm board, onboard WS2812
on **GP16**. Same silicon descriptor as Pico 2; this page is the **Zero
carrier**. USB-C is native USB — there is no UART-bridge chip.

!!! tip "Live status"
    The tables below are a maintained snapshot. Authoritative automation:

    - [Chip conformance scoreboard](../coverage/chip-conformance.md)
    - [Tier-1 matrix](../coverage/tier1-scoreboard.md)
    - [Target support rubric](../target_support_rubric.md)

---

## Status at a glance

| Aspect | Status |
|--------|--------|
| Chip descriptor | [`configs/chips/rp2350.yaml`](../../configs/chips/rp2350.yaml) |
| System YAML | [`configs/systems/rp2350-zero.yaml`](../../configs/systems/rp2350-zero.yaml) |
| Example | [`examples/rp2350-zero/`](../../examples/rp2350-zero/README.md) |
| Playground board id | `rp2350-zero` |
| Reference firmware | `crates/firmware-rp2350-demo/` (UART0 smoke) |
| Tier (snapshot) | smoke-manual (UART0). **No silicon capture.** |

Pico 2 (GP25 LED, different USB connector) is [`rp2350.md`](rp2350.md) /
[`examples/pico2/`](../../examples/pico2/README.md).

---

## Flash / firmware artifact

| Use | Artifact | Notes |
|-----|----------|--------|
| Twin / CLI | **ELF** linked at XIP `0x10000000` | `firmware-rp2350-demo` |
| Bench | **UF2** | Hold BOOT, tap RUN, copy to the UF2 disk |
| Arduino-Pico | `waveshare_rp2350_zero` | Hosted compile id `rp2350-zero`. Browser flash is **not** wired (UF2 on the bench). |

!!! warning "USB-C is not UART0"
    The cable is RP2350 USB CDC. Header UART0 is GP0 (TX) / GP1 (RX). The twin
    sinks UART0 only. USB device enumeration is not modeled.

---

## Twin vs USB cable

| | Twin | USB-C cable |
|---|---|---|
| Console | UART0 PL011 @ `0x40070000` | CDC (desk capture: VID `2e8a` PID `0009`) |
| LED | `board_io` LED on SIO GP16 (WS2812 stand-in) | WS2812 DIN GP16 |
| Load | ELF into XIP | UF2 |

---

## Pins (castellated header, USB-C at top)

Silk from Waveshare wiki pinout + `RP2350_Zero.pdf` P1 (23 pads). Defaults
from pico-sdk `waveshare_rp2350_zero.h`. Authored drawing:
[`examples/rp2350-zero/images/pinout.svg`](../../examples/rp2350-zero/images/pinout.svg).

### Right edge (top → bottom)

| Silk | GPIO | Default (pico-sdk) |
|------|------|--------------------|
| GP0 | GPIO0 | UART0 TX |
| GP1 | GPIO1 | UART0 RX |
| GP2 | GPIO2 | |
| GP3 | GPIO3 | |
| GP4 | GPIO4 | |
| GP5 | GPIO5 | |
| GP6 | GPIO6 | I2C1 SDA |
| GP7 | GPIO7 | I2C1 SCL |

### Bottom edge (left → right)

| Silk | GPIO | Default (pico-sdk) |
|------|------|--------------------|
| GP13 | GPIO13 | SPI1 CSN |
| GP12 | GPIO12 | SPI1 RX |
| GP11 | GPIO11 | SPI1 TX |
| GP10 | GPIO10 | SPI1 SCK |
| GP9 | GPIO9 | |
| GP8 | GPIO8 | |

### Left edge (top → bottom)

| Silk | GPIO | Notes |
|------|------|--------|
| 5V | — | VBUS |
| GND | — | |
| 3V3 | — | ME6217 3.3 V |
| GP29 | GPIO29 | ADC3 |
| GP28 | GPIO28 | ADC2 |
| GP27 | GPIO27 | ADC1 |
| GP26 | GPIO26 | ADC0 |
| GP15 | GPIO15 | |
| GP14 | GPIO14 | |

### Onboard / solder (not the 20-pin header)

| Silk | GPIO | Notes |
|------|------|--------|
| WS2812 DIN | GPIO16 | **Used.** Twin: `led_gp16` stand-in. Not a free header GPIO. |
| GP17–GP25 | GPIO17–25 | Underside solder points (9 GPIOs + GND). Schematic P2/P3. |
| SWCLK / SWDIO | debug | Pads; no onboard probe. |
| BOOT / RUN | — | UF2 entry: BOOT then RUN. |

Always confirm with `labwired_describe` once the playground id exists.

---

## Support matrix

| Mark | Meaning |
|------|---------|
| ✅ | Modeled well enough for the demos we ship |
| ⚠️ | Present but partial, stubbed, or easy to misuse |
| ❌ | Not simulated — use the bench |

### Core & boot

| Block | Status | Notes |
|-------|--------|--------|
| Cortex-M33 core 0 | ✅ | `thumbv8m.main-none-eabi` smoke ELF |
| Core 1 / SIO FIFO | ❌ | Not a dual-core product story |
| Hazard3 RISC-V | ❌ | Unmodeled |
| TrustZone | ❌ | Unmodeled |
| Bootrom / IMAGE_DEF / UF2 | ❌ | Twin loads ELF. Bench uses UF2. |
| `rp2350` clocks/resets | ✅ | Moved APB map vs RP2040 |

### Console, GPIO, LED

| Block | Status | Notes |
|-------|--------|--------|
| UART0 | ✅ | Twin console. Header GP0/GP1 — not the USB cable. |
| USB CDC / USB device | ❌ | Bench only |
| SIO GPIO | ⚠️ | Enough for the GP16 LED stand-in |
| WS2812 on GP16 | ⚠️ | Stand-in LED. PIO/WS2812 timing not claimed |
| ADC GP26–29 | ⚠️ | Partial analog |

### Buses

| Block | Status | Notes |
|-------|--------|--------|
| I2C0/1, SPI0/1 | ⚠️ | RP2040-lane models on RP2350 bases |
| PIO0/1/2 | ⚠️ | v1-compatible only; v2-only ops unmodeled |
| DMA, PWM, TIMER0, WATCHDOG | ⚠️ | See chip page |

Firmware that touches unmodeled MMIO will hit `MemoryAccessViolation` or poll
forever — that is intentional.

---

## What it catches vs what needs a bench

**Twin is strong for:** UART0 bring-up of an RP2350 Thumb ELF; GP16 as a
digital LED stand-in; deterministic CLI oracles that match UART0.

**Needs the bench:** anything on USB CDC, UF2 boot, WS2812 color, SWD
register parity, PIO v2, dual-core.

---

## How to run

### CLI

```bash
RUSTFLAGS="-C link-arg=-Tlink.x" cargo build -p firmware-rp2350-demo \
  --release --target thumbv8m.main-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/rp2350-zero/uart-smoke.yaml \
  --output-dir out/rp2350-zero/uart-smoke \
  --no-uart-stdout
```

Details: [`examples/rp2350-zero/VALIDATION.md`](../../examples/rp2350-zero/VALIDATION.md).

### Playground

Board id `rp2350-zero` on [app.labwired.com](https://app.labwired.com). USB CDC
on the cable is still unmodeled — serial in the twin is UART0.

### Agent

[MCP](../agent/mcp.md) → describe the **chip** `rp2350` and run against
`examples/rp2350-zero/system.yaml`. Do not claim USB or WS2812 success.

---

## Related

- Chip / Pico 2: [RP2350](rp2350.md)
- Example: [`examples/rp2350-zero/`](../../examples/rp2350-zero/README.md)
- [Parts](../parts/index.md) · [Fidelity](../fidelity.md) · [Rubric](../target_support_rubric.md)
