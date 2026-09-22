# SEGGER RTT logging

SEGGER RTT (Real-Time Transfer) is the logging path most nRF52 and Cortex-M
projects use instead of a UART: the firmware links a small vendor library that
writes into a ring buffer in RAM, and a debug probe drains that buffer over SWD
while the CPU keeps running.

LabWired models the **probe side**. Firmware that links the stock
`SEGGER_RTT.c` runs unmodified, and the simulator drains the RAM control block
through the emulated bus. No UART wiring and no source changes are needed.

---

## Quick start

The example lab is `examples/nrf52840-rtt-lab/`.

Build the demo firmware:

```bash
rustup target add thumbv7em-none-eabi   # once
cargo build -p firmware-nrf52840-rtt --release --target thumbv7em-none-eabi
```

Stream RTT to the terminal:

```bash
cargo run -q -p labwired-cli -- \
  --firmware target/thumbv7em-none-eabi/release/firmware-nrf52840-rtt \
  --system examples/nrf52840-rtt-lab/system.yaml --rtt
```

Expected: `RTT hello from labwired` printed as the firmware writes it.

Gate it in CI with a test script:

```yaml
schema_version: "1.0"
inputs:
  firmware: "../../target/thumbv7em-none-eabi/release/firmware-nrf52840-rtt"
  system: "./system.yaml"
limits:
  max_steps: 200000
assertions:
  - rtt_contains: "RTT hello from labwired"
  - expected_stop_reason: max_steps
```

```bash
cargo run -q -p labwired-cli -- test \
  --script examples/nrf52840-rtt-lab/rtt-smoke.yaml \
  --output-dir out/nrf52840-rtt-lab/rtt-smoke
```

Expected: `PASS`, and `out/nrf52840-rtt-lab/rtt-smoke/rtt.log` contains the
banner. `rtt_contains` alone enables the RTT model; `--rtt` is only needed for
runs without an RTT assertion.

---

## How it maps to real RTT

The stock library keeps a control block in RAM:

| Offset | Field |
|--------|-------|
| `0x00` | `acIdentifier[16]`: `"SEGGER RTT"` + NULs |
| `0x10` | `MaxNumUpBuffers` |
| `0x18` | `aUp[0]`, then one 24-byte descriptor per channel |

Each up-channel descriptor points at a ring buffer with `WrOff` (firmware
write cursor) and `RdOff` (probe read cursor). LabWired's model:

1. Finds the control block from the ELF's `_SEGGER_RTT` symbol, or falls back
   to a bounded, cursor-resumable RAM search for the 16-byte ID.
2. Reads `RdOff..WrOff` with ring wrap and appends the bytes to a dedicated
   sink — never the UART buffer.
3. Writes `RdOff` back to `WrOff`, so blocking `BLOCK_IF_FIFO_FULL` writes are
   released exactly as a real probe releases them.
4. Polls on a cadence bounded in simulated CPU cycles (64 by default), so a
   long Cortex-M JIT window cannot stall a blocking writer.

Channels with non-zero upper `Flags` bytes (block-skip mode), broken geometry
or pointers outside RAM are skipped rather than misread. Zero-initialized
control blocks are normal and recover after `SEGGER_RTT_Init()`.

---

## CLI

| Flag | Effect |
|------|--------|
| `--rtt` | Global. `run`/interactive: echo drained RTT bytes to stdout. `test`: enable RTT capture and write `rtt.log`. |

Under `--json`, interactive RTT echo is suppressed (with a note on stderr)
rather than splicing raw bytes into the JSON document on stdout.

---

## Test pipeline

Add an assertion to a single-machine test script:

```yaml
assertions:
  - rtt_contains: "ready"
```

* The assertion reads only the dedicated RTT stream, so an RTT token cannot
  match a UART banner and vice versa.
* `rtt.log` is written next to `uart.log` for every `--output-dir` run (empty
  when RTT was not enabled).
* `result.json` gains an `rtt` block when RTT was enabled:

```json
"rtt": { "control_block_found": true, "bytes_drained": 24 }
```

* Environment/world scripts reject `rtt_contains` at validation: they cannot
  observe RTT yet, so the assertion fails rather than passing by silence.

### Zephyr

`CONFIG_LOG_BACKEND_RTT` defaults to a 1 KiB up-buffer. The 64-cycle drain
cadence keeps that buffer moving regardless of JIT window length, so a Zephyr
image that logs faster than the probe drains blocks exactly as it would on
hardware.

---

## Limitations

* Down-channel 0 is the host-to-target path (`SEGGER_RTT_GetKey` / `SEGGER_RTT_Read`). Other down channels are not written.
* 32-bit control blocks; 64-bit targets are out of scope.
* ARM paths only. Xtensa and ELF-less ROM-boot runs do not attach the model, so
  `rtt_contains` fails closed there.
* No channel names, terminal control or J-Link tooling: the stream is bytes.

Coverage lives in `crates/core/tests/e2e_segger_rtt.rs` (plain e2e plus a
blocking-mode test under the Cortex-M JIT), `crates/cli/tests/e2e_segger_rtt.rs`
(four CLI scenarios) and the unit tests in
`crates/core/src/peripherals/segger_rtt.rs`.
