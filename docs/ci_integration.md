# CI integration

Run the **same** `labwired test` command on your laptop and in GitHub Actions or GitLab. Pin a CLI release so firmware changes are judged by a fixed simulator version.

Default pin used in examples: **v0.22.1**.

---

## Local first

```bash
curl -fsSL https://labwired.com/install.sh | LABWIRED_VERSION=v0.22.1 sh

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
        uses: w1ne/labwired-core/.github/actions/labwired-test@cfc26b5df0218cceedcd832bc689c89d00a13e2d
        with:
          script: tests/firmware-test.yaml
          version: v0.22.1
          output-dir: out/labwired
          args: --no-uart-stdout

      - name: Link the automatic LabWired artifact
        if: always()
        run: echo "${{ steps.labwired.outputs.artifact-url }}" >> "$GITHUB_STEP_SUMMARY"
```

The public action reference is an **immutable action-source pin** to
`cfc26b5df0218cceedcd832bc689c89d00a13e2d`. Inputs: `script` (required), `version`
(default `v0.22.1`), `output-dir`, and `args`. The action downloads that CLI
release, writes JUnit to `output-dir/junit.xml`, appends `summary.md` to the job
summary, and always uploads the output directory (including on failure).

**Outputs:** `status`, `summary-md`, `report-html`, `artifact-url`, `exit-code`
(via the `labwired` step id).

---

## Container runner

The release image uses `labwired` as the entrypoint. Pass `test` after the image
name — do not repeat `labwired` in the container command:

```bash
docker run --rm \
  --user "$(id -u):$(id -g)" \
  --volume "$PWD:/workspace" \
  --workdir /workspace \
  ghcr.io/w1ne/labwired:v0.22.1 \
  test --script tests/firmware-test.yaml \
       --output-dir out/labwired \
       --no-uart-stdout
```

When you bind-mount a workspace, pass the caller UID/GID so generated artifacts
stay writable on the host.

---

## GitLab CI

Clear the image entrypoint so GitLab can start its job shell. See
[integration-templates/gitlab-ci.yml](integration-templates/gitlab-ci.yml):

```yaml
test:firmware:
  image:
    name: ghcr.io/w1ne/labwired:v0.22.1
    entrypoint: [""]
  script:
    - labwired test --script tests/firmware-test.yaml --output-dir out/labwired --no-uart-stdout
```

---

## Artifacts

Use `--output-dir` everywhere. A run writes `result.json`, `uart.log`, and JUnit
under that directory. The GitHub action always uploads the directory; other CI
systems should retain it on failure.

---

## Advanced: build from source

```bash
cargo build --release -p labwired-cli
./target/release/labwired test \
  --script tests/firmware-test.yaml \
  --output-dir out/labwired
```

More templates: [integration templates](integration-templates/README.md).

---

## What to assert

Use the [test script schema](ci_test_runner.md): UART substrings, register values,
stop reasons, step limits. Prefer checks that match product behavior — not only
“boot completed.”

---

## Next

| | |
|--|--|
| [Run firmware (CLI)](getting_started_firmware.md) | Local install and `run` / `test` |
| [Test script schema](ci_test_runner.md) | YAML fields |
| [Fidelity](fidelity.md) | What pass means |
| [Troubleshooting](troubleshooting.md) | Max steps, empty UART, … |
