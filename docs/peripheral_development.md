# Peripheral Development Guide

This guide explains how to develop custom, decoupled peripheral models for LabWired.

## Generating Peripherals from SVD
LabWired includes a tool to automatically generate peripheral descriptor YAML files from CMSIS-SVD files. This is the recommended way to onboard new MCUs.

```bash
# Generate a single peripheral
cargo run -p svd-ingestor -- --input STM32F401.svd --output-dir crates/config/peripherals --filter UART1

# Generate all peripherals in the SVD
cargo run -p svd-ingestor -- --input STM32F401.svd --output-dir crates/config/peripherals
```

The tool handles:
- Register arrays (unrolling `dim` elements)
- Nested clusters (flattening address space)
- Interrupt extraction

## The Peripheral Trait

All peripherals in LabWired must implement the `Peripheral` trait located in `labwired_core`.

```rust
pub trait Peripheral: std::fmt::Debug + Send {
    /// Read a single byte from the peripheral at the given offset
    fn read(&self, offset: u64) -> SimResult<u8>;

    /// Write a single byte to the peripheral at the given offset
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()>;

    /// Progress the peripheral state by one "tick"
    /// Returns any IRQs generated, cycles consumed, and any DMA bus requests
    fn tick(&mut self) -> PeripheralTickResult {
        PeripheralTickResult::default()
    }

    /// Return a JSON-serializable snapshot of the internal state
    fn snapshot(&self) -> serde_json::Value {
        serde_json::Value::Null
    }

    // Downcasting support for internal communication
    fn as_any(&self) -> Option<&dyn Any> { None }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> { None }
}
```

## Implementation Best Practices

### 1. State Management
Peripherals are generally "dumb" state containers. Use bit manipulation to handle multi-byte registers.

### 2. Multi-byte Access
If your peripheral registers are 32-bit (common in ARM), implement helper methods to handle byte-wise routing in `read` and `write`:

```rust
fn read(&self, offset: u64) -> SimResult<u8> {
    let reg_val = match offset & !3 {
        0x00 => self.state_reg,
        0x04 => self.data_reg,
        _ => 0,
    };
    let byte_shift = (offset % 4) * 8;
    Ok(((reg_val >> byte_shift) & 0xFF) as u8)
}
```

### 3. Ticking & Cycle Accounting
The `tick()` method is called once per simulation step. Use this to simulate:
- Data processing delays
- Interrupt triggers
- Real-time counters

### 4. Snapshots
Always derive `serde::Serialize` on your peripheral struct and implement `snapshot()` to enable state-saving features. Use `#[serde(skip)]` for non-serializable fields like callbacks or `Arc<Mutex<...>>`.

## Example: Simple Temperature Sensor

Below is a complete implementation of a mock I2C-like temperature sensor with a Status Register (SR) and a Data Register (DR).

```rust
use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::any::Any;

#[derive(Debug, serde::Serialize)]
pub struct TempSensor {
    pub sr: u32, // Status: Bit 0 = Busy, Bit 1 = Data Ready
    pub dr: u32, // Data: Temperature in Celsius

    #[serde(skip)]
    update_interval: u32,
    #[serde(skip)]
    ticks: u32,
}

impl TempSensor {
    pub fn new(interval: u32) -> Self {
        Self { sr: 0, dr: 25, update_interval: interval, ticks: 0 }
    }
}

impl Peripheral for TempSensor {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let val = match offset {
            0x00..=0x03 => self.sr,
            0x04..=0x07 => self.dr,
            _ => 0,
        };
        let shift = (offset % 4) * 8;
        Ok(((val >> shift) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, _value: u8) -> SimResult<()> {
        // Temperature sensor registers are read-only in this mock
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        self.ticks += 1;
        let mut irq = false;

        if self.ticks >= self.update_interval {
            self.ticks = 0;
            self.dr += 1; // Simulate rising temperature
            self.sr |= 0x2; // Set Data Ready bit
            irq = true; // Signal interrupt
        }

        PeripheralTickResult { irq, cycles: 1 }
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn as_any(&self) -> Option<&dyn Any> { Some(self) }
}
```

## DMA Bus Mastering

If your peripheral needs to perform DMA transfers, it can return `DmaRequest`s from `tick()`.

```rust
impl Peripheral for MyDmaController {
    fn tick(&mut self) -> PeripheralTickResult {
        let mut dma_requests = Vec::new();
        if self.active {
            dma_requests.push(DmaRequest {
                addr: self.src_addr as u64,
                val: 0, // Not used for Read
                direction: DmaDirection::Read,
            });
            dma_requests.push(DmaRequest {
                addr: self.dest_addr as u64,
                val: 0x42, // Value to write
                direction: DmaDirection::Write,
            });
        }
        PeripheralTickResult {
            irq: false,
            cycles: 1,
            dma_requests,
        }
    }
}
```

> [!NOTE]
> The `SystemBus` executes these requests after the peripheral tick phase.

## Integrating Your Peripheral

To use your peripheral:
1. Register it in the `crates/core/src/peripherals/mod.rs`.
2. Map it in your `SystemBus` configuration.
### 3. (Optional) Define it in a YAML chip descriptor for dynamic loading.

## Rapid Prototyping with Stubs

Sometimes you don't need a full peripheral model, just enough to stop the firmware from crashing when it accesses a specific address. For this, use `StubPeripheral`.

In your `crate/core/src/peripherals/mod.rs` (or where you register peripherals):

```rust
use crate::peripherals::stub::StubPeripheral;

// ... inside your registration logic
let uart_stub = Box::new(StubPeripheral::new(0)); // Returns 0 on read
bus.peripherals.push(PeripheralEntry {
    name: "uart_stub".to_string(),
    base: 0x4000_4400, // UART2 Base
    size: 0x400,
    irq: None,
    dev: uart_stub,
});
```

You can also pre-populate values:
```rust
let mut stub = StubPeripheral::new(0);
stub.values.insert(0x00, 0x00C0); // SR returns TXE/TC
```

## Summary Checklist
- [ ] Implement `read` and `write` with byte-alignment logic.
- [ ] Use `tick()` for time-based behavior and IRQs.
- [ ] Derive `Serialize` and implement `snapshot()`.
- [ ] Add `as_any` for downcasting if high-level API access is needed.

---

# Step-by-Step Tutorial: Building Custom Peripherals

This tutorial guides you through creating custom peripheral models for LabWired, from basic concepts to a complete I2C temperature sensor.

## Learning Objectives

By the end of this tutorial, you will:
- Understand the `Peripheral` trait and its lifecycle
- Build peripherals with increasing complexity
- Handle multi-byte register access correctly
- Implement time-based behavior and interrupt generation
- Create a complete I2C sensor with realistic behavior

## Prerequisites

- Basic Rust knowledge (traits, ownership, error handling)
- Understanding of embedded systems concepts (memory-mapped I/O, registers, interrupts)
- Familiarity with the LabWired architecture (see [architecture.md](architecture.md))

## What You'll Build

We'll build five progressively complex peripherals:
1. **StatusRegister** - A simple read-only register
2. **ConfigurableDevice** - Adding writable state
3. **PeriodicSensor** - Time-based updates using `tick()`
4. **InterruptGenerator** - Signaling the CPU
5. **I2cTempSensor** - Complete temperature sensor with full register map

---

## Step 1: Minimal Peripheral - Read-Only Status

Let's start with the simplest possible peripheral: a single read-only status register.

```rust
use labwired_core::{Peripheral, SimResult};

/// A simple status register that always reports "ready"
#[derive(Debug, Default, serde::Serialize)]
pub struct StatusRegister {
    status: u8,
}

impl StatusRegister {
    pub fn new() -> Self {
        Self { status: 0x01 } // Bit 0 = Ready
    }
}

impl Peripheral for StatusRegister {
    fn read(&self, offset: u64) -> SimResult<u8> {
        match offset {
            0x00 => Ok(self.status),
            _ => Ok(0), // Unmapped offsets return 0
        }
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        // Read-only peripheral - writes are ignored
        Ok(())
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}
```

**Key Concepts:**
- `read()` returns the current value at a given offset
- `write()` can silently ignore writes for read-only registers
- `snapshot()` enables state inspection for debugging
- `#[derive(serde::Serialize)]` is required for snapshots

**Testing:**
```rust
let reg = StatusRegister::new();
assert_eq!(reg.read(0x00).unwrap(), 0x01);
```

---

## Step 2: Adding Writable State

Now let's add a configuration register that firmware can modify.

```rust
use labwired_core::{Peripheral, SimResult};

/// Device with both status (read-only) and config (read/write) registers
#[derive(Debug, serde::Serialize)]
pub struct ConfigurableDevice {
    status: u8,  // 0x00 - Read-only
    config: u8,  // 0x04 - Read/write
}

impl ConfigurableDevice {
    pub fn new() -> Self {
        Self {
            status: 0x01,  // Ready
            config: 0x00,  // Default: disabled
        }
    }
}

impl Peripheral for ConfigurableDevice {
    fn read(&self, offset: u64) -> SimResult<u8> {
        match offset {
            0x00 => Ok(self.status),
            0x04 => Ok(self.config),
            _ => Ok(0),
        }
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        match offset {
            0x00 => {
                // Status register is read-only - ignore writes
            }
            0x04 => {
                self.config = value;
                tracing::debug!("Config updated: {:#02x}", value);
            }
            _ => {}
        }
        Ok(())
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}
```

**Key Concepts:**
- Separate handling for read-only vs read/write registers
- Use `tracing::debug!()` for observability during simulation
- Register offsets typically align to 4-byte boundaries (0x00, 0x04, 0x08...)

---

## Step 3: Time-Based Behavior with `tick()`

Peripherals often need to update their state over time. The `tick()` method is called once per simulation step.

```rust
use labwired_core::{Peripheral, PeripheralTickResult, SimResult};

/// A sensor that updates its value periodically
#[derive(Debug, serde::Serialize)]
pub struct PeriodicSensor {
    data: u16,           // 0x00-0x01 - Current sensor reading

    #[serde(skip)]
    update_interval: u32,
    #[serde(skip)]
    tick_count: u32,
}

impl PeriodicSensor {
    pub fn new(interval: u32) -> Self {
        Self {
            data: 100,
            update_interval: interval,
            tick_count: 0,
        }
    }
}

impl Peripheral for PeriodicSensor {
    fn read(&self, offset: u64) -> SimResult<u8> {
        match offset {
            0x00 => Ok((self.data & 0xFF) as u8),        // Low byte
            0x01 => Ok(((self.data >> 8) & 0xFF) as u8), // High byte
            _ => Ok(0),
        }
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        // Read-only sensor
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        self.tick_count += 1;

        if self.tick_count >= self.update_interval {
            self.tick_count = 0;
            self.data = self.data.wrapping_add(1); // Increment sensor value
            tracing::trace!("Sensor updated: {}", self.data);
        }

        PeripheralTickResult {
            irq: false,
            cycles: 1,
            ..Default::default()
        }
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}
```

**Key Concepts:**
- `tick()` is called once per simulation step
- Use `#[serde(skip)]` for internal timing state that shouldn't be in snapshots
- Return `cycles: 1` to indicate this peripheral consumes one cycle per tick
- `wrapping_add()` prevents overflow panics

---

## Step 4: Interrupt Generation

Peripherals can signal the CPU when events occur.

```rust
use labwired_core::{Peripheral, PeripheralTickResult, SimResult};

/// A sensor that generates interrupts when data is ready
#[derive(Debug, serde::Serialize)]
pub struct InterruptingSensor {
    data: u16,           // 0x00-0x01 - Sensor data
    control: u8,         // 0x04 - Control register (bit 0 = IRQ enable)
    status: u8,          // 0x05 - Status register (bit 0 = data ready)

    #[serde(skip)]
    update_interval: u32,
    #[serde(skip)]
    tick_count: u32,
}

impl InterruptingSensor {
    pub fn new(interval: u32) -> Self {
        Self {
            data: 0,
            control: 0,
            status: 0,
            update_interval: interval,
            tick_count: 0,
        }
    }
}

impl Peripheral for InterruptingSensor {
    fn read(&self, offset: u64) -> SimResult<u8> {
        match offset {
            0x00 => Ok((self.data & 0xFF) as u8),
            0x01 => Ok(((self.data >> 8) & 0xFF) as u8),
            0x04 => Ok(self.control),
            0x05 => Ok(self.status),
            _ => Ok(0),
        }
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        match offset {
            0x04 => {
                self.control = value;
            }
            0x05 => {
                // Writing to status clears the data ready flag
                if value & 0x01 != 0 {
                    self.status &= !0x01;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        self.tick_count += 1;

        let mut should_irq = false;

        if self.tick_count >= self.update_interval {
            self.tick_count = 0;
            self.data = self.data.wrapping_add(10);
            self.status |= 0x01; // Set data ready flag

            // Generate IRQ if enabled
            if self.control & 0x01 != 0 {
                should_irq = true;
            }
        }

        PeripheralTickResult {
            irq: should_irq,
            cycles: 1,
            ..Default::default()
        }
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}
```

**Key Concepts:**
- Return `irq: true` from `tick()` to signal an interrupt
- Implement interrupt enable/disable via control registers
- Use status flags to indicate event state
- Firmware should clear status flags after handling

---

## Step 5: Complete I2C Temperature Sensor

Now let's build a realistic I2C temperature sensor with a full register map.

```rust
use labwired_core::{Peripheral, PeripheralTickResult, SimResult};
use std::any::Any;

/// TMP102-style I2C Temperature Sensor
///
/// Register Map:
/// - 0x00: Temperature (16-bit, read-only, 12-bit precision)
/// - 0x04: Configuration (16-bit, read/write)
/// - 0x08: T_LOW threshold (16-bit, read/write)
/// - 0x0C: T_HIGH threshold (16-bit, read/write)
#[derive(Debug, serde::Serialize)]
pub struct I2cTempSensor {
    // Registers
    temperature: i16,  // 12-bit signed, left-aligned in 16-bit
    config: u16,       // Configuration register
    t_low: i16,        // Low temperature threshold
    t_high: i16,       // High temperature threshold

    // Internal state
    #[serde(skip)]
    tick_count: u32,
    #[serde(skip)]
    update_interval: u32,
}

impl I2cTempSensor {
    pub fn new() -> Self {
        Self {
            temperature: 0x190,  // 25.0°C (in 12-bit format)
            config: 0x60A0,      // Default config
            t_low: 0x4B0,        // 75°C
            t_high: 0x500,       // 80°C
            tick_count: 0,
            update_interval: 1000,
        }
    }

    /// Helper: Read a 16-bit register
    fn read_reg(&self, offset: u64) -> u16 {
        match offset {
            0x00 => self.temperature as u16,
            0x04 => self.config,
            0x08 => self.t_low as u16,
            0x0C => self.t_high as u16,
            _ => 0,
        }
    }

    /// Helper: Write a 16-bit register
    fn write_reg(&mut self, offset: u64, value: u16) {
        match offset {
            0x00 => {
                // Temperature register is read-only
            }
            0x04 => {
                self.config = value;
                tracing::debug!("TMP102 config: {:#04x}", value);
            }
            0x08 => {
                self.t_low = value as i16;
            }
            0x0C => {
                self.t_high = value as i16;
            }
            _ => {}
        }
    }
}

impl Default for I2cTempSensor {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for I2cTempSensor {
    fn read(&self, offset: u64) -> SimResult<u8> {
        // Align to 4-byte boundary for register selection
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;

        let reg_val = self.read_reg(reg_offset);

        // Extract the requested byte (little-endian)
        Ok(((reg_val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;

        // Read-modify-write for byte access
        let mut reg_val = self.read_reg(reg_offset);
        let mask = 0xFF << (byte_offset * 8);
        reg_val &= !mask;
        reg_val |= (value as u16) << (byte_offset * 8);

        self.write_reg(reg_offset, reg_val);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        self.tick_count += 1;

        let mut should_irq = false;

        // Simulate temperature drift every update_interval ticks
        if self.tick_count >= self.update_interval {
            self.tick_count = 0;

            // Slight temperature increase
            self.temperature = self.temperature.wrapping_add(1);

            // Check thresholds (if alert mode is enabled in config)
            if self.config & 0x0010 != 0 {  // TM bit (thermostat mode)
                if self.temperature >= self.t_high as i16 {
                    should_irq = true;
                    tracing::warn!("Temperature HIGH: {}", self.temperature);
                } else if self.temperature <= self.t_low as i16 {
                    should_irq = true;
                    tracing::warn!("Temperature LOW: {}", self.temperature);
                }
            }
        }

        PeripheralTickResult {
            irq: should_irq,
            cycles: 1,
            ..Default::default()
        }
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}
```

**Key Concepts:**
- **Helper methods**: `read_reg()` and `write_reg()` simplify register access
- **Byte alignment**: Use `offset & !3` to align to 4-byte boundaries
- **Read-modify-write**: Essential for byte-level writes to multi-byte registers
- **Realistic behavior**: Temperature drift, threshold checking, interrupt generation
- **Downcasting support**: `as_any()` allows high-level API access if needed

---

## Firmware Integration Example

Here's how firmware would interact with our I2C sensor:

```rust
// In your firmware (e.g., crates/firmware/src/main.rs)
const TMP102_BASE: u32 = 0x5000_0000;

fn read_temperature() -> i16 {
    unsafe {
        let temp_ptr = TMP102_BASE as *const u16;
        temp_ptr.read_volatile() as i16
    }
}

fn set_high_threshold(threshold: i16) {
    unsafe {
        let t_high_ptr = (TMP102_BASE + 0x0C) as *mut u16;
        t_high_ptr.write_volatile(threshold as u16);
    }
}

#[entry]
fn main() -> ! {
    // Configure sensor
    set_high_threshold(0x320); // 50°C

    loop {
        let temp = read_temperature();
        // Process temperature...
    }
}
```

**System Configuration** (`system.yaml`):

```yaml
chip: configs/chips/stm32f103.yaml
peripherals:
  - name: "tmp102"
    type: "i2c_temp_sensor"
    base_address: 0x50000000
```

---

## Testing Strategies

### Unit Testing

Test peripheral logic in isolation:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_temperature_read() {
        let sensor = I2cTempSensor::new();

        // Read low byte
        let low = sensor.read(0x00).unwrap();
        // Read high byte
        let high = sensor.read(0x01).unwrap();

        let temp = (low as u16) | ((high as u16) << 8);
        assert_eq!(temp, 0x190); // 25°C
    }

    #[test]
    fn test_threshold_write() {
        let mut sensor = I2cTempSensor::new();

        // Write T_HIGH threshold (0x0C)
        sensor.write(0x0C, 0x20).unwrap(); // Low byte
        sensor.write(0x0D, 0x03).unwrap(); // High byte

        assert_eq!(sensor.t_high, 0x320); // 50°C
    }

    #[test]
    fn test_temperature_drift() {
        let mut sensor = I2cTempSensor::new();
        let initial_temp = sensor.temperature;

        // Tick 1000 times to trigger an update
        for _ in 0..1000 {
            sensor.tick();
        }

        assert_eq!(sensor.temperature, initial_temp + 1);
    }
}
```

### Integration Testing with Firmware

Create a test firmware that exercises the peripheral:

```rust
// crates/firmware-test/src/main.rs
#[entry]
fn main() -> ! {
    let temp = read_temperature();

    // Print temperature via UART
    println!("Temperature: {}", temp);

    loop {}
}
```

**CI Test Script** (`tests/tmp102.yaml`):

```yaml
schema_version: "1.0"
inputs:
  firmware: "target/thumbv7m-none-eabi/release/firmware-test"
  system: "configs/systems/tmp102_test.yaml"
limits:
  max_steps: 10000
assertions:
  - uart_contains: "Temperature: 400"
```

Run with:
```bash
labwired test --script tests/tmp102.yaml
```

---

## Advanced Patterns

### Multi-Register Atomic Operations

Some peripherals require atomic multi-register access:

```rust
impl Peripheral for AtomicCounter {
    fn read(&self, offset: u64) -> SimResult<u8> {
        match offset {
            0x00..=0x03 => {
                // Reading any byte of the counter latches all 4 bytes
                if !self.latched {
                    self.latch_value = self.counter;
                    self.latched = true;
                }
                let byte_offset = offset % 4;
                Ok(((self.latch_value >> (byte_offset * 8)) & 0xFF) as u8)
            }
            _ => Ok(0),
        }
    }
}
```

### Peripheral-to-Peripheral Communication

Use `explicit_irqs` for direct signaling:

```rust
fn tick(&mut self) -> PeripheralTickResult {
    if self.trigger_external_event {
        PeripheralTickResult {
            explicit_irqs: vec![16], // Signal EXTI0
            ..Default::default()
        }
    } else {
        PeripheralTickResult::default()
    }
}
```

### Performance Considerations

Return accurate cycle counts:

```rust
fn tick(&mut self) -> PeripheralTickResult {
    if self.dma_active {
        // DMA transfer takes multiple cycles
        PeripheralTickResult {
            cycles: 4,
            ..Default::default()
        }
    } else {
        PeripheralTickResult {
            cycles: 1,
            ..Default::default()
        }
    }
}
```

---

## Common Pitfalls

### ❌ Incorrect Byte Alignment

```rust
// WRONG: Doesn't handle byte-level access
fn read(&self, offset: u64) -> SimResult<u8> {
    match offset {
        0x00 => Ok(self.reg as u8), // Only returns low byte!
        _ => Ok(0),
    }
}

// CORRECT: Handle all bytes of a multi-byte register
fn read(&self, offset: u64) -> SimResult<u8> {
    let reg_offset = offset & !3;
    let byte_offset = offset % 4;
    let reg_val = self.read_reg(reg_offset);
    Ok(((reg_val >> (byte_offset * 8)) & 0xFF) as u8)
}
```

### ❌ Forgetting Reset Values

```rust
// WRONG: Registers start at 0
pub fn new() -> Self {
    Self { config: 0 }
}

// CORRECT: Match datasheet reset values
pub fn new() -> Self {
    Self { config: 0x60A0 } // Per TMP102 datasheet
}
```

### ❌ Interrupt Timing Bugs

```rust
// WRONG: IRQ fires before status flag is set
fn tick(&mut self) -> PeripheralTickResult {
    let irq = self.data_ready;
    self.status |= 0x01; // Set flag AFTER returning IRQ
    PeripheralTickResult { irq, ..Default::default() }
}

// CORRECT: Set flag before returning IRQ
fn tick(&mut self) -> PeripheralTickResult {
    self.status |= 0x01; // Set flag FIRST
    PeripheralTickResult {
        irq: self.data_ready,
        ..Default::default()
    }
}
```

### ❌ Thread Safety Issues

```rust
// WRONG: Shared state without synchronization
pub struct Peripheral {
    shared_buffer: Vec<u8>, // Not thread-safe!
}

// CORRECT: Use Arc<Mutex<>> for shared state
use std::sync::{Arc, Mutex};

pub struct Peripheral {
    #[serde(skip)]
    shared_buffer: Arc<Mutex<Vec<u8>>>,
}
```

---

## Next Steps

- **Explore existing peripherals**: Study `crates/core/src/peripherals/` for real-world examples
- **Read the architecture guide**: [architecture.md](architecture.md) explains the simulation loop
- **Try declarative registers**: [declarative_registers.md](declarative_registers.md) for YAML-based peripherals
- **Build advanced peripherals**: See [advanced_peripherals.md](advanced_peripherals.md) for DMA and EXTI

## Further Reading

- [Getting Started with Real Firmware](getting_started_firmware.md)
- [CI Test Runner Documentation](ci_test_runner.md)
- [System Descriptor Format](design/system_descriptors.md)
