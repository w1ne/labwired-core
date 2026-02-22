# CI Integration

This guide details the integration of LabWired firmware simulations into continuous integration (CI) pipelines. By replacing physical hardware with deterministic simulation, teams can achieve scalable, parallelized regression testing.

## 1. Quick Start

### GitHub Actions
To enable automated testing on every push, create a workflow file at `.github/workflows/firmware-test.yml`:

```yaml
name: Firmware Simulation
on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Build Firmware
        run: cargo build --release --target thumbv7m-none-eabi
      
      - name: Run Simulation
        uses: w1ne/labwired/.github/actions/labwired-test@main
        with:
          script: tests/basic_boot.yaml
          artifact_name: test-results
```

### GitLab CI
For GitLab, add the following to `.gitlab-ci.yml`:

```yaml
test_simulation:
  image: rust:latest
  script:
    - cargo build --release --target thumbv7m-none-eabi
    - curl -L https://github.com/w1ne/labwired/releases/latest/download/labwired-cli -o labwired
    - chmod +x labwired
    - ./labwired test --script tests/basic_boot.yaml
```

## 2. Test Script Schema

LabWired uses a YAML-based test definition format to specify inputs, constraints, and assertions.

```yaml
schema_version: "1.0"

inputs:
  firmware: "target/thumbv7m-none-eabi/release/firmware.elf"
  system: "configs/system.yaml"

limits:
  max_steps: 100000        # Instruction limit
  wall_time_ms: 5000       # Real-time timeout
  max_cycles: 50000000     # Simulation cycle limit

assertions:
  - uart_contains: "Boot Successful"
  - expected_stop_reason: "halt"
```

## 3. Integration Patterns

### Matrix Testing
Validate firmware across multiple compile targets or configurations in parallel.

**GitHub Actions Example:**
```yaml
strategy:
  matrix:
    target: [thumbv6m-none-eabi, thumbv7m-none-eabi]
steps:
  - run: cargo build --target ${{ matrix.target }}
  - uses: w1ne/labwired/.github/actions/labwired-test@main
    with:
      script: tests/${{ matrix.target }}.yaml
```

### Fault Injection
Simulate hardware failures (e.g., sensor disconnects) in CI to verify error handling paths that are difficult to trigger on physical devices.

```yaml
# tests/sensor_fail.yaml
steps:
  - run: 100ms
  - write_peripheral:
      id: "i2c1"
      reg: "CR1"
      value: 0x0000 # Disable I2C controller mid-operation
  - assert_log: "I2C Error Detected"
```

## 4. Artifacts and Reporting

The test runner produces machine-readable outputs for integration with CI reporting tools.

- **`result.json`**: Detailed execution statistics (cycles, instructions, assertion results).
- **`junit.xml`**: Standard JUnit format for test result visualization in GitHub/GitLab UI.
- **`uart.log`**: Captured serial output for debugging failures.

Ensure your CI pipeline is configured to archive these artifacts upon failure.

## 5. Onboarding KPI Tracking

For board onboarding competitiveness, `core-onboarding-smoke.yml` runs a deterministic smoke path and emits:

- `onboarding-metrics.json`: elapsed time, failure stage, and first error signature.
- `onboarding-summary.md`: per-target summary for step output.
- `onboarding-scoreboard.json` / `onboarding-scoreboard.md`: aggregated run-level view.

This workflow uses a soft threshold (`3600s`) to track time-to-first-smoke without blocking merges.
