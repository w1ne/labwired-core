# Declarative Register Definitions

LabWired uses declarative YAML specifications to define the register interface of peripherals. This system decouples the hardware description from the implementation logic, ensuring consistent behavior across all simulated devices.

## 1. Specification Schema

The schema defines the register map, access permissions, and side-effects.

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
        access: "R/W"

  - id: "DR"
    address_offset: 0x0C
    size: 16
    access: "R/W"
    side_effects:
      on_read: "clear_rxne"
      on_write: "start_tx"
```

### Key Components
- **id**: Unique identifier for the register.
- **address_offset**: Byte offset from the peripheral base address.
- **access**: Access permissions (`R`, `W`, `R/W`). Violations trigger a BusFault.
- **side_effects**: Hooks that invoke custom Rust logic when the register is accessed.

## 2. Implementation Architecture

The declarative system consists of three phases:

1.  **Parsing**: The `labwired-config` crate deserializes the YAML into a `PeripheralDescriptor` intermediate representation (IR).
2.  **Runtime**: The `GenericPeripheral` implementation in `labwired-core` uses this descriptor to serve `read()` and `write()` requests. It handles bounds checking, access permissions, and bit masking automatically.
3.  **Hooks**: The `GenericPeripheral` delegates to a `HookHandler` trait when a side-effect (e.g., `on_write: "start_tx"`) is triggered.

## 3. Workflow

1.  **Generation**: Use `svd-to-yaml` to generate the initial descriptor from vendor SVD files.
2.  **Refinement**: Manually add `side_effects` to registers that require custom logic (e.g., triggering a state machine transition).
3.  **Implementation**: Implement the corresponding hook functions in Rust.
