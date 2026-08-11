# CI integration

Run the **same** `labwired test` command on your laptop and in GitHub Actions or GitLab. Pin a CLI release so firmware changes are judged by a fixed simulator version.

Default pin used in examples: **v0.21.0**.

---

## Local first

```bash
curl -fsSL https://labwired.com/install.sh | LABWIRED_VERSION=v0.21.0 sh

labwired test \
  --script tests/firmware-test.yaml \
  --output-dir out/labwired \
  --junit out/labwired/junit.xml
```

Exit **0** = pass. Non-zero = fail. Typical artifacts: `result.json`, `uart.log`, JUnit XML.

Product page: [labwired.com/ci](https://labwired.com/ci.html).

---

## GitHub Actions

Use the public action from `w1ne/labwired-core`. Pin both the **action commit** and the **CLI `version`**.

```yaml
name: Firmware simulation

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Build firmware
        run: # your normal firmware build; produce an ELF or flash image

      - id: labwired
        name: Run LabWired
        uses: w1ne/labwired-core/.github/actions/labwired-test@bfd879522914b586223081c4c89ba315db4a97ed
        with:
          script: tests/firmware-test.yaml
          version: v0.21.0
          output-dir: out/labwired
          args: --no-uart-stdout

      - name: Link the automatic LabWired artifact
        if: always()
        run: echo "${{ steps.labwired.outputs.artifact-url }}" >> "$GITHUB_STEP_SUMMARY"
```

**Inputs:** `script` (required), `version` (default `v0.21.0`), `output-dir`, `args`.  
**Outputs:** `status`, `summary-md`, `report-html`, `artifact-url`, `exit-code`.

The action downloads the pinned CLI release, runs `labwired test`, writes JUnit under `output-dir`, and uploads the directory even on failure.

Minimal one-liner style (if the CLI is already on the runner):

```yaml
- run: labwired test --script examples/ci/uart-ok.yaml --junit report.xml
```

---

## Container / self-built CLI

```bash
# After building from source
cargo build --release -p labwired-cli
./target/release/labwired test \
  --script tests/firmware-test.yaml \
  --output-dir out/labwired
```

Docker images and templates: [integration templates](integration-templates/README.md).

---

## What to assert

Use the [test script schema](ci_test_runner.md): UART substrings, register values, stop reasons, step limits. Prefer checks that match real product behavior — not only “boot completed.”

---

## Next

| | |
|--|--|
| [Run firmware (CLI)](getting_started_firmware.md) | Local install and `run` / `test` |
| [Test script schema](ci_test_runner.md) | YAML fields |
| [Fidelity](fidelity.md) | What pass means |
| [Troubleshooting](troubleshooting.md) | Max steps, empty UART, … |
