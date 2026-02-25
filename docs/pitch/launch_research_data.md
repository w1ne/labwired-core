# LabWired Launch Research Data (Cited)

Purpose: source-backed data pack for pitch claims

## 1) External Reality Check: Competitive Density

### Open-source baseline incumbents
- QEMU is a mature, general machine emulator and virtualizer with broad architecture scope.
  Source: https://www.qemu.org/
- Renode positions itself as a development framework for multi-node embedded systems and supports automatic testing.
  Source: https://github.com/renode/renode
- Wokwi provides embedded simulation with CI integration guidance, signaling strong developer-experience competition.
  Source: https://docs.wokwi.com/ci/

### Enterprise virtual platform incumbents
- Intel Simics is positioned for software development ahead of hardware availability.
  Source: https://www.intel.com/content/www/us/en/developer/tools/oneapi/simics-simulator.html
- Synopsys Virtualizer markets virtual prototypes for early software bring-up and shift-left workflows.
  Source: https://www.synopsys.com/verification/virtual-prototyping/virtualizer.html

## 2) Adoption Proxies (GitHub snapshot)

Repository API snapshots:
- `qemu/qemu`: 12,739 stars, 6,542 forks.
  Source: https://api.github.com/repos/qemu/qemu
- `renode/renode`: 2,277 stars, 407 forks.
  Source: https://api.github.com/repos/renode/renode

Interpretation:
- The space already contains mature projects with significant community gravity.
- A new entrant must win on focused wedge + execution speed, not breadth claims.

## 3) Market Size Signal (Directional)

- Public market research outlets project continued growth in digital twin markets through 2030+.
  Source example: https://www.grandviewresearch.com/industry-analysis/digital-twin-market-report

Interpretation:
- Even allowing for variance between analysts, directionality is clear: budget attention is increasing for simulation/twin workflows.
- LabWired should frame itself as the embedded firmware CI/determinism layer, not as a broad enterprise asset-graph platform.

## 4) Internal Evidence From This Repository (Launch Credibility)

### Deterministic showcase artifacts
- `docs/showcase-evidence/simulation_result.json` records deterministic pass/fail schema, executed steps, and assertion results.
- `docs/showcase-evidence/simulation_uart.log` includes expected success signal: `HIL Stress Test Passed`.
- `docs/HIL_DISPLACEMENT_SHOWCASE.md` documents methodology and engineering narrative for parity-style validation.

### Product surface already demonstrable
- CLI and CI runner workflows in `README.md` and `core/docs/ci_test_runner.md`.
- Debug workflows in `core/docs/debugging.md` and `core/docs/vscode_debugging.md`.
- Existing comparison content for QEMU and Renode in `marketing/comparisons/` and `landing_page/comparisons/`.

## 5) Claims You Can Safely Make Today

1. LabWired already supports deterministic firmware simulation workflows and produces machine-readable artifacts suitable for CI.
2. LabWired is already demonstrable on real embedded scenarios (for example, H563 and CI smoke flows) with reproducible UART-based assertions.
3. The market is competitive now (open-source and enterprise incumbents), so launch timing matters.
4. Near-term GTM should emphasize service-assisted onboarding to capture customer-specific proof quickly.

## 6) Claims To Avoid Until Further Validation

1. Broad "best simulator" claims across all architectures and peripherals.
2. Hard ROI numbers (for example, cost multipliers) without customer-backed case studies.
3. Regulated/safety compliance outcomes unless accompanied by formal audit evidence.

## 7) Recommended Metrics To Collect During First Pilots

1. Time-to-first-passing CI scenario (days).
2. Number of flaky failures eliminated per release cycle.
3. Engineer hours saved per week versus hardware bench workflow.
4. Mean time to reproduce and isolate firmware failures.
5. Pilot-to-paid conversion and expansion by board/peripheral scope.
