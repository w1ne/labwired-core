# CI Integration Guide

This guide shows you how to integrate LabWired firmware testing into your continuous integration pipeline, replacing physical hardware with deterministic simulation.

## Quick Start (5 Minutes)

### GitHub Actions

1. **Copy the workflow template:**
   ```bash
   cp examples/workflows/github-actions.yml .github/workflows/firmware-test.yml
   ```

2. **Create a test script** (`tests/firmware-test.yaml`):
   ```yaml
   schema_version: "1.0"

   inputs:
     firmware: target/thumbv7m-none-eabi/release/my-firmware
     system: system.yaml

   limits:
     max_steps: 100000
     wall_time_ms: 5000

   assertions:
     - uart_contains: "Tests passed"
     - expected_stop_reason: breakpoint
   ```

3. **Update the workflow** to build your firmware:
   ```yaml
   - name: Build firmware
     run: cargo build --release --target thumbv7m-none-eabi -p my-firmware
   ```

4. **Commit and push** - your tests will run automatically on every push!

### GitLab CI

1. **Copy the template:**
   ```bash
   cp examples/workflows/gitlab-ci.yml .gitlab-ci.yml
   ```

2. **Create your test script** (same as above)

3. **Update firmware build commands** in `.gitlab-ci.yml`

4. **Commit and push** - pipeline runs automatically!

## Integration Methods

### Method 1: Composite Action (GitHub Actions Only)

**Best for:** Most GitHub Actions users

**Pros:**
- Simplest setup
- Automatic caching
- Maintained by LabWired team

**Example:**
```yaml
- uses: w1ne/labwired/.github/actions/labwired-test@main
  with:
    script: tests/firmware-test.yaml
    output_dir: test-results
```

### Method 2: Docker Image

**Best for:** Consistent environments, any CI platform

**Pros:**
- Same environment everywhere
- No build time for LabWired
- Works on any CI that supports Docker

**Example (when published):**
```yaml
- name: Run tests
  run: |
    docker run --rm \
      -v $PWD:/workspace \
      -w /workspace \
      ghcr.io/w1ne/labwired:latest \
      test --script tests/firmware-test.yaml --output-dir results
```

### Method 3: Build from Source

**Best for:** Latest features, custom modifications

**Pros:**
- Always latest code
- Can apply patches
- Full control

**Example:**
```yaml
- name: Build LabWired
  run: |
    git clone https://github.com/w1ne/labwired.git
    cd labwired && cargo build --release -p labwired-cli
    echo "$PWD/target/release" >> $GITHUB_PATH
```

## Hardware-in-the-Loop Replacement

### Traditional Approach (Physical Hardware)

```yaml
test:
  script:
    # Flash firmware to physical board
    - openocd -f interface/stlink.cfg -c "program firmware.elf verify reset"
    # Connect to serial port
    - screen /dev/ttyUSB0 115200
    # Manual verification or flaky serial parsing
```

**Problems:**
- Requires physical hardware in CI environment
- Non-deterministic (timing, race conditions)
- Slow (flashing, serial delays)
- Can't test fault injection
- Limited parallelization

### LabWired Approach (Simulation)

```yaml
test:
  script:
    - labwired test --script tests/firmware-test.yaml
```

**Benefits:**
- ✅ No hardware required
- ✅ Deterministic and reproducible
- ✅ Fast (no flashing delays)
- ✅ Fault injection built-in
- ✅ Unlimited parallelization

## Test Script Authoring

### Basic Structure

```yaml
schema_version: "1.0"

inputs:
  firmware: path/to/firmware.elf
  system: configs/system.yaml  # Optional

limits:
  max_steps: 100000           # Stop after N instructions
  max_cycles: 1000000         # Stop after N cycles
  wall_time_ms: 5000          # Stop after N milliseconds
  max_uart_bytes: 10000       # Stop after N UART bytes

assertions:
  - uart_contains: "OK"       # UART must contain this string
  - uart_regex: "Test \\d+"   # UART must match regex
  - expected_stop_reason: breakpoint  # How should it stop?
```

### Common Patterns

**Pattern 1: Boot Test**
```yaml
# Verify firmware boots and prints banner
assertions:
  - uart_contains: "LabWired Firmware v1.0"
  - uart_contains: "System initialized"
limits:
  max_steps: 50000
```

**Pattern 2: Unit Test Suite**
```yaml
# Run embedded unit tests
assertions:
  - uart_contains: "Running 10 tests"
  - uart_contains: "test result: ok"
  - expected_stop_reason: breakpoint
limits:
  wall_time_ms: 10000
```

**Pattern 3: Timeout Detection**
```yaml
# Ensure firmware doesn't hang
limits:
  max_steps: 100000
assertions:
  - expected_stop_reason: max_steps  # Should hit limit, not hang
```

**Pattern 4: Fault Injection** (Future)
```yaml
# Test sensor failure handling
fault_injection:
  - at_cycle: 1000
    action: disconnect_peripheral
    target: I2C_SENSOR
assertions:
  - uart_contains: "Sensor error detected"
  - uart_contains: "Entering safe mode"
```

## Advanced Topics

### Matrix Testing

Test multiple configurations in parallel:

**GitHub Actions:**
```yaml
strategy:
  matrix:
    target:
      - thumbv6m-none-eabi   # Cortex-M0
      - thumbv7m-none-eabi   # Cortex-M3
      - thumbv7em-none-eabi  # Cortex-M4
    build_type: [debug, release]

steps:
  - name: Build
    run: cargo build --${{ matrix.build_type }} --target ${{ matrix.target }}

  - name: Test
    uses: w1ne/labwired/.github/actions/labwired-test@main
    with:
      script: tests/${{ matrix.target }}-test.yaml
```

**GitLab CI:**
```yaml
.test_template:
  script:
    - cargo build --target ${TARGET}
    - labwired test --script tests/${TARGET}-test.yaml

test:m0:
  extends: .test_template
  variables:
    TARGET: thumbv6m-none-eabi

test:m3:
  extends: .test_template
  variables:
    TARGET: thumbv7m-none-eabi
```

### Caching Strategies

**GitHub Actions:**
```yaml
- uses: Swatinem/rust-cache@v2
  with:
    workspaces: |
      . -> target
      labwired -> labwired/target
```

**GitLab CI:**
```yaml
cache:
  key: ${CI_COMMIT_REF_SLUG}-${CI_JOB_NAME}
  paths:
    - .cargo/
    - target/
  policy: pull-push
```

### Artifact Management

All test runs produce three artifacts:

1. **`result.json`** - Machine-readable results
   ```json
   {
     "status": "pass",
     "instructions_executed": 45231,
     "cycles_executed": 45231,
     "stop_reason": "breakpoint",
     "firmware_hash": "abc123...",
     "assertions": [...]
   }
   ```

2. **`uart.log`** - Complete UART output
   ```
   LabWired Firmware v1.0
   Running tests...
   [PASS] test_addition
   [PASS] test_multiplication
   All tests passed!
   ```

3. **`junit.xml`** - JUnit format for CI integration
   ```xml
   <testsuite name="firmware-tests" tests="2" failures="0">
     <testcase name="uart_contains: OK" />
     <testcase name="expected_stop_reason: breakpoint" />
   </testsuite>
   ```

**Upload in GitHub Actions:**
```yaml
- uses: actions/upload-artifact@v4
  if: always()
  with:
    name: test-results
    path: test-results/
    retention-days: 30
```

**Upload in GitLab CI:**
```yaml
artifacts:
  when: always
  paths:
    - test-results/
  reports:
    junit: test-results/junit.xml
  expire_in: 30 days
```

### Test Reporting

**GitHub Actions** automatically shows JUnit results in the UI.

**GitLab CI** integrates with merge request test reports:
```yaml
artifacts:
  reports:
    junit: test-results/junit.xml
```

**Custom Summary (GitHub):**
```yaml
- name: Generate summary
  if: always()
  run: |
    echo "## Test Results" >> $GITHUB_STEP_SUMMARY
    echo "Status: $(jq -r .status test-results/result.json)" >> $GITHUB_STEP_SUMMARY
    echo "Instructions: $(jq -r .instructions_executed test-results/result.json)" >> $GITHUB_STEP_SUMMARY
```

## Troubleshooting

### Problem: Tests timeout

**Symptom:** Tests hit `wall_time_ms` limit

**Solutions:**
1. Increase timeout in test script
2. Build firmware in release mode (faster)
3. Reduce `max_steps` if firmware is in infinite loop

### Problem: Assertion failures

**Symptom:** `uart_contains` assertion fails

**Solutions:**
1. Download `uart.log` artifact to see actual output
2. Check for typos in expected strings
3. Use `uart_regex` for flexible matching
4. Ensure firmware actually prints to UART

### Problem: Build failures in CI

**Symptom:** `cargo build` fails with linker errors

**Solutions:**
1. Ensure correct target installed: `rustup target add thumbv7m-none-eabi`
2. Check `Cargo.toml` has correct dependencies
3. Verify `memory.x` and `link.x` are present

### Problem: Docker image not found

**Symptom:** `docker pull ghcr.io/w1ne/labwired:latest` fails

**Solution:** The Docker image is not yet published. Use the composite action or build from source method instead.

### Problem: Inconsistent results

**Symptom:** Tests pass locally but fail in CI (or vice versa)

**Solutions:**
1. Ensure same LabWired version (pin to specific tag)
2. Use same firmware build profile (debug vs release)
3. Check for non-deterministic firmware behavior
4. Verify system.yaml is identical

## Migration Checklist

Migrating from physical hardware testing to LabWired:

- [ ] Identify firmware test cases currently using hardware
- [ ] Create LabWired test scripts for each case
- [ ] Verify assertions match expected hardware behavior
- [ ] Run tests locally to validate
- [ ] Add CI workflow using templates
- [ ] Run parallel (hardware + simulation) for validation period
- [ ] Monitor for discrepancies
- [ ] Gradually phase out hardware tests
- [ ] Document any simulation limitations

## Best Practices

1. **Start Simple:** Begin with basic boot tests, add complexity gradually
2. **Version Pin:** Use specific LabWired versions in production (`@v0.9.0` not `@main`)
3. **Fast Feedback:** Keep test scripts under 5 seconds wall time
4. **Artifact Everything:** Always upload test results, even on failure
5. **Matrix Test:** Test multiple targets/configs to catch portability issues
6. **Deterministic Firmware:** Avoid random numbers, timestamps in test assertions
7. **Clear Assertions:** Make failure messages obvious (specific strings)
8. **Cache Aggressively:** Cache Rust dependencies and LabWired builds

## Next Steps

1. ✅ Copy a workflow template
2. ✅ Create your first test script
3. ✅ Run locally: `cargo build --release -p labwired-cli && ./target/release/labwired test --script tests/my-test.yaml`
4. ✅ Commit and push
5. ✅ Monitor CI run and download artifacts
6. ✅ Iterate on assertions based on actual UART output
7. ✅ Add more test cases
8. ✅ Set up matrix testing for multiple targets

## Examples

See working examples in [`examples/ci/`](../examples/ci/):
- `uart-ok.yaml` - Basic passing test
- `dummy-fail-uart.yaml` - Assertion failure example
- `dummy-max-steps.yaml` - Step limit test

## Support

- [Test Script Schema Documentation](./test_script_schema.md)
- [GitHub Issues](https://github.com/w1ne/labwired/issues)
- [Example Workflows](../examples/workflows/)
