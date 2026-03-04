[← Back to Hub](../../README.md)

# Video Runbook: Blinky in VS Code + Parallel Aether Deterministic Check

Goal: record a 90-150s demo where the LabWired VS Code flow is shown together with a hardware/Aether deterministic checkpoint.

## 1) One-command prep

Run from anywhere:

```bash
/home/andrii/Projects/labwired/core/examples/nucleo-h563zi/scripts/run_video_demo.sh
```

This prepares:
- `core/examples/firmware-stm32f103-blinky/.vscode/launch.json`
- simulator artifacts in `out/pitch-video/blinky/sim/`
- summary snapshot in `out/pitch-video/blinky/comparison_snapshot.txt`

If you already built recently:

```bash
/home/andrii/Projects/labwired/core/examples/nucleo-h563zi/scripts/run_video_demo.sh --skip-build
```

## 2) VS Code shot (Blinky)

1. Open folder: `core/examples/firmware-stm32f103-blinky`.
2. Start debug with `LabWired: Demo Blinky`.
3. Show:
- breakpoint hit in firmware
- stepping (`F10`) updates PC/registers
- Command Center / telemetry panel updates

Voiceover:
- "This is the same firmware debug loop embedded teams already know, now running in deterministic simulation."

## 3) Parallel deterministic proof shot

Terminal command:

```bash
/home/andrii/Projects/labwired/core/examples/nucleo-h563zi/scripts/run_video_demo.sh --mode proof
```

Highlight:
- `status: pass`
- assertion list with `passed: true`
- deterministic `stop_reason`

Voiceover:
- "Every run gives explicit machine-readable pass/fail evidence, not just human interpretation."

## 4) Aether side-by-side shot

Collect board trace/log in Aether (your normal command flow), save to a file, then:

```bash
/home/andrii/Projects/labwired/core/examples/nucleo-h563zi/scripts/run_video_demo.sh --aether-log /path/to/aether.log
```

On screen layout:
- Left: VS Code/LabWired simulator debug moment
- Right: Aether board trace checkpoint

Voiceover:
- "Simulation gives fast deterministic iteration; Aether confirms on-board behavior for the same scenario."
- "This is functional parity for this path."

## 5) Recording checklist

1. Terminal font >= 16px.
2. Keep one shared behavior marker visible in both panes.
3. Keep run under 2.5 minutes.
4. Export 1080p, 30fps, H.264.
