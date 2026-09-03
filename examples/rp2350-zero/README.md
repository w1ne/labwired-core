# Waveshare RP2350-Zero

Run all commands from `core/`.

## Purpose

Deterministic bring-up for the **Waveshare RP2350-Zero** (RP2350A, USB-C,
castellated, 4 MB flash, WS2812 on **GP16**). Same chip descriptor as Pico 2;
different carrier. Twin console is UART0. The USB-C cable is native USB CDC
on the bench and is **not** modeled.

## Twin vs this USB cable

| | Twin | USB-C cable |
|---|---|---|
| Console | UART0 DR (`0x40070000`) | CDC (this desk: `/dev/cu.usbmodem11201`) |
| LED | SIO `board_io` on GP16 (WS2812 stand-in) | WS2812 DIN GP16 |
| Load | ELF at XIP `0x10000000` | UF2: hold BOOT, tap RUN, copy `.uf2` |

## Quick run (twin)

```bash
rustup target add thumbv8m.main-none-eabi   # once
RUSTFLAGS="-C link-arg=-Tlink.x" cargo build -p firmware-rp2350-demo \
  --release --target thumbv8m.main-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/rp2350-zero/uart-smoke.yaml \
  --output-dir out/rp2350-zero/uart-smoke \
  --no-uart-stdout
```

Expected: exit 0, UART contains `RP2350_SMOKE_OK`.

## Bench (UF2)

1. Hold **BOOT**, tap **RUN**, release BOOT. A UF2 disk appears.
2. Copy a `.uf2` built for `waveshare_rp2350_zero` / RP2350A.
3. Serial is USB CDC, not GP0/GP1 unless you wire a UART adapter.

## Files

1. `system.yaml` — GP16 LED stand-in via SIO.
2. `uart-smoke.yaml` — UART oracle (not CDC).
3. `REQUIRED_DOCS.md` — schematic, pico-sdk header, live USB id.
4. `EXTERNAL_COMPONENTS.md` — onboard WS2812 / no extra parts.
5. `VALIDATION.md` — smoke + audit + honesty notes.
6. `images/` — authored pinout and board illustration (schematic/wiki silk). Desk photographs are slice 2.

Pinout: [`images/pinout.svg`](images/pinout.svg)
