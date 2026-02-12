# Declarative Register Maps

One of LabWired's core goals is to break the "peripheral modeling bottleneck." Instead of writing manual Rust code for every peripheral, LabWired uses declarative YAML specifications to define register maps and behaviors.

> [!NOTE]
> This feature is fully implemented as of v0.11.0.

## YAML Schema

The schema allows defining registers, their bitfields, and side effects (e.g., clear-on-read):

```yaml
peripheral: "SPI"
version: "1.0"
registers:
  - id: "CR1"
    address_offset: 0x00
    size: 16
    access: "R/W"
    reset_value: 0x0000
    fields:
      - name: "SPE"
        bit_range: [6, 6]
        description: "SPI Enable"
      - name: "MSTR"
        bit_range: [2, 2]
        description: "Master Selection"

  - id: "DR"
    address_offset: 0x0C
    size: 16
    access: "R/W"
    side_effects:
      on_read: "clear_rxne"
      on_write: "start_tx"
```

## Architecture

1. **Parser**: The `labwired-config` crate reads these YAML files into a structured `PeripheralDescriptor` IR.
2. **Generic Peripheral**: A implementation of the `Peripheral` trait in `labwired-core` (`GenericPeripheral`) that uses the descriptor to manage register access.
3. **Logic Hooks**: Support for attaching custom Rust logic (e.g., "start_tx") to specific register offsets defined in the YAML via `SideEffects`.

## Benefits

- **Consistency**: All peripherals share the same basic MMIO logic (matching addresses, bit masking).
- **Correctness**: Reset values and access permissions are enforced by the generic engine.
- **Speed**: New peripherals can be "modeled" in minutes by copying data from a silicon vendor's SVD file.

## Getting Started

See the [Peripheral Development Guide](./peripheral_development.md) for a step-by-step workflow to build your first declarative peripheral.
