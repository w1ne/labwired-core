# CI Workflow Templates

This directory contains reference CI/CD workflow templates for integrating LabWired firmware testing into your continuous integration pipelines.

## Available Templates

### GitHub Actions ([github-actions.yml](./github-actions.yml))

Complete GitHub Actions workflow showing three integration approaches:

1. **Composite Action** (Recommended) - Use the pre-built LabWired action
2. **Docker Image** - Use the published Docker image directly
3. **Build from Source** - Clone and build LabWired in your workflow

Also includes a matrix testing example for testing across multiple ARM targets.

**Quick Start:**
```bash
# Copy to your repository
cp examples/workflows/github-actions.yml .github/workflows/firmware-test.yml

# Customize the firmware build commands and test scripts
# Then commit and push
```

### GitLab CI ([gitlab-ci.yml](./gitlab-ci.yml))

GitLab CI pipeline template with:

- Firmware build stage with caching
- Test execution with artifact collection
- JUnit XML test reporting integration
- Matrix testing across Cortex-M variants
- Test summary generation

**Quick Start:**
```bash
# Copy to your repository root
cp examples/workflows/gitlab-ci.yml .gitlab-ci.yml

# Customize the firmware package name and test scripts
# Then commit and push
```

## Customization Guide

### 1. Update Firmware Build Commands

Replace `your-firmware` with your actual firmware package name:

```yaml
# Before
cargo build --release --target thumbv7m-none-eabi -p your-firmware

# After
cargo build --release --target thumbv7m-none-eabi -p my-device-firmware
```

### 2. Configure Test Scripts

Update the test script paths to match your project structure:

```yaml
# Point to your actual test scripts
script: tests/my-firmware-test.yaml
```

### 3. Adjust Targets

Modify the ARM targets based on your hardware:

```yaml
# For Cortex-M0/M0+
targets: thumbv6m-none-eabi

# For Cortex-M3
targets: thumbv7m-none-eabi

# For Cortex-M4/M4F
targets: thumbv7em-none-eabi
```

## Integration Methods Comparison

| Method | Speed | Setup Complexity | Use Case |
|--------|-------|------------------|----------|
| **Composite Action** | Fast (cached) | Low | Most projects, recommended |
| **Docker Image** | Fast | Low | When you need consistent environment |
| **Build from Source** | Slow | Medium | When you need latest features or custom builds |

## Test Script Examples

See [examples/ci/](../ci/) for working test script examples:

- `uart-ok.yaml` - Basic UART output validation
- `dummy-max-steps.yaml` - Step limit testing
- `dummy-max-cycles.yaml` - Cycle limit testing
- `dummy-fail-uart.yaml` - Assertion failure example

## Artifact Collection

All templates collect test artifacts including:

- `result.json` - Machine-readable test results
- `uart.log` - Complete UART output
- `junit.xml` - JUnit format for CI integration

These artifacts are automatically uploaded and available in your CI dashboard.

## Hardware-in-the-Loop Replacement

### Before (Physical Hardware)
```yaml
- name: Flash and test
  run: |
    openocd -f interface/stlink.cfg -f target/stm32f1x.cfg -c "program firmware.elf verify reset exit"
    # Wait for serial output...
```

### After (LabWired Simulation)
```yaml
- name: Test firmware
  uses: w1ne/labwired/.github/actions/labwired-test@main
  with:
    script: tests/firmware-test.yaml
```

**Benefits:**
- No physical hardware required
- Deterministic, reproducible results
- Parallel testing across multiple targets
- Faster feedback (no flashing delays)
- Test fault injection scenarios

## Troubleshooting

### Build Failures

**Problem:** `cargo build` fails with linking errors

**Solution:** Ensure you have the correct target installed:
```bash
rustup target add thumbv7m-none-eabi
```

### Test Timeouts

**Problem:** Tests timeout with `wall_time` exceeded

**Solution:** Increase timeout in your test script:
```yaml
limits:
  wall_time_ms: 10000  # Increase from default 5000
```

### Assertion Failures

**Problem:** UART assertions fail unexpectedly

**Solution:** Check the actual UART output in `uart.log` artifact and adjust your assertions.

## Advanced Usage

### Caching

Both templates include caching strategies to speed up builds:

**GitHub Actions:**
```yaml
- uses: Swatinem/rust-cache@v2
```

**GitLab CI:**
```yaml
cache:
  key: ${CI_COMMIT_REF_SLUG}
  paths:
    - .cargo/
    - target/
```

### Matrix Testing

Test multiple configurations in parallel:

```yaml
strategy:
  matrix:
    target: [thumbv6m-none-eabi, thumbv7m-none-eabi]
    config: [debug, release]
```

## Next Steps

1. Copy the appropriate template to your repository
2. Customize firmware build commands and test scripts
3. Create test YAML files (see [examples/ci/README.md](../ci/README.md))
4. Commit and push to trigger your first CI run
5. Review artifacts and adjust assertions as needed

## Support

For more information:
- [LabWired Documentation](../../docs/)
- [Test Script Schema](../../docs/test_script_schema.md)
- [GitHub Issues](https://github.com/w1ne/labwired/issues)
