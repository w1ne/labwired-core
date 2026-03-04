[← Back to Hub](../README.md)

# LabWired Digital Twin Specification (v1.0)

This document formally specifies the configuration schemas used to define the "Digital Twin" of a hardware system in LabWired. A Digital Twin is composed of a **Chip Descriptor** (internal MCU architecture) and a **System Manifest** (board-level wiring).

## 1. File Structure

All configurations are YAML files.

*   `*.chip.yaml`: Defines an MCU (Memory, Core, Peripherals).
*   `system.yaml`: Defines a Board (Chip instance, External components, Wiring).

> [!TIP]
> **Agent Validation**: Use `labwired asset validate --system system.yaml` to verify your configuration against these schemas programmatically.
## 2. Chip Descriptor Schema (`*.chip.yaml`)

Describes the static properties of a microcontroller.

```yaml
# Schema Version (Optional, defaults to 1)
version: 1

# Metadata
name: "STM32F103C8"
family: "STM32F1"
vendor: "STMicroelectronics"

# Core Architecture
arch: "arm" # Options: "arm", "riscv"
core: "cortex-m3"

# Memory Map
# Defines the valid address spaces.
memory:
  flash:
    base: 0x08000000
    size: "64KB" # Supports KB/MB suffixes
    access: "rx" # Read-Execute

  ram:
    base: 0x20000000
    size: "20KB"
    access: "rwx" # Read-Write-Execute

# Peripheral Map
# Defines MMIO blocks found in the silicon.
peripherals:
  - id: "rcc"
    type: "rcc"
    base_address: 0x40021000
    description: "Reset and Clock Control"

  - id: "usart1"
    type: "uart"
    base_address: 0x40013800
    irq: 37 # NVIC Interrupt Number

  - id: "gpioa"
    type: "gpio"
    base_address: 0x40010800
```

### Supported Peripheral Types
*   `uart`: Univ. Sync/Async Receiver Transmitter
*   `gpio`: General Purpose I/O port
*   `timer`: Basic timer
*   `rcc`: Reset & Clock Control (stub)
*   `generic`: A generic MMIO block that logs accesses (good for unimplemented blocks)

## 3. System Manifest Schema (`system.yaml`)

Describes the board-level assembly and dynamic connections.

```yaml
# Metadata
name: "BluePill Development Board"

# Chip Reference
# Can be a relative path or an absolute path
chip: "../chips/stm32f103.yaml"

# Simulation Parameters
clock_freq: 72000000 # Main system clock in Hz (informative for metrics)

# External Device Wiring
# Connects chip peripherals to the outside world or simulated components.
external_devices:
  # Examples:

  # 1. UART Loopback / Console
  - id: "pc_console"
    type: "uart"
    connection: "usart1" # Connects to 'id' in chip.yaml
    backend: "stdio"     # Options: stdio, file, tcp

  # 2. LED (Visualizer)
  - id: "user_led"
    type: "gpio"
    connection: "pc13"
    active_level: "low"

  # 3. I2C Sensor (Mock)
  - id: "temp_sensor"
    type: "i2c_device"
    address: 0x48
    connection: "i2c1"
    model: "tmp102"      # Uses built-in behavioral model
```

## 4. Peripheral Stubbing

If your firmware accesses a peripheral that isn't fully modeled yet (causing a crash), you can declare it as a "Stub" in the chip descriptor or dynamically in the system manifest (future feature).

Currently, simply add it to `peripherals` list as `type: generic` to suppress memory faults:

```yaml
  - id: "usb_fs"
    type: "generic"
    base_address: 0x40005C00
    size: "1KB"
```
