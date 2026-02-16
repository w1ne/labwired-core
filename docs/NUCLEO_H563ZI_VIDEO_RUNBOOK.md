# NUCLEO-H563ZI Video Runbook

This runbook is optimized for recording a clean 4-8 minute demo.

## 1) Pre-Recording Setup

1. Connect NUCLEO-H563ZI over USB (ST-Link + VCP).
2. Open repository root in VS Code.
3. Open terminal at `core/`.
4. Keep these panes visible:
   - terminal output
   - `core/examples/nucleo-h563zi/system.yaml`
   - LabWired Command Center

## 2) Fast Single-Command Recording Path

From `core/`:

```bash
examples/nucleo-h563zi/scripts/run_video_demo.sh --mode all --port /dev/ttyACM0 --keep-artifacts
```

Optional verbosity override:
```bash
DEMO_LOG_LEVEL=info examples/nucleo-h563zi/scripts/run_video_demo.sh --mode emulator --keep-artifacts
```

If recording emulator only:

```bash
examples/nucleo-h563zi/scripts/run_video_demo.sh --mode emulator --keep-artifacts
```

## 3) On-Camera Story Beats

1. Show `system.yaml` `board_io` entries (`PB0`, `PF4`, `PG4`, `PC13`).
2. Run video script and narrate that it validates emulator first, then hardware.
3. Point to emulator proof lines:
   - `PASS: uart-smoke`
   - `PASS: io-smoke`
   - `PASS: fullchip-smoke`
   - `H563-IO`, `PB0=1 PF4=1 PG4=1`, `ALL=1`
4. During hardware phase, show physical board LEDs blinking.
5. Point to UART evidence:
   - `H563-BLINK-UART`
   - `BLINK ... PB0=1 ... PF4=1 ... PG4=1 ...`
   - `BLINK ... PB0=0 ... PF4=0 ... PG4=0 ...`
6. In VS Code Command Center, show live board IO state updates from telemetry.

## 4) Backup Commands (If Needed Live)

From `core/`:

```bash
examples/nucleo-h563zi/scripts/run_full_example.sh
examples/nucleo-h563zi/scripts/run_blink_uart_hardware.sh --port /dev/ttyACM0
```

## 5) Common Failure Recovery

1. Serial not found:
   - pass explicit `--port /dev/ttyACM0`
2. Flash failure:
   - reconnect board and rerun hardware script
3. No UART output:
   - increase capture window, e.g. `--timeout 12`
