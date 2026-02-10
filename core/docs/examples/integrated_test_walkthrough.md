# Walkthrough: Integrated STM32 Test Example

This example demonstrates how to use the `labwired test` runner to verify a real STM32 firmware on a custom system configuration.

## 1. The Building Blocks

### Chip Descriptor ([stm32f103.yaml](file:///home/andrii/Projects/labwired/configs/chips/stm32f103.yaml))
Defines the memory map and peripherals (DMA, EXTI, AFIO, UART).

### System Manifest ([stm32f103-integrated-test.yaml](file:///home/andrii/Projects/labwired/configs/systems/stm32f103-integrated-test.yaml))
Wires the chip into a specific system context. In this case, it just uses the raw chip.

### Firmware ([main.rs](file:///home/andrii/Projects/labwired/crates/firmware/src/main.rs))
A Rust `no_std` firmware that:
- Performs 32-bit division.
- Configures DMA1 Channel 1 for a memory-to-memory transfer.
- Triggers a software interrupt via EXTI Line 0.
- Enters an infinite loop.

## 2. The Test Script

The test script ([stm32f103_integrated_test.yaml](file:///home/andrii/Projects/labwired/examples/tests/stm32f103_integrated_test.yaml)) automates the simulation:

```yaml
schema_version: "1.0"
inputs:
  firmware: "../../target/thumbv7m-none-eabi/debug/firmware"
  system: "../../configs/systems/stm32f103-integrated-test.yaml"
limits:
  max_steps: 10000
  no_progress_steps: 1000
assertions:
  - expected_stop_reason: no_progress
```

## 3. Running the Test

Run the test locally using the CLI:

```bash
labwired test --script examples/tests/stm32f103_integrated_test.yaml --output-dir out/test-results
```

### Inspecting Results
The runner generates several artifacts in `out/test-results`:
- `result.json`: Summary of execution stats and assertion results.
- `snapshot.json`: Full state of the CPU and peripherals at the end of the run.
- `uart.log`: Captured data from the UART peripheral.

## 4. CI Integration

You can easily integrate this into a GitHub Actions workflow:

```yaml
- name: Run LabWired Integrated Test
  run: |
    labwired test \
      --script examples/tests/stm32f103_integrated_test.yaml \
      --output-dir artifacts
```
