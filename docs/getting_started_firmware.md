# Getting Started with Real Firmware

This guide helps you bring your own ARM Cortex-M firmware to the LabWired simulation environment.

## 1. Prerequisites

Ensure your firmware is compiled for an ARM Cortex-M target (e.g., `thumbv7m-none-eabi`). LabWired requires an **ELF binary** with valid section headers.

## 2. Configuration (chip.yaml)

LabWired uses YAML descriptors to define the memory map of your target MCU.

```yaml
# config/chips/my_chip.yaml
name: "MyCustomMCU"
flash:
  base: 0x08000000
  size: "512KB"
ram:
  base: 0x20000000
  size: "128KB"
peripherals:
  - id: "uart1"
    type: "uart"
    base_address: 0x40013800
```

## 3. Loading Firmware

Run the simulator pointing to your firmware and chip descriptor:

```bash
labwired run --config config/chips/my_chip.yaml -f build/firmware.elf
```

The loader will:
1. Parse the ELF entry point.
2. Load segments into the simulated Flash and RAM.
3. Automatically set the Stack Pointer (SP) and Program Counter (PC) from the vector table at `0x08000000`.

## 4. Troubleshooting Common Issues

### HardFault on Startup
- **Root Cause**: Usually a missing or misaligned vector table.
- **Fix**: Ensure your linker script (`link.x`) correctly places the vector table at the beginning of the Flash segment.

### Memory Violation at 0x...
- **Root Cause**: Firmware is trying to access a memory region not defined in `chip.yaml`.
- **Fix**: Check your memory map and ensure all peripheral regions (e.g., RCC, GPIO) are defined.

### Simulation Hangs
- **Root Cause**: Firmware is waiting for a hardware flag (e.g., `while(!(RCC->CR & RCC_CR_HSIRDY));`) that isn't provided by the mock.
- **Fix**: Stub the interfering peripheral or use the `--trace` flag to identify the spinning instruction.

## 5. Integration with HALs

LabWired is compatible with standard HAL libraries like `stm32f1xx-hal` and `embassy`.

### Example: `firmware-hal-test`
The project includes a reference implementation in `crates/firmware-hal-test` that demonstrates how to use the `stm32f1xx-hal` crate with LabWired.

To run this example:

1.  **Build the firmware**:
    ```bash
    cargo build --release --manifest-path crates/firmware-hal-test/Cargo.toml --target thumbv7m-none-eabi
    ```

2.  **Run the simulator**:
    ```bash
    cargo run -p labwired-cli -- \
      --firmware crates/firmware-hal-test/target/thumbv7m-none-eabi/release/firmware-hal-test \
      --system configs/systems/stm32f103-integrated-test.yaml
    ```

This firmware blinks an LED on PC13, demonstrating:
- Clock configuration (RCC)
- GPIO output configuration
- `cortex-m::asm::delay` usage

### Common Challenges with HALs
Since hardware flags are often partially mocked, you may encounter hangs where the HAL waits for a bit to set.
1.  **Use `StubPeripheral`** for unknown registers (see [Peripheral Development Guide](peripheral_development.md)).
2.  **Enable `--trace`** to see which register accesses are failing or where the code is spinning.
