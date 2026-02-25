# Video Runbook: Flaky Pain -> Deterministic Pass

Goal: record one polished 90-120s video showing the problem context and deterministic proof.

## 1) Prep (before recording)

Run from repo root:

```bash
mkdir -p out/pitch-video
```

Recommended one-command prep (works from any current directory):

```bash
/home/andrii/Projects/labwired/scripts/run_pitch_flaky_to_deterministic.sh
```

Fallback prep (reuse existing showcase artifacts, no build/run):

```bash
/home/andrii/Projects/labwired/scripts/run_pitch_flaky_to_deterministic.sh --use-existing
```

Manual live-run prep (if you want to run commands directly):

```bash
cd core
cargo build --release -p labwired-cli
./target/release/labwired test \
  --script examples/ci/uart-ok.yaml \
  --output-dir ../out/pitch-video/uart-ok \
  --no-uart-stdout
cd ..
```

If live run is not ready, use existing evidence files in `docs/showcase-evidence/`.

## 2) Shot-by-shot timeline

### Shot 1 (0:00-0:15) - Pain framing

On screen:
- Open [fixing-flaky-firmware-tests.md](/home/andrii/Projects/labwired/marketing/blog/fixing-flaky-firmware-tests.md)
- Scroll to the problem lines describing flaky reruns/HIL instability.

Voiceover:
- "Firmware CI on physical benches is often flaky. Teams rerun the same pipeline and get different outcomes."

### Shot 2 (0:15-0:30) - Deterministic test command

On screen (terminal):

Live option:
```bash
/home/andrii/Projects/labwired/scripts/run_pitch_flaky_to_deterministic.sh
```

Artifact option (no compute risk):
```bash
cat docs/showcase-evidence/simulation_result.json | jq '{status, stop_reason, assertions}'
```

Voiceover:
- "Now we run LabWired in deterministic CI mode with explicit assertions."

### Shot 3 (0:30-0:50) - Assertion proof

On screen:

Live artifacts:
```bash
/home/andrii/Projects/labwired/scripts/show_pitch_proof.sh
```

Existing evidence artifacts:
```bash
cat docs/showcase-evidence/simulation_result.json | jq '{status, stop_reason, assertions}'
```

Expected key proof to highlight:
- `"status": "pass"`
- assertion object present and `"passed": true`

Voiceover:
- "The run is machine-verifiable: pass/fail, stop reason, and assertion outcomes are explicit in JSON."

### Shot 4 (0:50-1:10) - UART proof

On screen:

Live artifacts:
```bash
strings /home/andrii/Projects/labwired/out/pitch-video/uart-ok/uart.log
```

Existing evidence artifacts (UTF-16-safe display):
```bash
strings docs/showcase-evidence/simulation_uart.log
```

Expected key proof:
- UART success token visible (`OK` in live CI smoke, or `HIL Stress Test Passed` in showcase evidence).

Voiceover:
- "UART output is captured as an artifact, so expected behavior is testable and replayable."

### Shot 5 (1:10-1:25) - Close

On screen:
- Return to pitch slide or README quick section with CI command.

Voiceover:
- "Instead of rerunning flaky hardware jobs, we get deterministic evidence on every PR."
- "LabWired turns firmware validation into a repeatable software workflow."

## 3) Optional extension: Aether board vs simulator parity (30-45s)

Add this as a second clip or as an extension to Shot 5.

### Shot A (side-by-side setup)

On screen:
- Left pane: simulator evidence (`result.json` + UART line).
- Right pane: Aether debugger output from physical board run.

Suggested simulator command:
```bash
cat docs/showcase-evidence/simulation_result.json | jq '{status, stop_reason, assertions}'
strings docs/showcase-evidence/simulation_uart.log
```

Suggested board evidence:
- Show Aether trace/log containing the matching behavior marker for the same firmware scenario.
- Prefer one shared token/checkpoint you can point at in both panes.

Voiceover:
- "Left is deterministic simulator output, right is physical board trace captured with Aether."
- "For this scenario, the behavioral checkpoints match."

### Shot B (explicit parity callout)

On screen:
- Zoom in on the matching marker(s): boot token, UART success string, or key state transition.

Voiceover:
- "This is functional parity for this test path: same observable behavior in simulation and hardware."
- "We use simulation for fast, repeatable CI, then validate final confidence on board."

Guardrail language:
- Say: "functional parity on this scenario."
- Do not say: "perfect parity across all peripherals."

## 4) Recording checklist

1. Terminal font >= 16px, line wrapping disabled.
2. Keep command history clean (`clear` between shots).
3. Zoom into `status`, `assertions`, and UART success line.
4. Keep total runtime under 2 minutes.
5. Export 1080p, 30fps, H.264.

## 5) Safety fallback (if anything fails live)

Use only static evidence and keep narrative intact:

```bash
cat docs/showcase-evidence/simulation_result.json | jq '{status, stop_reason, assertions}'
strings docs/showcase-evidence/simulation_uart.log
```

This still demonstrates deterministic pass + UART/assertion proof without live build risk.
