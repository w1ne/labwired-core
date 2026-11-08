# Waveshare RP2350-Zero Validation Runbook

Run commands from the repository root.

## 0) Firmware rationale

The smoke firmware is `firmware-rp2350-zero-ws2812-smoke`: bare-metal Cortex-M33
(`thumbv8m.main-none-eabi` + `cortex-m-rt`), linked at XIP flash
`0x10000000` with RAM at `0x20000000`. It writes `RP2350_SMOKE_OK\n` straight
to UART0 DR (`0x40070000` — the RP2350 PL011 base, not the RP2040's
`0x40034000`) and bit-bangs a red GRB frame to the board's WS2812 on GP16.

**What this proves:** the narrow RP2350-Zero profile boots a Thumb ELF, the
firmware drives GPIO16 through IO_BANK0/SIO, the WS2812 model decodes the red
pixel after the latch gap, and UART0 sinks the marker. The smoke gates on UART,
at least 48 GPIO16 edges, and one red pixel.

**What it cannot prove:** USB CDC on the Type-C port (unmodeled), dual-core/SIO
FIFO, TrustZone, real bootrom/IMAGE_DEF, PIO v2, HSTX, or silicon register
parity. The bit-banged smoke does not validate the Arduino-Pico PIO driver.
This chip is smoke-manual until an SWD capture exists.

Factory firmware on the desk CDC (`GPIO connet to GND`, ADC GP28/29) only
proves the USB port is alive. It is **not** a twin oracle.

**Build note:** cortex-m-rt firmware must be linked with `-Tlink.x`
(`RUSTFLAGS="-C link-arg=-Tlink.x"` or the crate's `.cargo/config.toml`).

## 1) Build smoke firmware

```bash
rustup target add thumbv8m.main-none-eabi   # once
RUSTFLAGS="-C link-arg=-Tlink.x" cargo build -p firmware-rp2350-zero-ws2812-smoke \
  --release --target thumbv8m.main-none-eabi
```

## 2) Run deterministic UART + WS2812 smoke

```bash
cargo run -q -p labwired-cli -- test \
  --script examples/rp2350-zero/uart-smoke.yaml \
  --output-dir out/rp2350-zero/uart-smoke \
  --no-uart-stdout
```

Pass criteria: exit `0`, UART contains `RP2350_SMOKE_OK`, SIO GP16 has at least
48 edges, and the attached `rgb_led` artifact contains one red pixel.

## 3) Run unsupported-instruction audit

```bash
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv8m.main-none-eabi/release/firmware-rp2350-zero-ws2812-smoke \
  --system examples/rp2350-zero/system.yaml \
  --max-steps 200000 \
  --out-dir out/unsupported-audit/rp2350-zero
```

Pass criteria: exit `0`, report exists, `unsupported_total: 0`.

## Bench (USB, not SWD)

This board has no onboard debugger. Flash is UF2 (hold BOOT, tap RUN/RESET,
copy `.uf2`). Console on the cable is USB CDC (this desk: VID `2e8a` PID
`0009`, `/dev/cu.usbmodem11201`). Header UART0 GP0/GP1 is a different
connector.

SWD pads (SWCLK/SWDIO) exist for a later silicon capture. Not used in this
runbook.

## Validation record

- The committed `evidence/` directory contains the current local smoke logs,
  structured output, and vendor-status receipt. No physical RP2350-Zero
  capture is included.
