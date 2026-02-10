# Declarative Peripheral Modeling

LabWired supports a declarative format for modeling memory-mapped peripherals using YAML. This allows for rapid simulation of hardware behaviors without writing custom Rust code.

## File Format

A peripheral is defined by a `PeripheralDescriptor` saved in a `.yaml` file.

```yaml
peripheral: USART1
version: "1.0"
registers:
  - id: SR
    address_offset: 0x00
    size: 32
    access: ReadWrite
    reset_value: 0x000000C0
    fields:
      - name: TXE
        bit_range: [7, 7]
        description: Transmit data register empty
      - name: TC
        bit_range: [6, 6]
        description: Transmission complete
    side_effects:
      write_action: one_to_clear # Typical for status flags

interrupts:
  TX: 37
  RX: 38

timing:
  - id: tx_complete
    trigger:
      write:
        register: DR
    delay_cycles: 10
    action:
      set_bits:
        register: SR
        bits: 0x80 # Set TXE
    interrupt: TX
```

## Core Concepts

### Registers

Each register specifies its `address_offset`, `size` (in bits), and `access` permissions (`ReadWrite`, `ReadOnly`, `WriteOnly`).

### Side Effects

Registers can have specialized behaviors:
- `read_action: clear`: The register (or byte) is cleared to 0 after it is read (Read-To-Clear).
- `write_action: one_to_clear`: Writing '1' to a bit clears it to '0'. Common in interrupt status registers.
- `write_action: zero_to_clear`: Writing '0' clears it.

### Timing Hooks

Timing hooks allow you to model hardware delays and asynchronous events.

#### Triggers
- `read`: Triggered when the specified register is read.
- `write`: Triggered on write. Can optionally match a specific `value` and `mask`.
- `periodic`: Fires every `period_cycles`.

#### Actions
- `set_bits`: Sets specific bits in a target register.
- `clear_bits`: Clears bits in a target register.
- `write_value`: Writes a full value to a target register.

#### Interrupts
Actions can optionally signal an interrupt defined in the `interrupts` section.

## Usage

Generated YAML files are typically placed in the `assets/peripherals/` directory and loaded by the simulation manifest.

```yaml
# system.yaml
- name: USART1
  base_address: 0x40013800
  model: assets/peripherals/usart1.yaml
```
## Limitations

- **Byte-level Access**: The current `GenericPeripheral` implementation processes memory access byte-by-byte. Triggers on multi-byte writes will currently match against the individual bytes being written.
- **Cycle Accuracy**: Timing hooks use `delay_cycles` which are decremented in the `tick()` method. The accuracy depends on the simulation clock frequency relative to the peripheral's internal clock.
