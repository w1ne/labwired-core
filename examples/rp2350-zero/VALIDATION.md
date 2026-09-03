# Waveshare RP2350-Zero Validation Runbook

Run all commands from `core/`.

## 0) Firmware rationale

The smoke firmware is `firmware-rp2350-demo`: bare-metal Cortex-M33
(`thumbv8m.main-none-eabi` + `cortex-m-rt`), linked at XIP flash
`0x10000000` with RAM at `0x20000000`. It writes `RP2350_SMOKE_OK\n` straight
to UART0 DR (`0x40070000` — the RP2350 PL011 base, not the RP2040's
`0x40034000`).

**What this proves:** the RP2350 descriptor boots a Thumb ELF against the
Zero system YAML (WS2812/LED stand-in on **GP16**), the vector table at
flash base is accepted, and UART0 on the RP2350 address map sinks bytes.

**What it cannot prove:** USB CDC on the Type-C port (unmodeled), WS2812
bit timing, dual-core/SIO FIFO, TrustZone, real bootrom/IMAGE_DEF, PIO v2,
HSTX, or silicon register parity. This chip is smoke-manual until an SWD
capture exists.

Factory firmware on the desk CDC (`GPIO connet to GND`, ADC GP28/29) only
proves the USB port is alive. It is **not** a twin oracle.

**Build note:** cortex-m-rt firmware must be linked with `-Tlink.x`
(`RUSTFLAGS="-C link-arg=-Tlink.x"` or the crate's `.cargo/config.toml`).

## 1) Build smoke firmware

```bash
rustup target add thumbv8m.main-none-eabi   # once
RUSTFLAGS="-C link-arg=-Tlink.x" cargo build -p firmware-rp2350-demo \
  --release --target thumbv8m.main-none-eabi
```

## 2) Run deterministic UART smoke

```bash
cargo run -q -p labwired-cli -- test \
  --script examples/rp2350-zero/uart-smoke.yaml \
  --output-dir out/rp2350-zero/uart-smoke \
  --no-uart-stdout
```

Pass criteria: exit `0`, UART contains `RP2350_SMOKE_OK`.

## 3) Run unsupported-instruction audit

```bash
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv8m.main-none-eabi/release/firmware-rp2350-demo \
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

- 2026-09-03: UART smoke exit 0 against `examples/rp2350-zero/uart-smoke.yaml`
  (`RP2350_SMOKE_OK` in `out/rp2350-zero/uart-smoke/uart.log`). Unsupported-
  instruction audit `unsupported_total: 0`
  (`out/unsupported-audit/rp2350-zero/metrics.json`). Firmware is
  `firmware-rp2350-demo` reused from Pico 2; system YAML LED stand-in is
  GP16. USB CDC observed on the desk board (VID `2e8a` PID `0009`); not
  claimed as sim evidence.
