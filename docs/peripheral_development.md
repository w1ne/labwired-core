# Peripheral Development Guide

This document outlines the architectural patterns and best practices for implementing custom peripherals in LabWired.

## 1. Peripheral Interface Contract

All peripherals must implement the `Peripheral` trait from `labwired-core`. This interface abstracts the hardware behavior into three primary operations.

```rust
pub trait Peripheral: std::fmt::Debug + Send {
    /// Handler for CPU read operations.
    /// Offset: Memory address relative to the peripheral base.
    /// Returns: 8-bit value or SimResult::Err on BusFault.
    fn read(&self, offset: u64) -> SimResult<u8>;

    /// Handler for CPU write operations.
    /// Offset: Memory address relative to the peripheral base.
    /// Value: 8-bit data to write.
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()>;

    /// State update function called once per simulation step.
    /// Used for time-based logic (timers, UART baud rate) and interrupt generation.
    fn tick(&mut self) -> PeripheralTickResult;
}
```

## 2. Register Access Patterns

### Byte-Level Granularity
The `SystemBus` performs all transactions at byte granularity. 32-bit CPU instructions (like `STR`) are decomposed into four consecutive 8-bit writes. Peripherals must handle this reconstruction if they model 32-bit registers.

**Implementation Pattern:**
```rust
fn read(&self, offset: u64) -> SimResult<u8> {
    // 1. Align offset to 4-byte boundary to identify the register
    let reg_val = match offset & !3 {
        0x00 => self.control_reg,
        0x04 => self.status_reg,
        _ => return Ok(0), // Define unmapped behavior (RAZ/WI)
    };
    
    // 2. Extract the specific byte requested
    let shift = (offset % 4) * 8;
    Ok((reg_val >> shift) as u8)
}
```

### Side Effects
Operations that clear flags or trigger hardware actions (e.g., "Write 1 to Clear") should be implemented in the `write` handler.

```rust
fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
    if offset == 0x00 { // status_reg
        // Clear flags if bit is set in value (W1C behavior)
        self.status_reg &= !(value as u32);
    }
    Ok(())
}
```

## 3. Time-Based Logic (`tick`)

The `tick()` method provides the simulation time base. It is invoked synchronously at the end of every CPU instruction cycle.

### Implementation Guidelines
- **Deterministic Execution**: Avoid `std::thread::sleep` or system time. Behavior must rely solely on the tick count to ensuring deterministic replayability.
- **Performance**: This method is on the hot path. Minimal logic should execute on every tick. Use state counters to decimate high-frequency logic.

**Example: 1MHz Timer at 100MHz CPU Clock**
```rust
fn tick(&mut self) -> PeripheralTickResult {
    self.cycles += 1;
    if self.cycles >= 100 { // 100 CPU cycles per timer tick
        self.cycles = 0;
        self.counter += 1;
        // Trigger IRQ logic...
    }
    PeripheralTickResult::default()
}
```

## 4. SVD Ingestion Tool

For standard peripherals, manual implementation of the register map is redundant. LabWired provides an SVD parsing tool to generate the boilerplate `PeripheralDescriptor` YAML.

**Usage:**
```bash
cargo run -p svd-ingestor -- --input STM32F4.svd --filter UART1 --output-dir crates/config/peripherals
```

This generates a YAML file compatible with the `GenericPeripheral` implementation, requiring only the hook logic to be written in Rust.
