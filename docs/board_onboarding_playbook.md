# Board Onboarding Playbook

This guide documents the procedure for adding new board targets to LabWired. It is designed for contributors and agents to ensure consistent, high-quality board support.

## 1. Prerequisites

Before starting, acquire the following primary sources:
1.  **MCU Reference Manual** (e.g., STM32H5 Reference Manual).
2.  **Datasheet** (for memory map boundaries).
3.  **Board User Manual** (for LED/Button GPIO mapping).
4.  **CMSIS Device Headers** (optional but helpful for IRQ numbers).

## 2. Fit Assessment

Verify that LabWired supports the critical peripherals required for a minimal "smoke test" (boot + UART output).

**Supported Peripherals:**
- `rcc` (Reset and Clock Control) - Essential for boot.
- `gpio` (General Purpose I/O) - Essential for pin muxing.
- `uart` (Universal Asynchronous Receiver-Transmitter) - Essential for debug output.
- `systick` (System Tick Timer) - Essential for RTOS/HAL timekeeping.

**If the board requires complex peripherals (USB, Ethernet) for basic operation, it may not be a good candidate for initial onboarding.**

## 3. Implementation Steps

### Step 1: Chip Descriptor (`core/configs/chips/`)

Create a YAML file defining the MCU's memory map and internal peripherals.

**Example: `stm32h563.yaml`**
```yaml
name: "STM32H563"
flash:
  base: 0x08000000
  size: "2MB"
ram:
  base: 0x20000000
  size: "640KB"

peripherals:
  - id: "rcc"
    type: "rcc"
    base_address: 0x44020C00
    
  - id: "usart3"
    type: "uart"
    base_address: 0x40004800
    irq: 55
```
*Source of Truth: MCU Reference Manual (Memory Map section).*

### Step 2: System Manifest (`core/configs/systems/`)

Create a YAML file instantiating the chip and defining board-level connections.

**Example: `nucleo-h563zi.yaml`**
```yaml
name: "NUCLEO-H563ZI"
chip: "../chips/stm32h563.yaml"

# Define board-level connections (e.g., Virtual COM Port)
connectors:
  - type: "uart"
    peripheral: "usart3"
    endpoint: "host_console"
```
*Source of Truth: Board User Manual (Schematics/Connector definition).*

### Step 3: Smoke Firmware

Create a minimal Rust/C firmware to verify execution.
- **Goal**: Initialize UART and print "OK".
- **Constraints**: No external dependencies if possible (minimize HAL complexity).

## 4. Validation

Run the standardized onboarding test suite.

```bash
# 1. Build Smoke Firmware
cargo build --release --target thumbv7m-none-eabi -p smoke-firmware

# 2. Run Simulation with Audit
labwired --firmware target/thumbv7m-none-eabi/release/smoke-firmware \
         --system configs/systems/nucleo-h563zi.yaml \
         --audit-unsupported
```

**Success Criteria:**
1.  **Boot**: PC initializes to Reset Vector.
2.  **UART**: "OK" printed to stdout.
3.  **Audit**: No critical "Unmapped Peripheral" errors (warnings are acceptable for unused blocks).

## 5. Documentation

Create a folder in `core/examples/<board>/` containing:
1.  `README.md`: Board specific instructions.
2.  `system.yaml`: A local copy of the system manifest for easy reproduction.
3.  `smoke.rs` (or reference): The source code used for validation.
