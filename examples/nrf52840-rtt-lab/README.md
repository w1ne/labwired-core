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

## SEGGER KB sample (printf + virtual terminals)

`crates/firmware-nrf52840-rtt-demo` is SEGGER's knowledge-base RTT example —
banner, `SEGGER_RTT_printf` counter, `SEGGER_RTT_TerminalOut` overflow beat —
ported to bare-metal Rust against the stock vendor library.

Prerequisites: `rustup target add thumbv7em-none-eabi` and an ARM C
cross-compiler (`gcc-arm-none-eabi`) — the firmware compiles the vendor RTT C
sources.

```bash
cargo build -p firmware-nrf52840-rtt-demo --release --target thumbv7em-none-eabi
cargo run -q -p labwired-cli -- \
  --firmware target/thumbv7em-none-eabi/release/firmware-nrf52840-rtt-demo \
  --system examples/nrf52840-rtt-lab/system.yaml --rtt
cargo run -q -p labwired-cli -- test \
  --script examples/nrf52840-rtt-lab/rtt-printf-smoke.yaml \
  --output-dir out/nrf52840-rtt-lab/rtt-printf-smoke
```

The same ELF runs in the browser as the `nrf52840-rtt-lab` playground lab.
The console is the RTT viewer when the firmware links SEGGER RTT: channel 0
terminals, and a down-channel 0 line that `SEGGER_RTT_GetKey` reads. `q`
prints `quit` and stops the loop. `labwired run --rtt` writes stdin into
that same down buffer.

## Zephyr note

Zephyr's RTT log backend (`CONFIG_LOG_BACKEND_RTT`) defaults to a 1 KiB up-buffer.
The model's poll cadence is bounded in simulated CPU cycles (64 by default), not in
bus ticks, so it keeps draining that buffer even when a long Cortex-M JIT window
spans thousands of instructions; a Zephyr image that logs faster than the probe
drains will block in `BLOCK_IF_FIFO_FULL` mode exactly as it would on hardware.

## Files

1. `system.yaml` - bare nRF52840 twin.
2. `rtt-smoke.yaml` - `rtt_contains` oracle.
3. `rtt-printf-smoke.yaml` - printf demo oracle (banner + counter advance).
4. `rtt-printf-overflow.yaml` - printf demo overflow oracle; needs ~21M steps to
   reach the `Counter overflow!` beat.
5. `README.md` - this file.

The firmware crates are `crates/firmware-nrf52840-rtt` and
`crates/firmware-nrf52840-rtt-demo`; they vendor `SEGGER_RTT.c/.h` and
`SEGGER_RTT_printf.c` unmodified (see `third_party/segger-rtt/README.md`).
