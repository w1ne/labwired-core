---
name: verify-ci-regression
description: Set up and run deterministic pass/fail hardware-in-simulation CI regressions with LabWired MCP and GitHub Actions.
---

# Verify CI Regression

Use this skill to turn a hardware + firmware project into a deterministic, repeatable CI regression check that fails pull requests on electrical regressions, broken boot sequences, protocol errors, or missed timing/GPIO invariants.

## Workflow

### 1. Define a Deterministic Verification Oracle
A useful hardware CI gate has falsifiable pass/fail criteria — never rely on "it compiled" or "it didn't crash":
- **Electrical gate:** `labwired_validate` must report zero ERC/connectivity errors.
- **Functional gate (`labwired_verify` / `labwired_project_verify`):**
  - `uart_contains` / `uart_regex`: verify boot banner, self-test pass line, or formatted telemetry
  - `gpio_edges`: verify expected toggles, chip-select pulses, or PWM activity (`{ "pin": "...", "min_edges": N }`)
  - `motor_speed_reached`: verify actuator control loop response
  - `stimuli` & `uart_injections`: drive real user/sensor inputs at deterministic timestamps
  - `faults`: inject at least one realistic fault (e.g., disconnected sensor or stuck line) when safety behavior matters

### 2. Prove Determinism via MCP
- Call `labwired_validate` on the project diagram.
- Call `labwired_compile` (if building from source) and `labwired_verify` with the full oracle.
- If `labwired_verify` returns a deferred `job_id`, poll with that `job_id` until settled (polling is free and does not consume additional daily quota).
- Inspect `verification_gaps` in the response: address any unmodeled or unattached stimuli so the pass verdict is genuinely earned.

### 3. Wire GitHub Actions CI
Create `.github/workflows/hardware-ci.yml` so every push and PR runs the same check headlessly:

```yaml
name: LabWired Hardware CI

on:
  push:
    branches: [main]
  pull_request:

jobs:
  hardware-verify:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Run LabWired deterministic verification
        env:
          LABWIRED_API_KEY: ${{ secrets.LABWIRED_API_KEY }}
        run: |
          curl -fsSL https://labwired.com/install.sh | sh
          labwired test --diagram diagram.json --firmware firmware.elf --timeout 30s
```

Remind the user to:
1. Copy their API key from `https://app.labwired.com/tools/mcp` (Step 2 -> API Token).
2. Add it as a repository secret named `LABWIRED_API_KEY`.
3. Optionally view live public CI run history at `https://app.labwired.com/ci`.

### 4. Interpret Failures Clearly
When a CI regression fails, report:
- Which oracle clause failed (expected vs observed UART/GPIO/bus output).
- Whether the failure is a diagram/ERC issue (`labwired_validate`), compile error (`labwired_compile`), or runtime behavior regression (`labwired_verify`).
- Exact reproduction command or MCP call so another developer can debug it immediately.
