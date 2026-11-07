# Waveshare RP2350-Zero

Run commands from the repository root.

**Demand rank:** item 2 in the demand-ordered list is RP2040/RP2350. The 2025
survey reports RP2040 at rank 5 (12%); it does not give RP2350 a separate
percentage. Waveshare's RP2350-Zero gives users a low-cost RP2350A board with
an onboard WS2812 on GP16. Current vendor status is recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Purpose

Deterministic bring-up for the **Waveshare RP2350-Zero** (RP2350A, USB-C,
castellated, 4 MB flash, WS2812 on **GP16**). Twin console is UART0. The board
profile uses a narrow RP2350 descriptor with UART0, IO_BANK0 and SIO models;
its onboard LED uses the WS2812 pixel decoder on the actual GPIO waveform. The
USB-C cable is native USB CDC on the bench and is **not** modeled.

## Twin vs this USB cable

| | Twin | USB-C cable |
|---|---|---|
| Console | UART0 DR (`0x40070000`) | CDC (this desk: `/dev/cu.usbmodem11201`) |
| LED | WS2812 decoder on SIO GP16 | WS2812 DIN GP16 |
| Load | ELF at XIP `0x10000000` | UF2: hold BOOT, tap RUN, copy `.uf2` |

## Quick run (twin)

```bash
scripts/onboarding_smoke.sh \
  --target-id rp2350-zero \
  --crate firmware-rp2350-zero-ws2812-smoke \
  --target thumbv8m.main-none-eabi \
  --script examples/rp2350-zero/uart-smoke.yaml \
  --system configs/systems/rp2350-zero.yaml \
  --out-dir examples/rp2350-zero/evidence \
  --firmware-build-dir crates/firmware-rp2350-zero-ws2812-smoke
```

Expected: exit 0, UART contains `RP2350_SMOKE_OK`, GPIO16 has at least 48
edges, and the attached pixel displays red.

## Copy-paste Arduino example

Install the Adafruit NeoPixel library, select **Waveshare RP2350-Zero** in the
Earle Philhower Arduino-Pico core, and upload:

```cpp
#include <Adafruit_NeoPixel.h>

Adafruit_NeoPixel led(1, 16, NEO_GRB + NEO_KHZ800);

void setup() {
  led.begin();
  led.setPixelColor(0, led.Color(255, 0, 0));
  led.show();
}

void loop() {}
```

The simulator smoke uses bare-metal Rust direct GPIO writes to generate the
same red pixel frame. It verifies the modeled waveform path; it does not
compile or verify the Arduino core or its PIO driver.

## Bench (UF2)

1. Hold **BOOT**, tap **RUN**, release BOOT. A UF2 disk appears.
2. Copy a `.uf2` built for `waveshare_rp2350_zero` / RP2350A.
3. Serial is USB CDC, not GP0/GP1 unless you wire a UART adapter.

## Files

1. `system.yaml` — narrow RP2350 profile and WS2812 device on GP16.
2. `uart-smoke.yaml` — UART, GPIO-edge, and decoded-pixel oracles (not CDC).
3. `REQUIRED_DOCS.md` — schematic, pico-sdk header, USB details.
4. `EXTERNAL_COMPONENTS.md` — onboard WS2812 / no extra parts.
5. `VALIDATION.md` — smoke + audit + honesty notes.
6. `images/` — authored pinout and board illustration (schematic/wiki silk). Desk photographs are slice 2.

Pinout: [`images/pinout.svg`](images/pinout.svg)
