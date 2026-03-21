[← Back to Hub](../README.md)

# CI Integration Guide

Run LabWired firmware tests in your CI pipeline. This guide uses GitHub Actions, but the same approach works with GitLab CI, Jenkins, or any container-based CI system.

## GitHub Actions

### Basic Setup

Add this workflow to `.github/workflows/firmware-test.yml`:

```yaml
name: Firmware Tests

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  firmware-test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: thumbv7m-none-eabi

      - name: Install LabWired
        run: cargo install labwired-cli

      - name: Build firmware
        run: cargo build --release --target thumbv7m-none-eabi

      - name: Run simulation tests
        run: |
          labwired test \
            --script tests/uart-smoke.yaml \
            --output-dir out/smoke \
            --no-uart-stdout

      - name: Upload test artifacts
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: simulation-results
          path: out/
```

### Using JUnit Reports

LabWired produces `junit.xml` for native CI integration:

```yaml
      - name: Run tests
        run: labwired test --script tests/smoke.yaml --output-dir out/

      - name: Publish test results
        uses: mikepenz/action-junit-report@v4
        if: always()
        with:
          report_paths: out/junit.xml
```

### Determinism Verification

Verify your firmware produces identical results across runs:

```yaml
      - name: Determinism check
        run: |
          for i in 1 2 3; do
            labwired test \
              --script tests/smoke.yaml \
              --output-dir out/run-$i \
              --no-uart-stdout
          done

          # Compare results
          diff <(cat out/run-1/result.json) <(cat out/run-2/result.json)
          diff <(cat out/run-1/result.json) <(cat out/run-3/result.json)
```

### Matrix Testing Across Chips

Test firmware on multiple target chips:

```yaml
jobs:
  test:
    strategy:
      matrix:
        target:
          - name: stm32f103
            script: tests/stm32f103-smoke.yaml
          - name: stm32h563
            script: tests/stm32h563-smoke.yaml
          - name: nrf52840
            script: tests/nrf52840-smoke.yaml
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Install LabWired
        run: cargo install labwired-cli
      - name: Test ${{ matrix.target.name }}
        run: |
          labwired test \
            --script ${{ matrix.target.script }} \
            --output-dir out/${{ matrix.target.name }}
```

## Docker-Based CI

For environments where Rust compilation isn't practical:

```yaml
      - name: Run in Docker
        run: |
          docker run --rm \
            -v ${{ github.workspace }}:/workspace \
            -w /workspace \
            ghcr.io/labwired/labwired:latest \
            test --script tests/smoke.yaml --output-dir out/
```

## GitLab CI

```yaml
firmware-test:
  image: rust:latest
  script:
    - cargo install labwired-cli
    - labwired test --script tests/smoke.yaml --output-dir out/
  artifacts:
    when: always
    paths:
      - out/
    reports:
      junit: out/junit.xml
```

## Key Tips

- **Always use `--no-uart-stdout`** in CI to keep logs clean
- **Upload artifacts** (`result.json`, `uart.log`, `junit.xml`) for post-mortem debugging
- **Set `wall_time` limits** in test scripts to prevent hung jobs
- **Use `--trace`** selectively (produces large files) — enable only for debugging failures
- Results are deterministic: if a test passes locally, it passes in CI (and vice versa)
