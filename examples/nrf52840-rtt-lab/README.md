# nRF52840 SEGGER RTT lab

Run all commands from `core/`.

## Purpose

Firmware that links the stock `SEGGER_RTT.c` prints through a RAM control block
instead of a UART. A J-Link drains that block over SWD while the CPU runs;
LabWired drains it in-simulation and exposes a dedicated RTT stream.

## Quick run (twin)

```bash
rustup target add thumbv7em-none-eabi   # once
cargo build -p firmware-nrf52840-rtt --release --target thumbv7em-none-eabi
cargo run -q -p labwired-cli -- \
  --firmware target/thumbv7em-none-eabi/release/firmware-nrf52840-rtt \
  --system examples/nrf52840-rtt-lab/system.yaml --rtt
```

Expected: the terminal prints `RTT hello from labwired` repeatedly.

## CI-style assertion run

```bash
cargo run -q -p labwired-cli -- test \
  --script examples/nrf52840-rtt-lab/rtt-smoke.yaml \
  --output-dir out/nrf52840-rtt-lab/rtt-smoke
```

Expected: exit 0, `out/nrf52840-rtt-lab/rtt-smoke/rtt.log` contains the banner.

## Files

1. `system.yaml` - bare nRF52840 twin.
2. `rtt-smoke.yaml` - `rtt_contains` oracle.
3. `README.md` - this file.

The firmware crate is `crates/firmware-nrf52840-rtt`; it vendors
`SEGGER_RTT.c/.h` unmodified (see `third_party/segger-rtt/README.md`).
