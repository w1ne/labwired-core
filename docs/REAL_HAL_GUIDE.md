# Running Real HAL Firmware in LabWired

LabWired is designed to execute production-grade firmware binaries without modification. This guide explains how to migrate an existing STM32Cube or Embassy project to run in the simulator.

## 1. Prerequisites

*   **LabWired CLI**: Ensure `labwired` is installed (`cargo install --path core/crates/cli`).
*   **Firmware**: A compiled `.elf` file targeting a supported Cortex-M chip (e.g., STM32F103, STM32H5).
*   **System Manifest**: A `system.yaml` file describing your board.

## 2. The "Zero-Modify" Philosophy

Unlike other simulators that require you to recompile against a "host" target (e.g., x86), LabWired executes your **real ARM binary**.

**What works out of the box:**
*   **Vector Table**: Reset handlers, Stack Pointers, and Exceptions are loaded exactly as on hardware.
*   **Memory Map**: Flash at `0x0800_0000`, RAM at `0x2000_0000` (or chip specific).
*   **Peripheral MMIO**: Reads/Writes to mapped implementation addresses (e.g., `0x40013800` for USART1) are intercepted.

**What needs configuration:**
*   **External Hardware**: LEDs, Sensors, and Loopbacks must be defined in `system.yaml`.

## 3. Step-by-Step Guide

### Step A: Build Your Firmware
Build your project as usual.
```bash
# STM32CubeIDE / Make
make all

# Rust / Embassy
cargo build --release
```
Locate the output ELF: `target/thumbv7m-none-eabi/release/firmware`

### Step B: Choose a Chip Config
Check if your chip is supported:
```bash
ls core/configs/chips/
```
If your exact chip isn't there, pick the closest match (e.g., `stm32f103.yaml` covers most F1 variants) or create a new one (see [Digital Twin Spec](DIGITAL_TWIN_SPEC.md)).

### Step C: Create a System Manifest
Create `system.yaml` in your project root:

```yaml
name: "my-production-board"
chip: "core/configs/chips/stm32f103.yaml" # Path to chip definition
clock_freq: 72000000

# Map external interactions here
external_devices:
  - id: "status_led"
    type: "gpio"
    connection: "pc13"

  - id: "console"
    type: "uart"
    connection: "usart1"
```

### Step D: Run the Simulation
```bash
labwired --firmware ./build/firmware.elf --system system.yaml
```

## 4. Common Pitfalls & Solutions

### "Simulation hangs at Reset_Handler"
*   **Cause**: The vector table might be missing or offset.
*   **Fix**: Ensure your linker script places the vector table at `0x0800_0000`. If using a bootloader, adjust `SCB->VTOR` in your startup code.

### "BusFault / MemoryViolation"
*   **Cause**: Firmware accessed a peripheral clock enable bit (RCC) that isn't fully modeled, or a peripheral address not in `chip.yaml`.
*   **Fix**:
    *   Check the log for the specific fault address.
    *   Add a `StubPeripheral` in `system.yaml` to mock that address range if strict behavior isn't needed.

### "UART output is missing"
*   **Cause**: GPIO alternate functions not configured?
*   **Fix**: LabWired currently routes UART directly based on the Peripheral ID (e.g., `USART1`), ignoring precise GPIO pin muxing complexity for ease of use. Ensure you are writing to the correct UART instance.

## 5. Continuous Integration (CI) Example

```yaml
# .github/workflows/test.yml
steps:
  - name: Run Firmware Test
    run: |
      labwired test \
        --firmware target/release/app \
        --system system.yaml \
        --script tests/smoke_test.yaml
```
