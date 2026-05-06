# ADXL345 Sensor Lab Playground Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a polished, real-simulation ADXL345 Sensor Lab as the default flagship playground experience.

**Architecture:** Use STM32F103 + I2C1 as the first real route because `core/configs/chips/stm32f103.yaml` already includes I2C and the demo-blinky firmware uses that target. Add a real ADXL345 I2C component in `core`, expose typed sensor input/snapshot APIs through the WASM bridge, then compose a guided Sensor Lab UI in `packages/playground` using reusable UI pieces from `packages/ui`.

**Tech Stack:** Rust core simulator, wasm-bindgen bridge, React 19, TypeScript, Vite, Vitest, SVG editor components.

---

## File Structure

- Create `core/crates/core/src/peripherals/components/adxl345.rs`: ADXL345 I2C device model with register pointer, ID register, power/data format registers, and mutable X/Y/Z acceleration samples.
- Modify `core/crates/core/src/peripherals/components/mod.rs`: export `Adxl345`.
- Modify `core/crates/core/src/peripherals/i2c.rs`: add a focused test that proves an attached ADXL345 can be read through the existing I2C flow.
- Modify `core/crates/core/src/bus/mod.rs`: attach external I2C devices declared in `SystemManifest.external_devices` to I2C controller peripherals.
- Create `core/configs/systems/adxl345-sensor-lab.yaml`: STM32F103 system with ADXL345 attached to `i2c1` and a board IO binding for UI discovery.
- Create `core/examples/adxl345-sensor-lab/`: firmware crate, linker config, system file, README, and smoke script.
- Modify `core/Cargo.toml`: add the firmware crate as a workspace member and release profile entry.
- Modify `core/crates/wasm/src/lib.rs`: add `set_i2c_sensor_sample` and `get_i2c_sensor_states` exports.
- Modify `packages/ui/src/wasm/simulator-bridge.ts`: type and wrap new sensor APIs.
- Create `packages/ui/src/components/Adxl345Visualizer/Adxl345Visualizer.tsx`: board-like visualization, axis readout, and simple chart.
- Create `packages/ui/src/components/GuidedLab/GuidedLab.tsx`: step rail and advanced drawer shell.
- Modify `packages/ui/src/index.ts`: export new components and types.
- Modify `packages/ui/src/editor/components/index.ts`: register an ADXL345 editor component for the canvas.
- Create `packages/ui/src/editor/components/adxl345.tsx`: SVG component definition with I2C pins.
- Modify `packages/playground/src/bundled-configs.ts`: add `adxl345-sensor-lab` board config.
- Modify `packages/playground/src/bundled-configs.test.ts`: assert the bundled ADXL345 config.
- Modify `packages/playground/src/App.tsx`: route the ADXL345 config into Guided Stage mode and bridge sensor input to the simulator.
- Modify `packages/playground/src/playground.css`: layout and responsive styles for the Guided Stage.
- Add `packages/playground/public/wasm/demo-adxl345-sensor-lab.elf`: built firmware artifact copied from `core/target/thumbv7m-none-eabi/release/adxl345-sensor-lab`.

## Task 1: Core ADXL345 I2C Device

**Files:**
- Create: `core/crates/core/src/peripherals/components/adxl345.rs`
- Modify: `core/crates/core/src/peripherals/components/mod.rs`
- Modify: `core/crates/core/src/peripherals/i2c.rs`

- [ ] **Step 1: Write the failing I2C component test**

Add this test to the `#[cfg(test)] mod tests` block in `core/crates/core/src/peripherals/i2c.rs`:

```rust
#[test]
fn test_adxl345_devid_and_axis_read() {
    use crate::peripherals::components::Adxl345;

    let mut i2c = I2c::new();
    let mut sensor = Adxl345::new(0x53);
    sensor.set_sample(256, -128, 64);
    i2c.attach(Box::new(sensor));

    i2c.write(0x00, 0x01).unwrap();
    i2c.write(0x01, 0x01).unwrap();
    for _ in 0..10 {
        i2c.tick();
    }
    assert_ne!(i2c.peek(0x14).unwrap() & 0x01, 0);

    i2c.write(0x10, 0xA6).unwrap();
    for _ in 0..20 {
        i2c.tick();
    }
    assert_ne!(i2c.peek(0x14).unwrap() & 0x02, 0);

    i2c.write(0x10, 0x00).unwrap();
    for _ in 0..20 {
        i2c.tick();
    }

    i2c.write(0x01, 0x01).unwrap();
    for _ in 0..10 {
        i2c.tick();
    }
    i2c.write(0x10, 0xA7).unwrap();
    for _ in 0..40 {
        i2c.tick();
    }
    assert_eq!(i2c.read(0x10).unwrap(), 0xE5);

    i2c.write(0x01, 0x02).unwrap();
    for _ in 0..10 {
        i2c.tick();
    }

    i2c.write(0x01, 0x01).unwrap();
    for _ in 0..10 {
        i2c.tick();
    }
    i2c.write(0x10, 0xA6).unwrap();
    for _ in 0..20 {
        i2c.tick();
    }
    i2c.write(0x10, 0x32).unwrap();
    for _ in 0..20 {
        i2c.tick();
    }
    i2c.write(0x01, 0x01).unwrap();
    for _ in 0..10 {
        i2c.tick();
    }
    i2c.write(0x10, 0xA7).unwrap();
    for _ in 0..40 {
        i2c.tick();
    }

    assert_eq!(i2c.read(0x10).unwrap(), 0x00);
    assert_eq!(i2c.read(0x10).unwrap(), 0x01);
    assert_eq!(i2c.read(0x10).unwrap(), 0x80);
    assert_eq!(i2c.read(0x10).unwrap(), 0xFF);
    assert_eq!(i2c.read(0x10).unwrap(), 0x40);
    assert_eq!(i2c.read(0x10).unwrap(), 0x00);
}
```

- [ ] **Step 2: Run the failing test**

Run:

```bash
cd core
cargo test -p labwired-core test_adxl345_devid_and_axis_read -- --nocapture
```

Expected: compile fails with `no Adxl345 in peripherals::components`.

- [ ] **Step 3: Add the ADXL345 model**

Create `core/crates/core/src/peripherals/components/adxl345.rs`:

```rust
use crate::peripherals::i2c::I2cDevice;

#[derive(Debug, serde::Serialize)]
pub struct Adxl345 {
    address: u8,
    current_register: u8,
    register_address_written: bool,
    power_ctl: u8,
    data_format: u8,
    bw_rate: u8,
    sample_x: i16,
    sample_y: i16,
    sample_z: i16,
}

impl Default for Adxl345 {
    fn default() -> Self {
        Self::new(0x53)
    }
}

impl Adxl345 {
    pub fn new(address: u8) -> Self {
        Self {
            address,
            current_register: 0,
            register_address_written: false,
            power_ctl: 0,
            data_format: 0,
            bw_rate: 0x0A,
            sample_x: 0,
            sample_y: 0,
            sample_z: 256,
        }
    }

    pub fn set_sample(&mut self, x: i16, y: i16, z: i16) {
        self.sample_x = x;
        self.sample_y = y;
        self.sample_z = z;
    }

    pub fn sample(&self) -> (i16, i16, i16) {
        (self.sample_x, self.sample_y, self.sample_z)
    }

    fn read_register(&self, reg: u8) -> u8 {
        match reg {
            0x00 => 0xE5,
            0x2C => self.bw_rate,
            0x2D => self.power_ctl,
            0x31 => self.data_format,
            0x32 => self.sample_x as u16 as u8,
            0x33 => ((self.sample_x as u16) >> 8) as u8,
            0x34 => self.sample_y as u16 as u8,
            0x35 => ((self.sample_y as u16) >> 8) as u8,
            0x36 => self.sample_z as u16 as u8,
            0x37 => ((self.sample_z as u16) >> 8) as u8,
            _ => 0,
        }
    }

    fn write_register(&mut self, reg: u8, value: u8) {
        match reg {
            0x2C => self.bw_rate = value,
            0x2D => self.power_ctl = value,
            0x31 => self.data_format = value,
            _ => {}
        }
    }
}

impl I2cDevice for Adxl345 {
    fn address(&self) -> u8 {
        self.address
    }

    fn read(&mut self) -> u8 {
        let value = self.read_register(self.current_register);
        self.current_register = self.current_register.wrapping_add(1);
        value
    }

    fn write(&mut self, data: u8) {
        if !self.register_address_written {
            self.current_register = data;
            self.register_address_written = true;
        } else {
            self.write_register(self.current_register, data);
            self.current_register = self.current_register.wrapping_add(1);
        }
    }

    fn stop(&mut self) {
        self.register_address_written = false;
    }
}
```

Modify `core/crates/core/src/peripherals/components/mod.rs`:

```rust
pub mod adxl345;
pub mod mpu6050;

pub use adxl345::Adxl345;
pub use mpu6050::Mpu6050;
```

- [ ] **Step 4: Run the test**

Run:

```bash
cd core
cargo test -p labwired-core test_adxl345_devid_and_axis_read -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/crates/core/src/peripherals/components/adxl345.rs core/crates/core/src/peripherals/components/mod.rs core/crates/core/src/peripherals/i2c.rs
git commit -m "feat(core): add ADXL345 I2C component"
```

## Task 2: Attach External I2C Devices from System Config

**Files:**
- Modify: `core/crates/core/src/bus/mod.rs`
- Test: `core/crates/core/src/bus/mod.rs`
- Create: `core/configs/systems/adxl345-sensor-lab.yaml`

- [ ] **Step 1: Write the failing system-bus test**

Add this test to the existing `#[cfg(test)]` tests in `core/crates/core/src/bus/mod.rs`:

```rust
#[test]
fn test_from_config_attaches_adxl345_external_device_to_i2c() {
    use labwired_config::{Arch, ChipDescriptor, ExternalDevice, MemoryRange, PeripheralConfig, SystemManifest};
    use std::collections::HashMap;

    let chip = ChipDescriptor {
        schema_version: "1.0".to_string(),
        name: "stm32f103-test".to_string(),
        arch: Arch::Arm,
        flash: MemoryRange { base: 0x0800_0000, size: "64KB".to_string() },
        ram: MemoryRange { base: 0x2000_0000, size: "20KB".to_string() },
        peripherals: vec![PeripheralConfig {
            id: "i2c1".to_string(),
            r#type: "i2c".to_string(),
            base_address: 0x4000_5400,
            size: Some("1KB".to_string()),
            irq: Some(31),
            config: HashMap::new(),
        }],
    };

    let mut config = HashMap::new();
    config.insert("i2c_address".to_string(), serde_yaml::Value::Number(0x53.into()));
    let manifest = SystemManifest {
        schema_version: "1.0".to_string(),
        name: "adxl345-test".to_string(),
        chip: "../chips/stm32f103.yaml".to_string(),
        memory_overrides: HashMap::new(),
        external_devices: vec![ExternalDevice {
            id: "adxl345".to_string(),
            r#type: "adxl345".to_string(),
            connection: "i2c1".to_string(),
            config,
        }],
        board_io: Vec::new(),
        peripherals: Vec::new(),
    };

    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let i2c_idx = bus.find_peripheral_index_by_name("i2c1").unwrap();
    let any = bus.peripherals[i2c_idx].dev.as_any_mut().unwrap();
    let i2c = any.downcast_mut::<crate::peripherals::i2c::I2c>().unwrap();
    assert_eq!(i2c.attached_devices.len(), 1);
}
```

- [ ] **Step 2: Run the failing test**

Run:

```bash
cd core
cargo test -p labwired-core test_from_config_attaches_adxl345_external_device_to_i2c -- --nocapture
```

Expected: FAIL because `attached_devices.len()` is `0`.

- [ ] **Step 3: Implement external device attachment**

In `core/crates/core/src/bus/mod.rs`, after all `PeripheralEntry` values are pushed and before `bus.rebuild_peripheral_ranges();`, add:

```rust
        for ext in &manifest.external_devices {
            let Some(idx) = bus.find_peripheral_index_by_name(&ext.connection) else {
                tracing::warn!(
                    "External device '{}' references missing connection '{}'",
                    ext.id,
                    ext.connection
                );
                continue;
            };

            let Some(any) = bus.peripherals[idx].dev.as_any_mut() else {
                tracing::warn!(
                    "External device '{}' connection '{}' cannot be downcast",
                    ext.id,
                    ext.connection
                );
                continue;
            };

            if let Some(i2c) = any.downcast_mut::<crate::peripherals::i2c::I2c>() {
                let address = ext
                    .config
                    .get("i2c_address")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0x53) as u8;
                match ext.r#type.as_str() {
                    "adxl345" => i2c.attach(Box::new(crate::peripherals::components::Adxl345::new(address))),
                    "mpu6050" => i2c.attach(Box::new(crate::peripherals::components::Mpu6050::new(address))),
                    _ => tracing::warn!(
                        "Unsupported I2C external device type '{}' for '{}'",
                        ext.r#type,
                        ext.id
                    ),
                }
            }
        }
```

Create `core/configs/systems/adxl345-sensor-lab.yaml`:

```yaml
name: "adxl345-sensor-lab"
chip: "../chips/stm32f103.yaml"
external_devices:
  - id: "adxl345"
    type: "adxl345"
    connection: "i2c1"
    config:
      i2c_address: 0x53
board_io:
  - id: "adxl345"
    kind: "i2c_device"
    peripheral: "i2c1"
    pin: 0
    signal: "input"
    active_high: true
    i2c_address: 0x53
    device_type: "adxl345"
```

- [ ] **Step 4: Run the test**

Run:

```bash
cd core
cargo test -p labwired-core test_from_config_attaches_adxl345_external_device_to_i2c -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/crates/core/src/bus/mod.rs core/configs/systems/adxl345-sensor-lab.yaml
git commit -m "feat(core): attach ADXL345 from system config"
```

## Task 3: ADXL345 Firmware Demo and Artifact

**Files:**
- Create: `core/examples/adxl345-sensor-lab/Cargo.toml`
- Create: `core/examples/adxl345-sensor-lab/build.rs`
- Create: `core/examples/adxl345-sensor-lab/memory.x`
- Create: `core/examples/adxl345-sensor-lab/src/main.rs`
- Create: `core/examples/adxl345-sensor-lab/system.yaml`
- Create: `core/examples/adxl345-sensor-lab/README.md`
- Modify: `core/Cargo.toml`
- Add: `packages/playground/public/wasm/demo-adxl345-sensor-lab.elf`

- [ ] **Step 1: Create the firmware crate**

Create `core/examples/adxl345-sensor-lab/Cargo.toml`:

```toml
[package]
name = "adxl345-sensor-lab"
version = "0.1.0"
edition = "2021"

[dependencies]
cortex-m = "0.7"
cortex-m-rt = "0.7"
panic-halt = "0.2"

[[bin]]
name = "adxl345-sensor-lab"
test = false
bench = false
```

Create `core/examples/adxl345-sensor-lab/build.rs`:

```rust
use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    fs::copy("memory.x", out_dir.join("memory.x")).unwrap();
    println!("cargo:rustc-link-search={}", out_dir.display());
    println!("cargo:rustc-link-arg=-Tlink.x");
    println!("cargo:rerun-if-changed=memory.x");
}
```

Create `core/examples/adxl345-sensor-lab/memory.x`:

```ld
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 64K
  RAM : ORIGIN = 0x20000000, LENGTH = 20K
}
```

Create `core/examples/adxl345-sensor-lab/system.yaml`:

```yaml
name: "adxl345-sensor-lab"
chip: "../../configs/chips/stm32f103.yaml"
external_devices:
  - id: "adxl345"
    type: "adxl345"
    connection: "i2c1"
    config:
      i2c_address: 0x53
board_io:
  - id: "adxl345"
    kind: "i2c_device"
    peripheral: "i2c1"
    pin: 0
    signal: "input"
    active_high: true
    i2c_address: 0x53
    device_type: "adxl345"
```

- [ ] **Step 2: Add firmware source**

Create `core/examples/adxl345-sensor-lab/src/main.rs`:

```rust
#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

const I2C1_BASE: u32 = 0x4000_5400;
const UART1_DR: *mut u8 = (0x4001_3800 + 0x04) as *mut u8;

const I2C1_CR1: *mut u32 = (I2C1_BASE + 0x00) as *mut u32;
const I2C1_DR: *mut u32 = (I2C1_BASE + 0x10) as *mut u32;
const I2C1_SR1: *const u32 = (I2C1_BASE + 0x14) as *const u32;

fn uart_byte(byte: u8) {
    unsafe { core::ptr::write_volatile(UART1_DR, byte) }
}

fn uart_str(value: &str) {
    for byte in value.bytes() {
        uart_byte(byte);
    }
}

fn uart_i16(value: i16) {
    if value < 0 {
        uart_byte(b'-');
    }
    let mut n = if value < 0 { value.wrapping_neg() as u16 } else { value as u16 };
    let mut buf = [0u8; 5];
    let mut len = 0;
    loop {
        buf[len] = b'0' + (n % 10) as u8;
        len += 1;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    while len > 0 {
        len -= 1;
        uart_byte(buf[len]);
    }
}

fn i2c_wait(mask: u32) {
    for _ in 0..128 {
        let sr1 = unsafe { core::ptr::read_volatile(I2C1_SR1) };
        if sr1 & mask != 0 {
            return;
        }
    }
}

fn i2c_start() {
    unsafe { core::ptr::write_volatile(I2C1_CR1, 0x0001 | 0x0100) }
    i2c_wait(0x0001);
}

fn i2c_stop() {
    unsafe { core::ptr::write_volatile(I2C1_CR1, 0x0001 | 0x0200) }
}

fn i2c_write(byte: u8) {
    unsafe { core::ptr::write_volatile(I2C1_DR, byte as u32) }
    i2c_wait(0x0080);
}

fn i2c_read_byte() -> u8 {
    i2c_wait(0x0040);
    unsafe { core::ptr::read_volatile(I2C1_DR) as u8 }
}

fn adxl345_read_register(reg: u8) -> u8 {
    i2c_start();
    i2c_write(0xA6);
    i2c_write(reg);
    i2c_start();
    i2c_write(0xA7);
    let value = i2c_read_byte();
    i2c_stop();
    value
}

fn read_axis(lo_reg: u8) -> i16 {
    let lo = adxl345_read_register(lo_reg) as u16;
    let hi = adxl345_read_register(lo_reg + 1) as u16;
    ((hi << 8) | lo) as i16
}

#[entry]
fn main() -> ! {
    uart_str("ADXL345 Sensor Lab\n");
    let devid = adxl345_read_register(0x00);
    uart_str("DEVID=");
    if devid == 0xE5 {
        uart_str("0xE5\n");
    } else {
        uart_str("ERR\n");
    }

    loop {
        let x = read_axis(0x32);
        let y = read_axis(0x34);
        let z = read_axis(0x36);
        uart_str("X=");
        uart_i16(x);
        uart_str(" Y=");
        uart_i16(y);
        uart_str(" Z=");
        uart_i16(z);
        uart_byte(b'\n');
        for _ in 0..200_000 {
            cortex_m::asm::nop();
        }
    }
}
```

- [ ] **Step 3: Add workspace member**

Modify `core/Cargo.toml` members to include:

```toml
    "examples/adxl345-sensor-lab",
```

Add release profile:

```toml
[profile.release.package.adxl345-sensor-lab]
codegen-units = 1
debug = true
opt-level = "s"
```

- [ ] **Step 4: Build and copy the artifact**

Run:

```bash
cd core
cargo build -p adxl345-sensor-lab --release --target thumbv7m-none-eabi
cp target/thumbv7m-none-eabi/release/adxl345-sensor-lab ../packages/playground/public/wasm/demo-adxl345-sensor-lab.elf
```

Expected: build exits `0` and copied file exists.

- [ ] **Step 5: Document the lab**

Create `core/examples/adxl345-sensor-lab/README.md`:

```markdown
# ADXL345 Sensor Lab

This firmware reads the ADXL345 device ID and X/Y/Z acceleration registers through LabWired's simulated I2C1 path on STM32F103.

Run from `core/`:

```bash
cargo build -p adxl345-sensor-lab --release --target thumbv7m-none-eabi
cargo run -q -p labwired-cli -- \
  --firmware target/thumbv7m-none-eabi/release/adxl345-sensor-lab \
  --system examples/adxl345-sensor-lab/system.yaml \
  --max-steps 200000
```

Expected UART begins with:

```text
ADXL345 Sensor Lab
DEVID=0xE5
X=
```
```

- [ ] **Step 6: Commit**

```bash
git add core/Cargo.toml core/examples/adxl345-sensor-lab packages/playground/public/wasm/demo-adxl345-sensor-lab.elf
git commit -m "feat(core): add ADXL345 sensor lab firmware"
```

## Task 4: WASM Bridge Sensor API

**Files:**
- Modify: `core/crates/core/src/peripherals/i2c.rs`
- Modify: `core/crates/wasm/src/lib.rs`
- Modify: `packages/ui/src/wasm/simulator-bridge.ts`

- [ ] **Step 1: Extend I2C device downcasting**

Modify the trait in `core/crates/core/src/peripherals/i2c.rs`:

```rust
pub trait I2cDevice: Send {
    fn address(&self) -> u8;
    fn read(&mut self) -> u8;
    fn write(&mut self, data: u8);
    fn start(&mut self) {}
    fn stop(&mut self) {}
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        None
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        None
    }
}
```

Add to `impl I2cDevice for Adxl345`:

```rust
fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
    Some(self)
}

fn as_any(&self) -> Option<&dyn std::any::Any> {
    Some(self)
}
```

- [ ] **Step 2: Add WASM exports**

In `core/crates/wasm/src/lib.rs`, add methods inside `impl WasmSimulator`:

```rust
#[wasm_bindgen]
pub fn set_i2c_sensor_sample(
    &mut self,
    device_id: &str,
    x: i16,
    y: i16,
    z: i16,
) -> Result<(), JsValue> {
    let binding = self
        .board_io
        .iter()
        .find(|b| b.id == device_id && b.device_type.as_deref() == Some("adxl345"))
        .cloned()
        .ok_or_else(|| JsValue::from_str(&format!("No ADXL345 board_io binding '{}'", device_id)))?;

    let machine = self.machine.as_mut().unwrap();
    let idx = machine
        .bus
        .find_peripheral_index_by_name(&binding.peripheral)
        .ok_or_else(|| JsValue::from_str(&format!("I2C peripheral '{}' not found", binding.peripheral)))?;
    let any = machine.bus.peripherals[idx]
        .dev
        .as_any_mut()
        .ok_or_else(|| JsValue::from_str("I2C peripheral does not support downcasting"))?;
    let i2c = any
        .downcast_mut::<labwired_core::peripherals::i2c::I2c>()
        .ok_or_else(|| JsValue::from_str("Peripheral is not I2C"))?;

    let address = binding.i2c_address.unwrap_or(0x53);
    for device in &mut i2c.attached_devices {
        if device.address() != address {
            continue;
        }
        if let Some(sensor) = device
            .as_any_mut()
            .and_then(|any| any.downcast_mut::<labwired_core::peripherals::components::Adxl345>())
        {
            sensor.set_sample(x, y, z);
            return Ok(());
        }
    }

    Err(JsValue::from_str("ADXL345 device not found on I2C bus"))
}

#[wasm_bindgen]
pub fn get_i2c_sensor_states(&self) -> JsValue {
    let machine = self.machine.as_ref().unwrap();
    let mut states: Vec<serde_json::Value> = Vec::new();

    for binding in &self.board_io {
        if binding.device_type.as_deref() != Some("adxl345") {
            continue;
        }
        let Some(idx) = machine.bus.find_peripheral_index_by_name(&binding.peripheral) else {
            continue;
        };
        let Some(any) = machine.bus.peripherals[idx].dev.as_any() else {
            continue;
        };
        let Some(i2c) = any.downcast_ref::<labwired_core::peripherals::i2c::I2c>() else {
            continue;
        };
        let address = binding.i2c_address.unwrap_or(0x53);
        for device in &i2c.attached_devices {
            if device.address() != address {
                continue;
            }
            if let Some(sensor) = device
                .as_any()
                .and_then(|any| any.downcast_ref::<labwired_core::peripherals::components::Adxl345>())
            {
                let (x, y, z) = sensor.sample();
                states.push(serde_json::json!({
                    "id": binding.id,
                    "kind": "adxl345",
                    "x": x,
                    "y": y,
                    "z": z,
                }));
            }
        }
    }

    serde_wasm_bindgen::to_value(&states).unwrap_or(JsValue::NULL)
}
```

- [ ] **Step 3: Add TypeScript bridge types**

Modify `packages/ui/src/wasm/simulator-bridge.ts`:

```ts
export interface I2cSensorState {
  id: string;
  kind: 'adxl345';
  x: number;
  y: number;
  z: number;
}
```

Add raw methods to `WasmSimulatorInstance`:

```ts
set_i2c_sensor_sample(device_id: string, x: number, y: number, z: number): void;
get_i2c_sensor_states(): I2cSensorState[];
```

Add wrapper methods to `SimulatorBridge`:

```ts
setI2cSensorSample(deviceId: string, x: number, y: number, z: number): void {
  this.sim.set_i2c_sensor_sample(deviceId, x, y, z);
}

getI2cSensorStates(): I2cSensorState[] {
  return this.sim.get_i2c_sensor_states() ?? [];
}
```

- [ ] **Step 4: Build WASM and TypeScript**

Run:

```bash
cd core
cargo test -p labwired-core test_adxl345_devid_and_axis_read -- --nocapture
cargo build -p labwired-wasm --target wasm32-unknown-unknown
cd ../packages/ui
npm test -- --run
```

Expected: Rust test passes, WASM crate builds, UI tests pass.

- [ ] **Step 5: Commit**

```bash
git add core/crates/core/src/peripherals/i2c.rs core/crates/wasm/src/lib.rs packages/ui/src/wasm/simulator-bridge.ts
git commit -m "feat(wasm): expose ADXL345 sensor input"
```

## Task 5: Reusable Sensor Lab UI Components

**Files:**
- Create: `packages/ui/src/components/Adxl345Visualizer/Adxl345Visualizer.tsx`
- Create: `packages/ui/src/components/GuidedLab/GuidedLab.tsx`
- Modify: `packages/ui/src/index.ts`

- [ ] **Step 1: Create ADXL345 visualizer**

Create `packages/ui/src/components/Adxl345Visualizer/Adxl345Visualizer.tsx`:

```tsx
export interface Adxl345Sample {
  x: number;
  y: number;
  z: number;
}

export interface Adxl345VisualizerProps {
  sample: Adxl345Sample;
  history: Adxl345Sample[];
  onSampleChange: (sample: Adxl345Sample) => void;
}

function clampSample(value: number): number {
  return Math.max(-512, Math.min(512, Math.round(value)));
}

export function Adxl345Visualizer({ sample, history, onSampleChange }: Adxl345VisualizerProps) {
  const points = history.slice(-40);
  const width = 280;
  const height = 96;
  const line = (axis: keyof Adxl345Sample) => points
    .map((point, index) => {
      const x = points.length <= 1 ? 0 : (index / (points.length - 1)) * width;
      const y = height / 2 - (point[axis] / 512) * (height / 2 - 8);
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(' ');

  return (
    <div className="adxl345-visualizer">
      <div className="adxl345-board" aria-label="ADXL345 breakout board">
        <div className="adxl345-chip">ADXL345</div>
        <div className="adxl345-pins">VCC GND SDA SCL</div>
      </div>
      <div className="adxl345-controls">
        {(['x', 'y', 'z'] as const).map((axis) => (
          <label key={axis} className="axis-control">
            <span>{axis.toUpperCase()}</span>
            <input
              type="range"
              min="-512"
              max="512"
              value={sample[axis]}
              onChange={(event) => onSampleChange({ ...sample, [axis]: clampSample(Number(event.target.value)) })}
            />
            <output>{sample[axis]}</output>
          </label>
        ))}
      </div>
      <svg className="adxl345-chart" viewBox={`0 0 ${width} ${height}`} role="img" aria-label="Acceleration chart">
        <polyline points={line('x')} fill="none" stroke="#e83e8c" strokeWidth="2" />
        <polyline points={line('y')} fill="none" stroke="#27c93f" strokeWidth="2" />
        <polyline points={line('z')} fill="none" stroke="#569cd6" strokeWidth="2" />
      </svg>
    </div>
  );
}
```

- [ ] **Step 2: Create Guided Lab shell**

Create `packages/ui/src/components/GuidedLab/GuidedLab.tsx`:

```tsx
import type { ReactNode } from 'react';

export interface GuidedLabStep {
  id: string;
  label: string;
  status: 'done' | 'active' | 'pending';
}

export interface GuidedLabProps {
  title: string;
  subtitle: string;
  steps: GuidedLabStep[];
  stage: ReactNode;
  sensor: ReactNode;
  serial: ReactNode;
  advanced: ReactNode;
  advancedOpen: boolean;
  onToggleAdvanced: () => void;
}

export function GuidedLab({
  title,
  subtitle,
  steps,
  stage,
  sensor,
  serial,
  advanced,
  advancedOpen,
  onToggleAdvanced,
}: GuidedLabProps) {
  return (
    <section className="guided-lab">
      <aside className="guided-lab-rail">
        <div>
          <h2>{title}</h2>
          <p>{subtitle}</p>
        </div>
        <ol>
          {steps.map((step) => (
            <li key={step.id} className={`guided-step ${step.status}`}>
              <span>{step.label}</span>
            </li>
          ))}
        </ol>
      </aside>
      <main className="guided-lab-stage">{stage}</main>
      <aside className="guided-lab-inspector">
        {sensor}
        {serial}
        <button className="guided-advanced-toggle" type="button" onClick={onToggleAdvanced}>
          {advancedOpen ? 'Hide Advanced' : 'Show Advanced'}
        </button>
      </aside>
      {advancedOpen && <div className="guided-lab-advanced">{advanced}</div>}
    </section>
  );
}
```

- [ ] **Step 3: Export components**

Modify `packages/ui/src/index.ts`:

```ts
export { Adxl345Visualizer } from './components/Adxl345Visualizer/Adxl345Visualizer';
export type { Adxl345Sample, Adxl345VisualizerProps } from './components/Adxl345Visualizer/Adxl345Visualizer';
export { GuidedLab } from './components/GuidedLab/GuidedLab';
export type { GuidedLabProps, GuidedLabStep } from './components/GuidedLab/GuidedLab';
export type { I2cSensorState } from './wasm/simulator-bridge';
```

- [ ] **Step 4: Run UI tests**

Run:

```bash
cd packages/ui
npm test -- --run
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/ui/src/components/Adxl345Visualizer packages/ui/src/components/GuidedLab packages/ui/src/index.ts
git commit -m "feat(ui): add guided ADXL345 lab components"
```

## Task 6: Editor ADXL345 Component and Bundled Config

**Files:**
- Create: `packages/ui/src/editor/components/adxl345.tsx`
- Modify: `packages/ui/src/editor/components/index.ts`
- Modify: `packages/playground/src/bundled-configs.ts`
- Modify: `packages/playground/src/bundled-configs.test.ts`

- [ ] **Step 1: Add the editor component**

Create `packages/ui/src/editor/components/adxl345.tsx`:

```tsx
import type { ComponentDef } from '../types';

export const adxl345Component: ComponentDef = {
  type: 'adxl345',
  label: 'ADXL345',
  category: 'sensor',
  width: 96,
  height: 64,
  boardIoKind: 'i2c_device',
  pins: [
    { id: 'VCC', x: 0, y: 14, side: 'left' },
    { id: 'GND', x: 0, y: 30, side: 'left' },
    { id: 'SDA', x: 96, y: 22, side: 'right' },
    { id: 'SCL', x: 96, y: 42, side: 'right' },
  ],
  defaultAttrs: {},
  render: (_attrs, state) => (
    <g>
      <rect width="96" height="64" rx="6" fill={state?.selected ? '#fff7fb' : '#f8f9fa'} stroke="#111" strokeWidth="2" />
      <rect x="25" y="18" width="46" height="28" rx="3" fill="#111" />
      <text x="48" y="35" textAnchor="middle" fontSize="9" fill="#fff" fontFamily="monospace">ADXL345</text>
      <circle cx="10" cy="14" r="3" fill="#e83e8c" />
      <circle cx="10" cy="30" r="3" fill="#444" />
      <circle cx="86" cy="22" r="3" fill="#27c93f" />
      <circle cx="86" cy="42" r="3" fill="#569cd6" />
    </g>
  ),
};
```

Modify `packages/ui/src/editor/components/index.ts` to import and register `adxl345Component`.

- [ ] **Step 2: Add bundled config**

Modify `packages/playground/src/bundled-configs.ts`:

```ts
import systemAdxl345SensorLab from '../../../core/examples/adxl345-sensor-lab/system.yaml?raw';
```

Add to `BOARD_CONFIGS` before generic boards:

```ts
{
  boardId: 'adxl345-sensor-lab',
  chipId: 'stm32f103',
  name: 'ADXL345 Sensor Lab',
  description: 'Guided STM32F103 + ADXL345 accelerometer lab over simulated I2C.',
  arch: 'ARM Cortex-M3',
  chipYaml: chipStm32f103,
  systemYaml: systemAdxl345SensorLab,
  demoFirmwarePath: `${BASE}wasm/demo-adxl345-sensor-lab.elf`,
  mcuComponentType: 'stm32-dev',
},
```

- [ ] **Step 3: Test bundled config**

Modify `packages/playground/src/bundled-configs.test.ts`:

```ts
const adxl345 = BOARD_CONFIGS.find((config) => config.boardId === 'adxl345-sensor-lab');
expect(adxl345).toBeDefined();
expect(adxl345?.systemYaml).toContain('type: "adxl345"');
expect(adxl345?.systemYaml).toContain('kind: "i2c_device"');
expect(adxl345?.demoFirmwarePath).toContain('demo-adxl345-sensor-lab.elf');
```

- [ ] **Step 4: Run playground tests**

Run:

```bash
cd packages/playground
npm test -- --run
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/ui/src/editor/components/adxl345.tsx packages/ui/src/editor/components/index.ts packages/playground/src/bundled-configs.ts packages/playground/src/bundled-configs.test.ts
git commit -m "feat(playground): bundle ADXL345 sensor lab config"
```

## Task 7: Guided Playground Experience

**Files:**
- Modify: `packages/playground/src/App.tsx`
- Modify: `packages/playground/src/playground.css`

- [ ] **Step 1: Add ADXL345 starter diagram**

In `makeStarterDiagram` in `packages/playground/src/App.tsx`, add:

```ts
if (config.boardId === 'adxl345-sensor-lab') {
  return {
    ...createEmptyDiagram(config.chipId),
    parts: [
      mcu,
      { id: 'adxl345', type: 'adxl345', x: 390, y: 80, rotate: 0, scale: 1.4, attrs: {} },
    ],
    wires: [
      { from: { part: 'mcu', pin: 'PB7' }, to: { part: 'adxl345', pin: 'SDA' }, color: '#27c93f' },
      { from: { part: 'mcu', pin: 'PB6' }, to: { part: 'adxl345', pin: 'SCL' }, color: '#569cd6' },
    ],
  };
}
```

- [ ] **Step 2: Add guided state**

Import from `@labwired/ui`:

```ts
Adxl345Visualizer,
GuidedLab,
type Adxl345Sample,
type GuidedLabStep,
```

Add state near the existing UI state:

```ts
const [advancedOpen, setAdvancedOpen] = useState(false);
const [adxlSample, setAdxlSample] = useState<Adxl345Sample>({ x: 0, y: 0, z: 256 });
const [adxlHistory, setAdxlHistory] = useState<Adxl345Sample[]>([]);
const isAdxl345Lab = selectedBoard.boardId === 'adxl345-sensor-lab';
```

Add effect:

```ts
useEffect(() => {
  if (!bridge || !isAdxl345Lab) return;
  bridge.setI2cSensorSample('adxl345', adxlSample.x, adxlSample.y, adxlSample.z);
  setAdxlHistory((prev) => [...prev.slice(-79), adxlSample]);
}, [bridge, isAdxl345Lab, adxlSample]);
```

- [ ] **Step 3: Render Guided Stage for ADXL345**

Before the existing unified layout return branch, create:

```tsx
const guidedSteps: GuidedLabStep[] = [
  { id: 'wire', label: 'Wire check', status: canvasValidationMessage ? 'active' : 'done' },
  { id: 'run', label: 'Upload and run firmware', status: simActive ? 'done' : 'active' },
  { id: 'watch', label: 'Watch acceleration', status: simActive ? 'active' : 'pending' },
  { id: 'share', label: 'Share or embed', status: simActive ? 'pending' : 'pending' },
];
```

Inside `<div className="editor-layout">`, branch the center content:

```tsx
{isAdxl345Lab ? (
  <GuidedLab
    title="ADXL345 Sensor Lab"
    subtitle="Tilt the simulated accelerometer and watch real firmware read it over I2C."
    steps={guidedSteps}
    stage={(
      <EditorCanvas
        state={editor.state}
        boardIoStates={boardIoStateMap}
        validationMessage={canvasValidationMessage}
        invalidPins={invalidPins}
        onMovePart={editor.movePart}
        onResizePart={editor.resizePart}
        onSelect={editor.select}
        onSelectRect={editor.selectRect}
        onStartWire={handleStartWire}
        onCompleteWire={handleCompleteWire}
        onCancelWire={handleCancelWire}
        onDeleteWire={editor.deleteWire}
        onDropPart={handleDropPart}
        onButtonToggle={handleButtonToggle}
        onAnalogChange={handleAnalogChange}
      />
    )}
    sensor={<Adxl345Visualizer sample={adxlSample} history={adxlHistory} onSampleChange={setAdxlSample} />}
    serial={<SerialMonitor output={simState.uartOutput} onClear={clearUart} style={{ height: 180 }} />}
    advanced={(
      <div className="guided-advanced-grid">
        <RegisterGrid registers={registers} />
        <InstructionTrace entries={traceEntries} />
        <MemoryInspector data={stackMemory} baseAddress={stackBase} />
        <pre>{selectedBoard.systemYaml}</pre>
      </div>
    )}
    advancedOpen={advancedOpen}
    onToggleAdvanced={() => setAdvancedOpen((open) => !open)}
  />
) : (
  /* existing editor-center content */
)}
```

Keep the existing non-ADXL345 workbench path intact.

- [ ] **Step 4: Add CSS**

Add to `packages/playground/src/playground.css`:

```css
.guided-lab {
  display: grid;
  grid-template-columns: 240px minmax(0, 1fr) 320px;
  grid-template-rows: minmax(0, 1fr) auto;
  gap: 12px;
  width: 100%;
  height: 100%;
  padding: 12px;
  background: #f8f9fa;
  color: #111;
}

.guided-lab-rail,
.guided-lab-inspector,
.guided-lab-advanced {
  border: 2px solid #111;
  border-radius: 8px;
  background: #fff;
  box-shadow: 4px 4px 0 #111;
  padding: 14px;
}

.guided-lab-stage {
  min-width: 0;
  min-height: 0;
  border: 2px solid #111;
  border-radius: 8px;
  overflow: hidden;
  background: #181825;
  box-shadow: 4px 4px 0 #111;
}

.guided-lab-rail h2 {
  font-size: 1.35rem;
  margin-bottom: 6px;
}

.guided-lab-rail p {
  color: #444;
  line-height: 1.35;
}

.guided-lab-rail ol {
  margin-top: 18px;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.guided-step {
  border: 1px solid #111;
  border-radius: 6px;
  padding: 8px;
  font-weight: 700;
}

.guided-step.done { background: #dff8e4; }
.guided-step.active { background: #ffe4f0; }
.guided-step.pending { background: #f1f3f5; color: #555; }

.adxl345-visualizer {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.adxl345-board {
  border: 2px solid #111;
  border-radius: 8px;
  background: #f8f9fa;
  padding: 16px;
}

.adxl345-chip {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 120px;
  height: 52px;
  background: #111;
  color: #fff;
  font-family: var(--lw-font-mono);
  font-weight: 700;
}

.adxl345-pins {
  margin-top: 10px;
  font-family: var(--lw-font-mono);
  color: #444;
}

.axis-control {
  display: grid;
  grid-template-columns: 24px 1fr 48px;
  align-items: center;
  gap: 8px;
  font-family: var(--lw-font-mono);
}

.adxl345-chart {
  width: 100%;
  height: 96px;
  border: 1px solid #111;
  border-radius: 6px;
  background: #fff;
}

.guided-advanced-toggle {
  width: 100%;
  border: 2px solid #111;
  background: #111;
  color: #fff;
  border-radius: 6px;
  padding: 9px 10px;
  font-weight: 700;
  cursor: pointer;
}

.guided-lab-advanced {
  grid-column: 1 / -1;
  max-height: 260px;
  overflow: auto;
}

.guided-advanced-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(220px, 1fr));
  gap: 12px;
}

@media (max-width: 900px) {
  .guided-lab {
    grid-template-columns: 1fr;
    grid-template-rows: auto minmax(360px, 1fr) auto auto;
  }
}
```

- [ ] **Step 5: Run tests**

Run:

```bash
cd packages/playground
npm test -- --run
npm run build
```

Expected: tests pass and Vite build completes.

- [ ] **Step 6: Commit**

```bash
git add packages/playground/src/App.tsx packages/playground/src/playground.css
git commit -m "feat(playground): add guided ADXL345 sensor lab"
```

## Task 8: End-to-End Verification and Polish

**Files:**
- Modify: `docs/ops/VS_CODE_UI_DEMO_CHECKLIST.md`

- [ ] **Step 1: Run core validation**

Run:

```bash
cd core
cargo test -p labwired-core test_adxl345_devid_and_axis_read -- --nocapture
cargo test -p labwired-core test_from_config_attaches_adxl345_external_device_to_i2c -- --nocapture
cargo build -p adxl345-sensor-lab --release --target thumbv7m-none-eabi
cargo run -q -p labwired-cli -- \
  --firmware target/thumbv7m-none-eabi/release/adxl345-sensor-lab \
  --system examples/adxl345-sensor-lab/system.yaml \
  --max-steps 200000
```

Expected: tests pass, firmware builds, CLI UART includes `ADXL345 Sensor Lab` and `DEVID=0xE5`.

- [ ] **Step 2: Run UI validation**

Run:

```bash
cd packages/ui
npm test -- --run
npm run build
cd ../playground
npm test -- --run
npm run build
```

Expected: tests and builds pass.

- [ ] **Step 3: Manual browser validation**

Run:

```bash
cd packages/playground
npm run dev -- --host 127.0.0.1
```

Open the printed Vite URL and verify:

- ADXL345 Sensor Lab appears as a board option.
- Selecting it loads the guided layout.
- Run Demo starts the real bundled ELF.
- Serial output includes `ADXL345 Sensor Lab` and axis lines.
- Moving X/Y/Z sliders changes the chart and subsequent serial output.
- Show Advanced reveals registers, trace, memory, and system YAML.
- Narrow viewport below 900px stacks the layout without text overlap.
- Embed URL keeps the lab usable without the full workbench chrome.

- [ ] **Step 4: Update checklist with the public demo flow**

Append this section to `docs/ops/VS_CODE_UI_DEMO_CHECKLIST.md`:

```markdown
## ADXL345 Sensor Lab Playground

- Open the playground Vite URL.
- Select `ADXL345 Sensor Lab`.
- Run the bundled demo.
- Confirm serial output starts with `ADXL345 Sensor Lab` and `DEVID=0xE5`.
- Move X/Y/Z controls and confirm the chart updates.
- Open Advanced and confirm registers, trace, memory, and system YAML are visible.
- Open embed mode and confirm the guided lab remains usable.
```

- [ ] **Step 5: Final commit**

```bash
git add docs/ops/VS_CODE_UI_DEMO_CHECKLIST.md
git commit -m "docs: document ADXL345 sensor lab validation"
```

## Self-Review

- Spec coverage: The plan covers the real ADXL345 simulator model, guided playground UI, live X/Y/Z display, serial output, advanced drawer, share/embed compatibility through existing sharing paths, and KiCad as a non-goal.
- Placeholder scan: The plan contains no incomplete markers or unspecified test steps.
- Type consistency: The new sensor bridge uses `I2cSensorState`, `Adxl345Sample`, `setI2cSensorSample`, and `getI2cSensorStates` consistently across Rust WASM and TypeScript wrapper layers.
