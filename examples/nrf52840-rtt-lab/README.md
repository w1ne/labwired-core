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

## Zephyr note

Zephyr's RTT log backend (`CONFIG_LOG_BACKEND_RTT`) defaults to a 1 KiB up-buffer.
The model's poll cadence is bounded in simulated CPU cycles (64 by default), not in
bus ticks, so it keeps draining that buffer even when a long Cortex-M JIT window
spans thousands of instructions; a Zephyr image that logs faster than the probe
drains will block in `BLOCK_IF_FIFO_FULL` mode exactly as it would on hardware.

## Files

1. `system.yaml` - bare nRF52840 twin.
2. `rtt-smoke.yaml` - `rtt_contains` oracle.
3. `README.md` - this file.

The firmware crate is `crates/firmware-nrf52840-rtt`; it vendors
`SEGGER_RTT.c/.h` unmodified (see `third_party/segger-rtt/README.md`).
