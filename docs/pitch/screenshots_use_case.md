# LabWired Pitch Screenshots + Use Case Storyboard

## 1) Ready Screenshot Assets

Generated assets:
- `docs/pitch/assets/landing-home.png`
- `docs/pitch/assets/comparison-qemu.png`
- `docs/pitch/assets/comparison-renode.png`

Capture commands used:
```bash
mkdir -p docs/pitch/assets
cd landing_page
npx playwright screenshot --device='Desktop Chrome' \
  file:///home/andrii/Projects/labwired/landing_page/index.html \
  /home/andrii/Projects/labwired/docs/pitch/assets/landing-home.png
npx playwright screenshot --device='Desktop Chrome' \
  file:///home/andrii/Projects/labwired/landing_page/comparisons/qemu.html \
  /home/andrii/Projects/labwired/docs/pitch/assets/comparison-qemu.png
npx playwright screenshot --device='Desktop Chrome' \
  file:///home/andrii/Projects/labwired/landing_page/comparisons/renode.html \
  /home/andrii/Projects/labwired/docs/pitch/assets/comparison-renode.png
```

## 2) Suggested Slide Placement

1. `landing-home.png` on title/vision slide.
2. `comparison-qemu.png` and `comparison-renode.png` on competitive context slide.
3. Terminal/runtime evidence (text snippets from `docs/showcase-evidence/`) on proof slide.

## 3) Use Case (Pitch-Friendly)

### Persona
Firmware lead at a 12-person embedded team shipping STM32-based industrial controllers.

### Problem
- CI is blocked by limited hardware benches.
- Regression runs are flaky due to timing variance and bench contention.
- Reproducing one intermittent bug can take days.

### LabWired Flow
1. Team defines board/chip manifests and adds smoke assertions.
2. CI runs deterministic simulations on every PR.
3. Failing runs emit repeatable logs/artifacts for direct debugging.
4. Hardware bench is reserved for final confidence gates, not routine regression.

### Outcome (positioning language)
- Faster firmware feedback loops.
- Fewer unreproducible failures.
- Better engineering throughput with less dependence on physical bench availability.

## 4) 60-Second Demo Script

1. Show landing screenshot and one-sentence value proposition.
2. Show a deterministic run artifact with pass/fail assertion.
3. Show comparison screenshot to anchor market context.
4. Close with pilot offer: "Bring one flaky test, we stabilize it in your CI in 2 weeks." 

## 5) Video Runbook

- Full shot-by-shot script: `docs/pitch/video_flaky_to_deterministic_runbook.md`
- Blinky + VS Code + Aether flow: `docs/pitch/video_blinky_vscode_aether_runbook.md`
