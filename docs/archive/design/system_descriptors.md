# Design: LabWired System Descriptors

## Objective
Enable users to define complex hardware setups using declarative descriptor files. Instead of hardcoding memory maps and peripherals, the simulator will load a **System Manifest** that describes the chip (e.g., STM32F103) and its connected environment (e.g., an I2C sensor).

## 1. Descriptor Hierarchy

### A. Chip Descriptor (`chips/stm32f103.yaml`)
Defines the internal architecture of a specific SoC.
- **CPU**: Type (Cortex-M3), Clock frequency.
- **Memory Map**: Flash, RAM, and Reserved regions.
- **Internal Peripherals**: UART, GPIO, SPI, I2C, Timers (with base addresses).

### B. Board/System Manifest (`boards/my_project.yaml`)
Defines the "Wiring" and external components.
- **Target Chip**: Reference to a Chip Descriptor.
- **External Devices**: Stubs or functional models (e.g., Temperature Sensor).
- **Connections**: How external devices map to chip pins/peripherals (e.g., Sensor on I2C1).

## 2. Proposed YAML Schema (Example)

```yaml
# system.yaml
name: "Industrial Sensor Node"
chip: "stm32f103c8" # Looked up in chip registry

memory_overrides:
  flash_size: 128KB
  ram_size: 20KB

peripherals:
  - id: "uart1"
    type: "uart"
    base_address: 0x40013800
    config:
      baud_rate: 115200

external_devices:
  - id: "temp_sensor"
    type: "functional_stub"
    model: "tmp102"
    connection:
      interface: "i2c1"
      address: 0x48
    initial_state:
      temperature: 25.0
```

## 3. Implementation Workflow

1. **`labwired-config` Crate**: New crate to handle parsing and validation of YAML/JSON descriptors.
2. **Dynamic `SystemBus`**: Refactor `SystemBus` to hold a collection of `Peripheral` trait objects mapped by address range, rather than hardcoded fields.
3. **Peripheral Factory**: A central registry that instantiates peripherals (UART, Timer, etc.) based on the `type` string in the descriptor.
4. **Device Stubbing**: Support for "functional models" where users can provide simple scripts or parameters to simulate external sensor data.

## 4. Integration with CLI
The user will run:
```bash
labwired --firmware firmware.elf --system my_project.yaml
```
The simulator will auto-configure everything based on the YAML.

## 5. Next Steps (Iteration 6)
1. Define the `SystemManifest` struct.
2. Implement YAML parsing using `serde_yaml`.
3. Refactor `SystemBus` to use a dynamic memory map.
