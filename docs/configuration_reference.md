# Configuration Reference

LabWired uses a YAML-based configuration system to define the simulated hardware environment. This separation allows the same firmware binary to be tested against different hardware configurations (e.g., changing memory sizes or remapping peripherals) without recompilation.

## 1. File Hierarchy

A complete simulation requires two descriptor files:

1.  **Chip Descriptor** (`chips/<name>.yaml`): Defines the internal architecture of the SoC (Flash/RAM size, internal peripheral addresses).
2.  **System Manifest** (`systems/<name>.yaml`): Instantiates a chip and defines board-level wiring (external sensors, UART loopbacks).

## 2. Chip Descriptor Schema

Defines the invariant properties of the silicon.

```yaml
name: "STM32F103"
flash:
  base: 0x08000000
  size: "64KB"  # Supports KB/MB suffixes
ram:
  base: 0x20000000
  size: "20KB"

peripherals:
  # Internal Peripheral Definition
  - id: "usart1"
    type: "uart"
    base_address: 0x40013800
    irq: 37
    config:
      profile: "stm32f1"  # Loads architecture-specific register map

  - id: "gpioa"
    type: "gpio"
    base_address: 0x40010800
    config:
      profile: "stm32f1"

  # Declarative Peripheral (Custom)
  - id: "my_custom_timer"
    type: "declarative"
    base_address: 0x40004000
    config:
      path: "../peripherals/custom_timer.yaml"
```

### Supported Peripheral Types
- `uart`, `usart`: Universal Asynchronous Receiver Transmitter
- `gpio`: General Purpose I/O
- `rcc`: Reset and Clock Control
- `timer`: Basic Timer
- `i2c`: Inter-Integrated Circuit
- `spi`: Serial Peripheral Interface
- `exti`: External Interrupt Controller
- `afio`: Alternate Function I/O
- `dma`: Direct Memory Access Controller
- `systick`: System Tick Timer
- `declarative`: Loads a generic peripheral from a YAML register description.

## 3. System Manifest Schema

Defines the board-level environment.

```yaml
name: "BluePill Board"
chip: "../chips/stm32f103.yaml"  # Path relative to this file

# External Device Connections (Planned)
connectors:
  - type: "uart"
    peripheral: "usart1"
    endpoint: "host_console"  # Pipes UART output to simulator stdout
```

## 4. CLI Usage

To run a simulation, provide both the firmware and the system manifest:

```bash
labwired --firmware firmware.elf --system configs/systems/bluepill.yaml
```

The simulator loads the system manifest, resolves the chip descriptor, initializes the memory map, and begins execution at the Reset Vector.
