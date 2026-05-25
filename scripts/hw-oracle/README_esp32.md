# ESP32-WROOM-32 HW-oracle (chip-model verification)

Operator runbook for the ESP32-WROOM-32 capture-and-replay harness that
backs the full ESP32 chip-model rebuild (roadmap: issue #105).

The harness has two halves:

1. **`esp32_capture.sh`** — drives a real ESP32 via OpenOCD, flashes a
   firmware ELF, and dumps a baseline trace (PC samples + memory
   checkpoints) into a timestamped capture directory.
2. **`esp32_replay_in_sim`** (Rust binary in `crates/hw-oracle`) — loads
   the SAME firmware into our simulator, runs it for a comparable cycle
   budget, re-reads the same checkpoints, and emits a JSON diff with the
   first divergence point.

This pair is the verification gate for every new peripheral added to the
ESP32-classic chip model: if the sim's trace converges to HW for a given
firmware, we have evidence the peripheral matters in that firmware
behave correctly. If it diverges, the diff tells us at which PC / which
checkpoint to look.

For the existing decoder-vs-`objdump` oracle scripts (`b7-sweep.sh`,
`b8-sweep.sh`), see [`README.md`](README.md).

## Hardware

| Component             | Notes                                                |
| --------------------- | ---------------------------------------------------- |
| ESP32-WROOM-32 module | DevKitC, DOIT v1, NodeMCU-32S, or any breakout that  |
|                       | exposes 5 V, GND, EN, IO0, IO12 (TDI), IO13 (TCK),   |
|                       | IO14 (TMS), IO15 (TDO).                              |
| ESP-Prog JTAG adapter | FT2232H-based; Espressif sells one but any FT2232H   |
|                       | board works (e.g. Tigard). USB → 6-pin JTAG header.  |
| USB-UART bridge       | Either the dev kit's onboard CP2102 or a CH340 USB- |
|                       | UART module on UART0 (IO1 / IO3). Used by `tio` for  |
|                       | the optional UART stdout capture.                    |
| 5 V USB power         | Both the ESP-Prog and the dev kit need power. With   |
|                       | the DevKitC's onboard regulator you can power the    |
|                       | board from its USB jack while JTAG is on ESP-Prog.   |

### Cabling

JTAG header (ESP-Prog 2x5 1.27 mm IDC → ESP32 pin):

| ESP-Prog | ESP32 GPIO | Function |
| -------: | ---------: | -------- |
|   VTREF  | 3.3 V      | level ref |
|   GND    | GND        |          |
|   TMS    | IO14       |          |
|   TCK    | IO13       |          |
|   TDO    | IO15       |          |
|   TDI    | IO12       |          |
|   RESET  | EN         | optional |

Wire EN to RESET so OpenOCD's `reset halt` actually toggles RESET. Without
it you can still attach but `reset` is a no-op.

> The 2.9" Waveshare tri-color e-paper module on the same board does NOT
> need disconnecting for capture — the firmware still drives it normally
> while OpenOCD samples PC.

## Software

| Tool        | Why                              | Install                                       |
| ----------- | -------------------------------- | --------------------------------------------- |
| openocd     | JTAG bridge                      | Espressif fork: <https://github.com/espressif/openocd-esp32/releases> (distro `openocd` may lack `target/esp32.cfg`) |
| python3     | mem-snapshot post-processing     | usually pre-installed                         |
| tio         | UART stdout capture (optional)   | `sudo apt install tio`                        |
| Rust 1.80+  | running the sim replay binary    | already required by the rest of LabWired core |

Verify OpenOCD has the ESP32 config:

```sh
openocd -f interface/ftdi/esp32_devkitj_v1.cfg -f target/esp32.cfg -c "init; reset halt; shutdown"
```

You should see `Info : esp32.cpu0: Target halted, PC: 0x40000400`. If
instead you get "could not find target/esp32.cfg", install the Espressif
fork.

## Running a capture

```sh
# Flash the firmware to HW, sample PC 256 times every 20 ms, dump
# checkpoint memory before and after. Writes to
# scripts/hw-oracle/captures/esp32-wroom/<utc-ts>/.
./scripts/hw-oracle/esp32_capture.sh path/to/firmware.elf

# Optional: tune the sample plan via env vars
ESP32_SAMPLES=512 ESP32_SAMPLE_MS=10 \
  ./scripts/hw-oracle/esp32_capture.sh path/to/firmware.elf
```

Expected output:

```
[esp32_capture] capture dir: scripts/hw-oracle/captures/esp32-wroom/20260525T141855Z
[esp32_capture] probing for ESP32 via openocd (interface/ftdi/esp32_devkitj_v1.cfg + target/esp32.cfg)...
[esp32_capture] flashing /abs/path/firmware.elf...
[esp32_capture] capturing pre-run memory snapshot...
[esp32_capture] sampling PC: 256 samples every 20 ms...
[esp32_capture] PC trace: 256 samples in 5180 ms
[esp32_capture] capturing post-run memory snapshot...
[esp32_capture] done: scripts/hw-oracle/captures/esp32-wroom/20260525T141855Z
[esp32_capture] next: cargo run --release -p labwired-hw-oracle --bin esp32_replay_in_sim -- \
                        --capture .../20260525T141855Z --elf /abs/path/firmware.elf
```

### Graceful degradation when no hardware is connected

The script returns:

| Exit | Meaning                                            |
| ---: | -------------------------------------------------- |
|    0 | Capture complete (`oracle.json` `status="ok"`)     |
|    2 | OpenOCD not on PATH                                |
|    3 | No ESP32 detected (writes `status="no_hardware"` ) |
|    4 | Bad args (missing/unreadable ELF)                  |

Exit codes 2 and 3 are the headless-CI path — the script never hangs.

### Optional: capture UART stdout in parallel

The current sim has no UART0 model for ESP32-classic, so this only feeds
the **HW** side. Useful for diagnosing where firmware boot diverges:

```sh
tio --log --log-file uart0.txt /dev/ttyUSB0 -b 115200
```

Run `tio` BEFORE invoking `esp32_capture.sh`. After capture, move
`uart0.txt` into the capture dir for future diffing once we model UART0.

## Replaying in the simulator

```sh
cargo run --release -p labwired-hw-oracle --bin esp32_replay_in_sim -- \
    --capture scripts/hw-oracle/captures/esp32-wroom/<ts> \
    --elf path/to/firmware.elf      # optional; defaults to oracle.json's elf
```

Output (stdout AND written to `<capture>/diff.json`):

```json
{
  "schema": "labwired-hw-oracle/esp32-wroom/diff/v1",
  "capture_dir": "scripts/hw-oracle/captures/esp32-wroom/20260525T141855Z",
  "elf": "/abs/path/firmware.elf",
  "pc_samples": 256,
  "pc_first_diverge": { "step": 17, "hw_pc": "0x400d0204", "sim_pc": "0x400d0210" },
  "mem_pre_match": true,
  "mem_post_mismatches": [
    { "addr": "0x3ff44004", "hw": "0x00000020", "sim": "0x00000000" }
  ],
  "summary": "diverged"
}
```

### Tuning the cycle budget

The replay assumes 240 MHz CPU and converts `pc_sample_interval_ms`
(from the manifest) to a sim cycle count. Override:

```sh
cargo run --release -p labwired-hw-oracle --bin esp32_replay_in_sim -- \
    --capture <dir> \
    --cycles-per-sample 50000 \
    --max-steps 500_000_000
```

The defaults are intentionally generous (4× the nominal budget for
`max-steps`) so the sim has slack to overshoot HW without bailing.

## Interpreting the diff

| Field                  | What divergence here means                            |
| ---------------------- | ----------------------------------------------------- |
| `pc_first_diverge`     | First sample where sim PC ≠ HW PC. Look at the HW PC |
|                        | in objdump; if it's inside an unmodeled peripheral    |
|                        | thunk (e.g. SPI), that's your next peripheral to add. |
| `mem_pre_match: false` | Sim didn't observe the same boot state HW saw. Likely |
|                        | a missing flash XIP segment or BROM init.             |
| `mem_post_mismatches`  | Specific MMIO/RAM addresses where sim and HW ended up |
|                        | with different values. The first 1–2 entries usually  |
|                        | point at the responsible peripheral.                  |
| `summary: "sim_only"`  | Capture exited with `status=no_hardware`; only the    |
|                        | sim half ran. Useful for replay-pipeline sanity.      |
| `summary: "ok"`        | All sampled signals matched. Promote this firmware to |
|                        | a CI baseline.                                        |

## Limitations (today)

- **SPI / I²C bus snoop**: not captured. Requires a Saleae or similar logic
  analyzer. Punt until we have one wired into CI.
- **PC sampling intervals are nominal**: OpenOCD halt/resume round-trips
  take ≈1 ms each, so a 20 ms-nominal sample is actually 21–25 ms of
  wall-clock. Good enough for "did sim and HW diverge?", not good enough
  for cycle-accurate timing comparisons.
- **No interrupt capture**: BROM IRQs that fire between samples are
  invisible to this harness. Add interrupt-vector breakpoint hooks once
  we need them.
- **UART0 stdout** is captured separately (via `tio`) on the HW side only;
  the sim has no UART0 model for ESP32-classic yet.

## File layout reference

```
scripts/hw-oracle/
├── README.md              ← objdump-based decoder oracle (unchanged)
├── README_esp32.md        ← this file
├── b7-sweep.sh            ← branch-decoder oracle (unchanged)
├── b8-sweep.sh            ← call/return-decoder oracle (unchanged)
├── esp32_capture.sh       ← NEW: ESP32-WROOM-32 HW capture
└── captures/
    └── esp32-wroom/
        └── <utc-ts>/
            ├── oracle.json
            ├── pc_trace.tsv
            ├── mem_pre.json
            ├── mem_post.json
            ├── openocd.log
            ├── elf.path
            └── diff.json          (written by the replay tool)

crates/hw-oracle/src/bin/
└── esp32_replay_in_sim.rs ← NEW: sim-side replay + diff
```
