# SPI L3 Declarative Primitive — Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a declarative `spi_device` primitive — a datasheet-shaped `behavior.spi` descriptor interpreted by a generic engine — so a register-style SPI sensor becomes a YAML file with zero per-part Rust, byte-compatible with the hand-written reference family.

**Architecture:** Mirror the just-landed I²C L3 trio (`declarative_i2c.rs`). Add a `behavior.spi` schema to `labwired_config`, reusing the measurement→word machinery (`Encode`/`ScaleFrom`/`Endian`) that is already protocol-agnostic; extract those shared pure helpers into a new `declarative_regs.rs` so both engines share one definition. Add a `GenericSpiDevice` (impl `SpiDevice` + `SimInput`) whose only new logic is the SPI **CS-framed command/register byte state machine**, plus a `DeclarativeSpiKit`. Phase 1 registers **no real parts** — a test-only fixture drives the unit tests, so the offline peripherals manifest is unchanged.

**Tech Stack:** Rust (workspace crates `labwired_config`, `labwired-core`), `serde` / `serde_yaml`, `anyhow`. Tests are inline `#[cfg(test)]` modules run with `cargo test`.

## Global Constraints

- **License header** on every new `.rs` file, verbatim from existing files:
  ```
  // LabWired - Firmware Simulation Platform
  // Copyright (C) 2026 Andrii Shylenko
  // SPDX-License-Identifier: MIT
  ```
- **Manifest must stay unchanged** in Phase 1: do NOT add any entry to `registry::KITS`. The `DeclarativeSpiKit` machinery ships, but no real part is registered.
- **Struct reuse via rename + alias:** rename shared schema types to protocol-neutral names and keep `pub type` aliases so all existing I²C code and YAML compile unchanged. The existing `declarative_i2c` test suite is the regression guard — it MUST stay green after every task.
- **No new dependencies.** Everything is already in the workspace.
- **TDD:** write the failing test first, watch it fail, implement minimally, watch it pass, commit.
- **Commit style:** conventional-commit subject, no AI/Claude references or trailers.
- Work in the worktree `../labwired-core-spi-l3` on branch `feat/spi-l3-declarative` (already created off latest `main` @ `92d8a8cb`).

**Reference files to read before starting (do not modify except where a task says so):**
- `crates/core/src/peripherals/components/declarative_i2c.rs` — the engine being mirrored.
- `crates/core/src/peripherals/components/max7219.rs` — a hand-written `SpiDevice` + `PeripheralKit` (the CS-framed shift model, `attach_spi_device`, kit metadata shape).
- `crates/core/src/peripherals/spi.rs` — the `SpiDevice` trait.
- `crates/config/src/lib.rs` — schema (structs `DeviceBehavior`, `I2cRegister`, `Encode`, `ScaleFrom`, `Endian`, `I2cAccess`, `I2cSpec`; helper `one_f64`).

---

## File Structure

| Path | Responsibility | Task |
|------|----------------|------|
| `crates/config/src/lib.rs` | Rename shared register structs → neutral names + aliases; add `SpiFraming`, `SpiSpec`, `DeviceBehavior.spi`. | 1 |
| `crates/core/src/peripherals/components/declarative_regs.rs` (new) | Shared pure helpers: `width_max`, `encode_raw`, `pack`, `unpack`, `register_read_bytes`, `scale_from_factor`. One home for both engines. | 2 |
| `crates/core/src/peripherals/components/declarative_i2c.rs` | Switch its private helpers to the shared module (behaviour-preserving; I²C tests guard it). | 2 |
| `crates/core/src/peripherals/components/declarative_spi.rs` (new) | `GenericSpiDevice` (SPI CS-framed engine, impl `SpiDevice` + `SimInput`) + `DeclarativeSpiKit`. | 3, 4 |
| `crates/core/src/peripherals/components/declarative_spi_fixture.yaml` (new) | Test-only register-style SPI part exercising every framing field. Not registered. | 3 |
| `crates/core/src/peripherals/components/mod.rs` | `pub mod` + `pub use` the two new modules. | 2, 4 |

---

## Task 1: Config schema — neutral register structs + SPI spec

**Files:**
- Modify: `crates/config/src/lib.rs`

**Interfaces:**
- Consumes: existing `Encode`, `ScaleFrom`, `Endian`, `I2cRegister`, `I2cAccess`, `DeviceBehavior`.
- Produces:
  - `pub struct RegisterSpec { name: String, addr: u8, width: u8, endian: Endian, access: RegisterAccess, reset: u32, source: Option<String>, encode: Option<Encode>, scale_from: Option<ScaleFrom> }` (was `I2cRegister`)
  - `pub enum RegisterAccess { R, Rw }` (was `I2cAccess`)
  - `pub type I2cRegister = RegisterSpec;` and `pub type I2cAccess = RegisterAccess;`
  - `pub struct SpiFraming { command_bytes: u8, rw_bit: Option<u8>, rw_read_high: bool, addr_mask: u8, addr_shift: u8, auto_increment: bool }` with `Default`.
  - `pub struct SpiSpec { framing: SpiFraming, registers: Vec<RegisterSpec> }`
  - `DeviceBehavior.spi: Option<SpiSpec>`

- [ ] **Step 1: Write the failing test** — append to the existing `#[cfg(test)] mod tests` in `crates/config/src/lib.rs` (or add one if none):

```rust
#[test]
fn spi_device_descriptor_parses_framing_and_registers() {
    let yaml = r#"
type: test_spi
behavior:
  primitive: spi_device
  spi:
    framing: { command_bytes: 1, rw_bit: 7, rw_read_high: true, addr_mask: 0x3F, auto_increment: true }
    registers:
      - { name: WHOAMI, addr: 0x00, width: 1, endian: le, access: r, reset: 0xE5 }
      - { name: DATA, addr: 0x32, width: 2, endian: le, access: r, source: accel }
metadata:
  inputs:
    - { key: accel, label: "Accel X", unit: g, min: -16, max: 16, default: 0 }
"#;
    let d = DeviceDescriptor::from_yaml(yaml).unwrap();
    let spi = d.behavior.spi.as_ref().expect("behavior.spi present");
    assert_eq!(spi.framing.command_bytes, 1);
    assert_eq!(spi.framing.rw_bit, Some(7));
    assert_eq!(spi.registers.len(), 2);
    assert_eq!(spi.registers[0].reset, 0xE5);
}

#[test]
fn spi_framing_defaults_are_adxl_shaped() {
    let f = SpiFraming::default();
    assert_eq!(f.command_bytes, 1);
    assert_eq!(f.rw_bit, Some(7));
    assert!(f.rw_read_high);
    assert_eq!(f.addr_mask, 0x3F);
    assert_eq!(f.addr_shift, 0);
    assert!(f.auto_increment);
}

#[test]
fn i2c_register_alias_still_names_the_shared_struct() {
    // The rename must not break existing I2c-named references.
    let _r: I2cRegister = RegisterSpec {
        name: "R".into(), addr: 0, width: 1, endian: Endian::Le,
        access: I2cAccess::R, reset: 0, source: None, encode: None, scale_from: None,
    };
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p labwired-config spi_ 2>&1 | tail -20`
Expected: FAIL — `SpiFraming` / `RegisterSpec` / `behavior.spi` do not exist (compile error).

- [ ] **Step 3: Rename the shared structs and add aliases**

In `crates/config/src/lib.rs`, rename the struct `I2cRegister` → `RegisterSpec` and enum `I2cAccess` → `RegisterAccess` (rename the definitions and their doc comments; leave `Encode`, `ScaleFrom`, `Endian` names as-is — already neutral). Update the field type on `I2cSpec.registers` to `Vec<RegisterSpec>` and any internal reference. Immediately after the `RegisterAccess` enum definition, add the aliases:

```rust
/// Back-compat aliases: the register/access structs were originally I²C-named.
/// Both declarative engines (I²C register-pointer, SPI CS-framed) share one
/// definition; these keep every existing `I2c*` reference compiling.
pub type I2cRegister = RegisterSpec;
pub type I2cAccess = RegisterAccess;
```

- [ ] **Step 4: Add the SPI schema types**

After the `I2cSpec` block in `crates/config/src/lib.rs`, add:

```rust
/// The `behavior.spi` section of a declarative `spi_device` — datasheet-shaped
/// wire framing for a register-style SPI sensor, interpreted by the engine's
/// generic device. The measurement→word machinery (`endian`/`source`/`encode`/
/// `scale_from` on each [`RegisterSpec`]) is shared verbatim with the I²C
/// primitive; only the leading-command framing differs.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SpiSpec {
    /// How the leading command byte encodes read/write + register address.
    #[serde(default)]
    pub framing: SpiFraming,
    /// Register map addressed by the command byte.
    #[serde(default)]
    pub registers: Vec<RegisterSpec>,
}

/// SPI command-byte framing. Defaults are the ADXL345 convention: one command
/// byte, bit 7 = read/write, bits [5:0] = register address, multi-byte bursts
/// auto-increment the address.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SpiFraming {
    /// Width of the leading command word in bytes. `0` = read-only part with no
    /// command (MAX31855: CS↓, clock out register 0); `1` = ADXL345-style.
    #[serde(default = "default_command_bytes")]
    pub command_bytes: u8,
    /// Bit position in the command byte selecting read vs write. `None` ⇒
    /// direction is fixed by each register's `access`.
    #[serde(default = "default_rw_bit")]
    pub rw_bit: Option<u8>,
    /// If true, `rw_bit` set (=1) means READ (ADXL345). If false, set means write.
    #[serde(default = "default_true")]
    pub rw_read_high: bool,
    /// Mask applied (after `addr_shift`) to the command byte to get the address.
    #[serde(default = "default_addr_mask")]
    pub addr_mask: u8,
    #[serde(default)]
    pub addr_shift: u8,
    /// A multi-byte burst walks ascending register addresses from the selected
    /// one; false ⇒ only the selected register is served.
    #[serde(default = "default_true")]
    pub auto_increment: bool,
}

impl Default for SpiFraming {
    fn default() -> Self {
        Self {
            command_bytes: default_command_bytes(),
            rw_bit: default_rw_bit(),
            rw_read_high: true,
            addr_mask: default_addr_mask(),
            addr_shift: 0,
            auto_increment: true,
        }
    }
}

fn default_command_bytes() -> u8 {
    1
}
fn default_rw_bit() -> Option<u8> {
    Some(7)
}
fn default_true() -> bool {
    true
}
fn default_addr_mask() -> u8 {
    0x3F
}
```

Then add the field to `DeviceBehavior` (right after `pub i2c: Option<I2cSpec>,`):

```rust
    /// For the `spi_device` primitive: the datasheet-shaped SPI wire framing the
    /// engine's generic SPI device interprets. Absent for non-SPI primitives.
    #[serde(default)]
    pub spi: Option<SpiSpec>,
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p labwired-config 2>&1 | tail -20`
Expected: PASS — the three new tests pass and every pre-existing `labwired-config` test still passes (the aliases keep old references compiling).

- [ ] **Step 6: Commit**

```bash
git add crates/config/src/lib.rs
git commit -m "feat(config): neutral RegisterSpec + declarative behavior.spi schema"
```

---

## Task 2: Extract shared register helpers into `declarative_regs.rs`

**Files:**
- Create: `crates/core/src/peripherals/components/declarative_regs.rs`
- Modify: `crates/core/src/peripherals/components/declarative_i2c.rs` (switch to the shared helpers)
- Modify: `crates/core/src/peripherals/components/mod.rs` (`pub mod declarative_regs;`)

**Interfaces:**
- Produces (all `pub(crate)`):
  - `fn width_max(width: u8) -> f64`
  - `fn encode_raw(value: f64, enc: Option<&Encode>, extra_scale: f64, width: u8) -> u32`
  - `fn pack(raw: u32, width: u8, endian: Endian) -> Vec<u8>`
  - `fn unpack(bytes: &[u8], endian: Endian) -> u32`
  - `fn scale_from_factor(reg: &RegisterSpec, reg_values: &HashMap<String, u32>) -> f64`
  - `fn register_read_bytes(reg: &RegisterSpec, slots: &HashMap<String, f64>, reg_values: &HashMap<String, u32>) -> Vec<u8>`
- Consumes: `labwired_config::{Encode, Endian, RegisterSpec}`.

Note: `crc8` stays in `declarative_i2c.rs` (SPI Phase 1 needs no CRC). Only the register/encode helpers move.

- [ ] **Step 1: Write the failing test** — create `crates/core/src/peripherals/components/declarative_regs.rs` with the license header, then a test module referencing the not-yet-written functions:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use labwired_config::{Endian, RegisterAccess, RegisterSpec};
    use std::collections::HashMap;

    fn reg(name: &str, addr: u8, width: u8, endian: Endian, source: Option<&str>) -> RegisterSpec {
        RegisterSpec {
            name: name.into(), addr, width, endian, access: RegisterAccess::R,
            reset: 0, source: source.map(Into::into), encode: None, scale_from: None,
        }
    }

    #[test]
    fn pack_unpack_round_trip_le_and_be() {
        assert_eq!(pack(0x1234, 2, Endian::Le), vec![0x34, 0x12]);
        assert_eq!(pack(0x1234, 2, Endian::Be), vec![0x12, 0x34]);
        assert_eq!(unpack(&[0x34, 0x12], Endian::Le), 0x1234);
        assert_eq!(unpack(&[0x12, 0x34], Endian::Be), 0x1234);
    }

    #[test]
    fn register_read_bytes_sources_and_packs() {
        let r = reg("DATA", 0x32, 2, Endian::Le, Some("accel"));
        let mut slots = HashMap::new();
        slots.insert("accel".to_string(), 100.0);
        let b = register_read_bytes(&r, &slots, &HashMap::new());
        assert_eq!(b, vec![100, 0]); // 100 LE, scale 1
    }

    #[test]
    fn storage_register_echoes_reg_value() {
        let r = reg("CTRL", 0x2D, 1, Endian::Le, None);
        let mut regs = HashMap::new();
        regs.insert("CTRL".to_string(), 0x08u32);
        assert_eq!(register_read_bytes(&r, &HashMap::new(), &regs), vec![0x08]);
    }
}
```

- [ ] **Step 2: Write the shared helpers** (above the test module). Copy the bodies from `declarative_i2c.rs` verbatim (`width_max`, `encode_raw`, `pack`, `unpack`) and lift `scale_from_factor` / `register_read_bytes` out of `impl GenericI2cDevice` into free functions taking the maps explicitly:

```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Shared register/measurement helpers for the declarative device engines.
//! The I²C (register-pointer) and SPI (CS-framed) primitives address registers
//! differently but pack the SAME datasheet-shaped word: a `source` measurement
//! run through a linear `encode` (+ optional bit-field `scale_from`), or a
//! plain storage register echoing its written value. This module is the one
//! home for that math so both engines stay byte-identical.

use std::collections::HashMap;

use labwired_config::{Encode, Endian, RegisterSpec};

/// Largest value representable in `width` bytes, as f64 (width ≤ 4).
pub(crate) fn width_max(width: u8) -> f64 {
    ((1u64 << (8 * width as u64)) - 1) as f64
}

/// Apply a linear encode (scale/offset/clamp) plus an extra scale factor,
/// yielding the raw integer packed into a `width`-byte word.
pub(crate) fn encode_raw(value: f64, enc: Option<&Encode>, extra_scale: f64, width: u8) -> u32 {
    let scale = enc.map(|e| e.scale).unwrap_or(1.0) * extra_scale;
    let offset = enc.map(|e| e.offset).unwrap_or(0.0);
    let mut raw = value * scale + offset;
    if let Some(e) = enc {
        if let Some(lo) = e.clamp_min {
            raw = raw.max(lo);
        }
        if let Some(hi) = e.clamp_max {
            raw = raw.min(hi);
        }
    }
    raw.round().clamp(0.0, width_max(width)) as u32
}

/// Pack `raw` into `width` bytes in the given order.
pub(crate) fn pack(raw: u32, width: u8, endian: Endian) -> Vec<u8> {
    let mut le: Vec<u8> = (0..width).map(|i| (raw >> (8 * i as u32)) as u8).collect();
    if endian == Endian::Be {
        le.reverse();
    }
    le
}

/// Unpack `width` bytes (in `endian` order) into a value.
pub(crate) fn unpack(bytes: &[u8], endian: Endian) -> u32 {
    let mut acc = 0u32;
    match endian {
        Endian::Le => {
            for (i, &b) in bytes.iter().enumerate() {
                acc |= (b as u32) << (8 * i as u32);
            }
        }
        Endian::Be => {
            for &b in bytes {
                acc = (acc << 8) | b as u32;
            }
        }
    }
    acc
}

/// The extra scale factor a register's `scale_from` selects from the current
/// value of another register's bit-field (1.0 when absent / unmapped).
pub(crate) fn scale_from_factor(reg: &RegisterSpec, reg_values: &HashMap<String, u32>) -> f64 {
    let Some(sf) = &reg.scale_from else {
        return 1.0;
    };
    let regval = reg_values.get(&sf.register).copied().unwrap_or(0);
    let field = (regval >> sf.shift as u32) & sf.mask;
    sf.map.get(&field).copied().unwrap_or(1.0)
}

/// The bytes a read of `reg` returns: a sourced+encoded measurement, or the
/// plain stored value (seeded to reset) for a storage register.
pub(crate) fn register_read_bytes(
    reg: &RegisterSpec,
    slots: &HashMap<String, f64>,
    reg_values: &HashMap<String, u32>,
) -> Vec<u8> {
    let raw = if let Some(src) = &reg.source {
        let value = slots.get(src).copied().unwrap_or(0.0);
        let extra = scale_from_factor(reg, reg_values);
        encode_raw(value, reg.encode.as_ref(), extra, reg.width)
    } else {
        reg_values.get(&reg.name).copied().unwrap_or(reg.reset)
    };
    pack(raw, reg.width, reg.endian)
}
```

- [ ] **Step 3: Register the module** — in `crates/core/src/peripherals/components/mod.rs`, add near the other `pub mod` lines:

```rust
pub mod declarative_regs;
```

- [ ] **Step 4: Run the new tests**

Run: `cargo test -p labwired-core declarative_regs 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Switch `declarative_i2c.rs` onto the shared helpers**

In `declarative_i2c.rs`: delete the private `width_max`, `encode_raw`, `pack`, `unpack` free functions and the `scale_from_factor` / `register_read_bytes` methods on `impl GenericI2cDevice`. Add `use super::declarative_regs::{encode_raw, pack, unpack, register_read_bytes};` (keep the local `crc8`). Replace the two former-method call sites:
- in `register_read_bytes` usage inside `read()`: `Some(reg) => self.register_read_bytes(reg)` → `Some(reg) => register_read_bytes(reg, &self.slots, &self.reg_values)`.
- `build_response` / `response_word_raw` still call `encode_raw` and `pack` as free functions (unchanged names) — confirm they resolve to the `use` import.

- [ ] **Step 6: Run the full I²C suite — the regression guard**

Run: `cargo test -p labwired-core declarative_i2c 2>&1 | tail -20`
Expected: PASS — every pre-existing `declarative_i2c` test still passes (behaviour-preserving extraction).

- [ ] **Step 7: Commit**

```bash
git add crates/core/src/peripherals/components/declarative_regs.rs \
        crates/core/src/peripherals/components/declarative_i2c.rs \
        crates/core/src/peripherals/components/mod.rs
git commit -m "refactor(core): share declarative register helpers across engines"
```

---

## Task 3: `GenericSpiDevice` engine + fixture

**Files:**
- Create: `crates/core/src/peripherals/components/declarative_spi.rs`
- Create: `crates/core/src/peripherals/components/declarative_spi_fixture.yaml`

**Interfaces:**
- Consumes: `super::declarative_regs::{register_read_bytes, unpack}`; `labwired_config::{DeviceDescriptor, RegisterSpec, RegisterAccess, SpiSpec, SpiFraming}`; `crate::peripherals::spi::SpiDevice`; `crate::sim_input::{InputChannel, SimInput, SimInputError}`.
- Produces:
  - `pub struct GenericSpiDevice` with `pub fn from_descriptor(descriptor: &DeviceDescriptor, cs_pin: String, channels: &'static [InputChannel]) -> anyhow::Result<Self>` and `pub fn from_yaml(yaml: &str, cs_pin: &str) -> anyhow::Result<Self>`.
  - `impl SpiDevice for GenericSpiDevice` and `impl SimInput for GenericSpiDevice`.

**Engine state machine (the only genuinely new logic):**
- Per CS-assert (`cs_select`): reset frame — `cmd_consumed = 0`, `is_read = None`, `cur_addr = None`, `read_buf` cleared, `read_idx = 0`, `write_buf` cleared, `latched = false`. If `framing.command_bytes == 0`, the frame is a read of register `0` immediately (`is_read = Some(true)`, `cur_addr = Some(0)`).
- `transfer(mosi)`:
  1. If `command_bytes > 0 && cmd_consumed < command_bytes`: this byte is (part of) the command. On the last command byte, decode `is_read` from `rw_bit`/`rw_read_high` (if `rw_bit` is `None`, leave `is_read = None` → per-register access decides), and `cur_addr = Some((byte >> addr_shift) & addr_mask)`. Increment `cmd_consumed`. Return `0x00` (MISO idle during command).
  2. Else data phase:
     - **Read** (`is_read == Some(true)`, or `is_read` is `None` and the pointed register is `R`/`Rw`-readable): on first data byte, latch `read_buf = build_read_buf(cur_addr)`; return `read_buf[read_idx]` (or `0xFF` past end), advance `read_idx`.
     - **Write** (`is_read == Some(false)`, or `is_read` is `None` and pointed register is `Rw`): push `mosi` into `write_buf`; whenever `write_buf` fills the current register's `width`, `unpack` + store into `reg_values`, and if `auto_increment` advance `cur_addr` to the next-higher register addr and clear the per-register accumulator. Return `0x00`.
- `build_read_buf(start)`: if `auto_increment`, concatenate `register_read_bytes` for every register with `addr >= start`, ascending by `addr`; else just the register whose `addr == start` (empty if none → reads yield `0xFF`).
- `SimInput`: identical contract to `GenericI2cDevice` (channels, `set_input` with range check via a `require_channel` helper copied from the I²C engine, `component_id`).

- [ ] **Step 1: Write the fixture** — `crates/core/src/peripherals/components/declarative_spi_fixture.yaml`:

```yaml
# Test-only fixture for the declarative SPI device primitive (register framing).
#
# A fictional accelerometer-shaped part. NOT a real part and NOT registered in
# KITS, so it never enters the peripherals manifest — it exists solely to
# exercise GenericSpiDevice from declarative_spi.rs tests. It demonstrates the
# ADXL345 framing (rw-bit + auto-increment), a fixed WHOAMI, a rw config
# register, a sourced+encoded measurement, and scale_from range scaling.
type: test_spi_fixture

behavior:
  primitive: spi_device
  spi:
    framing:
      command_bytes: 1
      rw_bit: 7
      rw_read_high: true
      addr_mask: 0x3F
      auto_increment: true
    registers:
      # Fixed identity register (datasheet constant), read-only.
      - { name: WHOAMI, addr: 0x00, width: 1, endian: le, access: r, reset: 0xE5 }
      # rw config; its low 2 bits select the DATA range via scale_from.
      - { name: RANGE, addr: 0x31, width: 1, endian: le, access: rw, reset: 0x00 }
      # X-axis sample, little-endian 16-bit, sources `accel_x`.
      - name: DATAX
        addr: 0x32
        width: 2
        endian: le
        access: r
        source: accel_x
        encode: { scale: 256.0 }        # 256 LSB/g
        scale_from: { register: RANGE, mask: 0x3, shift: 0, map: { 0: 1.0, 1: 2.0, 3: 4.0 } }

metadata:
  label: "Declarative SPI fixture"
  summary: "Test-only accelerometer-shaped SPI part (register framing)."
  category: spi
  inputs:
    - { key: accel_x, label: "Accel X", unit: g, min: -16, max: 16, default: 1 }
```

- [ ] **Step 2: Write the failing engine tests** — create `declarative_spi.rs` with the license header, a `use` block, and this test module (implementation comes next, so it will not compile yet):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::spi::SpiDevice;

    const FIXTURE: &str = include_str!("declarative_spi_fixture.yaml");

    fn dev() -> GenericSpiDevice {
        GenericSpiDevice::from_yaml(FIXTURE, "PA4").unwrap()
    }

    /// Clock a read: assert CS, send command byte (read | addr), read `n` bytes.
    fn read_reg(d: &mut GenericSpiDevice, addr: u8, n: usize) -> Vec<u8> {
        d.cs_select();
        d.transfer(0x80 | addr); // rw_bit=7 set ⇒ read
        let out: Vec<u8> = (0..n).map(|_| d.transfer(0x00)).collect();
        d.cs_release();
        out
    }

    /// Clock a write: assert CS, send command byte (write | addr), send data.
    fn write_reg(d: &mut GenericSpiDevice, addr: u8, data: &[u8]) {
        d.cs_select();
        d.transfer(addr); // rw_bit=7 clear ⇒ write
        for &b in data {
            d.transfer(b);
        }
        d.cs_release();
    }

    #[test]
    fn cs_pin_is_wired() {
        assert_eq!(dev().cs_pin(), "PA4");
    }

    #[test]
    fn whoami_reads_fixed_reset_value() {
        let mut d = dev();
        assert_eq!(read_reg(&mut d, 0x00, 1), vec![0xE5]);
    }

    #[test]
    fn data_register_sources_measurement_little_endian() {
        // accel_x default 1 g × 256 LSB/g × range×1 = 256 = 0x0100, LE.
        let mut d = dev();
        assert_eq!(read_reg(&mut d, 0x32, 2), vec![0x00, 0x01]);
    }

    #[test]
    fn set_input_drives_the_data_register() {
        let mut d = dev();
        d.set_input("accel_x", 2.0).unwrap();
        // 2 g × 256 = 512 = 0x0200, LE.
        assert_eq!(read_reg(&mut d, 0x32, 2), vec![0x00, 0x02]);
    }

    #[test]
    fn rw_register_write_then_scale_from_changes_data() {
        let mut d = dev();
        // Program RANGE = 3 ⇒ scale_from ×4 ⇒ 1 g × 256 × 4 = 1024 = 0x0400.
        write_reg(&mut d, 0x31, &[0x03]);
        assert_eq!(read_reg(&mut d, 0x32, 2), vec![0x00, 0x04]);
    }

    #[test]
    fn auto_increment_walks_ascending_registers() {
        // Read starting at 0x31 for 3 bytes: RANGE(1B, reset 0) then DATAX(2B).
        let mut d = dev();
        let b = read_reg(&mut d, 0x31, 3);
        assert_eq!(b, vec![0x00, 0x00, 0x01]); // RANGE=0, then DATAX 256 LE
    }

    #[test]
    fn reads_past_the_last_register_return_ff() {
        let mut d = dev();
        let b = read_reg(&mut d, 0x32, 4); // DATAX is 2 bytes; 2 more ⇒ 0xFF
        assert_eq!(&b[2..], &[0xFF, 0xFF]);
    }

    #[test]
    fn out_of_range_and_unknown_channels_are_rejected() {
        let mut d = dev();
        assert!(d.set_input("accel_x", 99.0).is_err());
        assert!(d.set_input("nope", 1.0).is_err());
    }
}
```

- [ ] **Step 3: Run to verify failure**

Run: `cargo test -p labwired-core declarative_spi 2>&1 | tail -20`
Expected: FAIL — `GenericSpiDevice` undefined (compile error).

- [ ] **Step 4: Implement the engine** — above the test module in `declarative_spi.rs`:

```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic **declarative SPI device** — one engine device driven entirely by a
//! datasheet-shaped [`labwired_config::SpiSpec`], so a register-style SPI sensor
//! that fits the CS-framed command/register shape is a YAML file with zero Rust.
//!
//! The wire model is the near-universal register-sensor framing (ADXL345,
//! BMP280-SPI, LIS3DH): CS↓, one command byte carrying a read/write bit and a
//! register address, then a streamed word; a multi-byte burst auto-increments
//! the address. A read-only part (`command_bytes: 0`, e.g. MAX31855) clocks its
//! register-0 word straight out on CS↓. The measurement→word math is shared with
//! the I²C engine via [`super::declarative_regs`]; only the framing is new.

use std::any::Any;
use std::collections::HashMap;

use anyhow::{bail, Context, Result};
use labwired_config::{DeviceDescriptor, RegisterAccess, RegisterSpec, SpiFraming};

use super::declarative_regs::{register_read_bytes, unpack};
use crate::peripherals::spi::SpiDevice;
use crate::sim_input::{InputChannel, SimInput, SimInputError};

pub struct GenericSpiDevice {
    cs_pin: String,
    framing: SpiFraming,
    registers: Vec<RegisterSpec>,

    slots: HashMap<String, f64>,
    reg_values: HashMap<String, u32>,

    // Per-frame state.
    cmd_consumed: u8,
    is_read: Option<bool>,
    cur_addr: Option<u8>,
    read_buf: Vec<u8>,
    read_idx: usize,
    latched: bool,
    /// Bytes accumulated toward the current write register's width.
    write_acc: Vec<u8>,

    channels: &'static [InputChannel],
    component_id: Option<String>,
}

impl GenericSpiDevice {
    pub fn from_descriptor(
        descriptor: &DeviceDescriptor,
        cs_pin: String,
        channels: &'static [InputChannel],
    ) -> Result<Self> {
        let spec = descriptor
            .behavior
            .spi
            .as_ref()
            .context("declarative spi device is missing behavior.spi")?;
        if spec.registers.is_empty() {
            bail!("behavior.spi declares no registers");
        }
        if spec.framing.command_bytes > 1 {
            bail!(
                "behavior.spi command_bytes {} unsupported (0 or 1)",
                spec.framing.command_bytes
            );
        }
        let mut slots = HashMap::new();
        if let Some(meta) = &descriptor.metadata {
            for input in &meta.inputs {
                slots.insert(input.key.clone(), input.default.unwrap_or(0.0));
            }
        }
        let reg_values = spec
            .registers
            .iter()
            .map(|r| (r.name.clone(), r.reset))
            .collect();
        Ok(Self {
            cs_pin,
            framing: spec.framing.clone(),
            registers: spec.registers.clone(),
            slots,
            reg_values,
            cmd_consumed: 0,
            is_read: None,
            cur_addr: None,
            read_buf: Vec::new(),
            read_idx: 0,
            latched: false,
            write_acc: Vec::with_capacity(4),
            channels,
            component_id: None,
        })
    }

    pub fn from_yaml(yaml: &str, cs_pin: &str) -> Result<Self> {
        let descriptor = DeviceDescriptor::from_yaml(yaml)?;
        let channels = super::declarative_i2c::leak_channels(&descriptor);
        Self::from_descriptor(&descriptor, cs_pin.to_string(), channels)
    }

    fn find_register(&self, addr: u8) -> Option<&RegisterSpec> {
        self.registers.iter().find(|r| r.addr == addr)
    }

    fn next_addr_above(&self, addr: u8) -> Option<u8> {
        self.registers
            .iter()
            .filter(|r| r.addr > addr)
            .map(|r| r.addr)
            .min()
    }

    /// Concatenated read stream from `start`: every register at addr ≥ start in
    /// ascending order (auto-increment), or just the matched register.
    fn build_read_buf(&self, start: u8) -> Vec<u8> {
        let mut out = Vec::new();
        if self.framing.auto_increment {
            let mut regs: Vec<&RegisterSpec> =
                self.registers.iter().filter(|r| r.addr >= start).collect();
            regs.sort_by_key(|r| r.addr);
            for r in regs {
                out.extend(register_read_bytes(r, &self.slots, &self.reg_values));
            }
        } else if let Some(r) = self.find_register(start) {
            out.extend(register_read_bytes(r, &self.slots, &self.reg_values));
        }
        out
    }

    fn require_channel(&self, key: &str, value: f64) -> Result<(), SimInputError> {
        let ch = self
            .channels
            .iter()
            .find(|c| c.key == key)
            .ok_or_else(|| SimInputError::UnknownChannel(key.to_string()))?;
        if value < ch.min || value > ch.max {
            return Err(SimInputError::OutOfRange {
                key: key.to_string(),
                value,
                min: ch.min,
                max: ch.max,
            });
        }
        Ok(())
    }
}

impl SpiDevice for GenericSpiDevice {
    fn cs_pin(&self) -> &str {
        &self.cs_pin
    }

    fn cs_select(&mut self) {
        self.cmd_consumed = 0;
        self.is_read = None;
        self.cur_addr = None;
        self.read_buf.clear();
        self.read_idx = 0;
        self.latched = false;
        self.write_acc.clear();
        if self.framing.command_bytes == 0 {
            self.is_read = Some(true);
            self.cur_addr = Some(0);
        }
    }

    fn cs_release(&mut self) {
        self.write_acc.clear();
    }

    fn transfer(&mut self, mosi: u8) -> u8 {
        // Command phase.
        if self.framing.command_bytes > 0 && self.cmd_consumed < self.framing.command_bytes {
            self.cmd_consumed += 1;
            if self.cmd_consumed == self.framing.command_bytes {
                if let Some(bit) = self.framing.rw_bit {
                    let set = (mosi >> bit) & 1 == 1;
                    self.is_read = Some(set == self.framing.rw_read_high);
                }
                self.cur_addr =
                    Some((mosi >> self.framing.addr_shift) & self.framing.addr_mask);
            }
            return 0x00;
        }
        // Data phase.
        let addr = self.cur_addr.unwrap_or(0);
        let write = matches!(self.is_read, Some(false))
            || (self.is_read.is_none()
                && self
                    .find_register(addr)
                    .map(|r| r.access == RegisterAccess::Rw)
                    .unwrap_or(false)
                && false); // read-biased when rw_bit is None; writes need explicit rw_bit
        if write {
            self.write_acc.push(mosi);
            if let Some(reg) = self.find_register(addr) {
                if reg.access == RegisterAccess::Rw
                    && self.write_acc.len() == reg.width as usize
                {
                    let val = unpack(&self.write_acc, reg.endian);
                    self.reg_values.insert(reg.name.clone(), val);
                    self.write_acc.clear();
                    if self.framing.auto_increment {
                        if let Some(next) = self.next_addr_above(addr) {
                            self.cur_addr = Some(next);
                        }
                    }
                }
            }
            return 0x00;
        }
        // Read.
        if !self.latched {
            self.read_buf = self.build_read_buf(addr);
            self.latched = true;
        }
        let byte = self.read_buf.get(self.read_idx).copied().unwrap_or(0xFF);
        self.read_idx += 1;
        byte
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
    fn as_sim_input_mut(&mut self) -> Option<&mut dyn SimInput> {
        Some(self)
    }
}

impl SimInput for GenericSpiDevice {
    fn input_channels(&self) -> &'static [InputChannel] {
        self.channels
    }
    fn set_input(&mut self, key: &str, value: f64) -> Result<(), SimInputError> {
        self.require_channel(key, value)?;
        self.slots.insert(key.to_string(), value);
        Ok(())
    }
    fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }
    fn set_component_id(&mut self, id: String) {
        self.component_id = Some(id);
    }
}
```

Note on the write path: with `rw_bit` set (the fixture and every register part), `is_read` is always `Some(true|false)`, so the `is_read.is_none()` branch is dead for those parts — the `&& false` makes that explicit (writes require an explicit `rw_bit`, matching the datasheets). Keep it simple; do not invent a heuristic write path.

- [ ] **Step 5: Make `leak_channels` reachable** — in `declarative_i2c.rs`, change `fn leak_channels` to `pub(crate) fn leak_channels` (it is currently private). This is the single shared channel-leaking helper; both engines use it.

- [ ] **Step 6: Register the module** — in `mod.rs` add `pub mod declarative_spi;`.

- [ ] **Step 7: Run the engine tests**

Run: `cargo test -p labwired-core declarative_spi 2>&1 | tail -25`
Expected: PASS — all eight engine tests pass.

- [ ] **Step 8: Run the I²C suite (guard the `leak_channels` visibility change)**

Run: `cargo test -p labwired-core declarative_i2c 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/core/src/peripherals/components/declarative_spi.rs \
        crates/core/src/peripherals/components/declarative_spi_fixture.yaml \
        crates/core/src/peripherals/components/declarative_i2c.rs \
        crates/core/src/peripherals/components/mod.rs
git commit -m "feat(core): GenericSpiDevice declarative SPI engine + fixture"
```

---

## Task 4: `DeclarativeSpiKit` (no real parts registered)

**Files:**
- Modify: `crates/core/src/peripherals/components/declarative_spi.rs`
- Modify: `crates/core/src/peripherals/components/mod.rs` (`pub use`)

**Interfaces:**
- Consumes: `crate::peripherals::kit::{AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport}`; `GenericSpiDevice`.
- Produces: `pub struct DeclarativeSpiKit` with `pub fn from_yaml(yaml: &str) -> Result<Self>` and `impl PeripheralKit`. No entry added to `registry::KITS` (manifest unchanged).

- [ ] **Step 1: Write the failing test** — add to the `declarative_spi.rs` test module:

```rust
    #[test]
    fn declarative_spi_kit_builds_metadata_from_descriptor() {
        let kit = DeclarativeSpiKit::from_yaml(FIXTURE).unwrap();
        let m = kit.metadata();
        assert_eq!(m.device_type, "test_spi_fixture");
        assert!(matches!(m.transport, crate::peripherals::kit::Transport::Spi));
        assert_eq!(m.inputs.len(), 1);
        assert!(m.inputs.iter().any(|c| c.key == "accel_x"));
    }

    #[test]
    fn declarative_spi_kit_rejects_wrong_primitive() {
        let yaml = r#"
type: bad
behavior:
  primitive: i2c_device
  spi:
    registers:
      - { name: A, addr: 0, width: 1, endian: le, access: r }
"#;
        assert!(DeclarativeSpiKit::from_yaml(yaml).is_err());
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p labwired-core declarative_spi_kit 2>&1 | tail -15`
Expected: FAIL — `DeclarativeSpiKit` undefined.

- [ ] **Step 3: Implement the kit** — append to `declarative_spi.rs` (after the `SimInput` impl, before the test module):

```rust
// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

/// A [`PeripheralKit`] backed by a declarative `spi_device` descriptor — one
/// instance per YAML device. Phase 1 registers no real parts, so nothing is
/// added to `registry::KITS` and the offline peripherals manifest is unchanged.
pub struct DeclarativeSpiKit {
    descriptor: DeviceDescriptor,
    channels: &'static [InputChannel],
    metadata: &'static KitMetadata,
}

impl DeclarativeSpiKit {
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        let descriptor = DeviceDescriptor::from_yaml(yaml)?;
        if descriptor.behavior.primitive != "spi_device" {
            bail!(
                "declarative spi kit requires behavior.primitive: spi_device, got '{}'",
                descriptor.behavior.primitive
            );
        }
        descriptor
            .behavior
            .spi
            .as_ref()
            .context("declarative spi kit is missing behavior.spi")?;
        let channels = super::declarative_i2c::leak_channels(&descriptor);
        let metadata = leak_metadata(&descriptor, channels);
        Ok(Self {
            descriptor,
            channels,
            metadata,
        })
    }
}

fn leak_metadata(
    descriptor: &DeviceDescriptor,
    channels: &'static [InputChannel],
) -> &'static KitMetadata {
    let meta = descriptor.metadata.as_ref();
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    let label = meta
        .and_then(|m| m.label.clone())
        .unwrap_or_else(|| descriptor.r#type.clone());
    let summary = meta
        .and_then(|m| m.summary.clone())
        .unwrap_or_else(|| "Declarative SPI device.".to_string());
    let config_keys: &'static [ConfigKey] = Box::leak(
        vec![ConfigKey {
            name: "cs_pin",
            ty: ConfigType::Str,
            doc: "CS GPIO pin wired as SPI chip-select (e.g. \"PA4\").",
        }]
        .into_boxed_slice(),
    );
    Box::leak(Box::new(KitMetadata {
        device_type: leak(descriptor.r#type.clone()),
        label: leak(label),
        summary: leak(summary.clone()),
        detail: leak(summary),
        transport: Transport::Spi,
        category: Category::Spi,
        config_keys,
        labs: &[],
        inputs: channels,
    }))
}

impl PeripheralKit for DeclarativeSpiKit {
    fn metadata(&self) -> &'static KitMetadata {
        self.metadata
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> Result<()> {
        let cs_pin = ctx.config_str("cs_pin").unwrap_or("PA4").to_string();
        let device = GenericSpiDevice::from_descriptor(&self.descriptor, cs_pin, self.channels)?;
        ctx.attach_spi_device(Box::new(device))
    }
}
```

> **Verify before writing:** confirm `AttachCtx` exposes `config_str(&str) -> Option<&str>` (it is used by `max7219.rs:242`). If the accessor differs, match `max7219.rs` exactly.

- [ ] **Step 4: Export from `mod.rs`** — add:

```rust
pub use declarative_spi::{DeclarativeSpiKit, GenericSpiDevice};
```

- [ ] **Step 5: Run the kit tests**

Run: `cargo test -p labwired-core declarative_spi 2>&1 | tail -20`
Expected: PASS — engine + kit tests all pass.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/peripherals/components/declarative_spi.rs \
        crates/core/src/peripherals/components/mod.rs
git commit -m "feat(core): DeclarativeSpiKit for declarative SPI parts"
```

---

## Task 5: Workspace gate — manifest unchanged, fmt, clippy

**Files:** none (verification + final commit only).

- [ ] **Step 1: Confirm the peripherals manifest is unchanged**

Run: `git diff --stat origin/main -- 'crates/**/peripherals*.json' 'docs/boards/'`
Expected: **empty** — Phase 1 registered no parts, so no manifest / matrix file changed. If anything appears, a `registry::KITS` entry was added by mistake — remove it.

- [ ] **Step 2: Full test suite for both crates**

Run: `cargo test -p labwired-config -p labwired-core 2>&1 | tail -15`
Expected: PASS — no regressions anywhere (the I²C suite in particular).

- [ ] **Step 3: Format**

Run: `cargo fmt --all && git diff --stat`
Expected: no changes, or only whitespace in the files touched this plan (commit them if so).

- [ ] **Step 4: Clippy (matches the CI gate)**

Run: `cargo clippy -p labwired-config -p labwired-core --all-targets -- -D warnings 2>&1 | tail -20`
Expected: no warnings. Common fixes: an unused import from the `declarative_i2c` helper extraction (Task 2) — remove it; the `&& false` dead branch in the engine may trip `clippy::overly_complex_bool_expr` — if so, replace the whole `write` binding with `let write = matches!(self.is_read, Some(false));` and drop the `is_none()` clause (writes require an explicit `rw_bit`, so this is equivalent and clearer).

- [ ] **Step 5: Final commit (only if fmt/clippy changed anything)**

```bash
git add -A
git commit -m "style(core): fmt + clippy for declarative SPI primitive"
```

---

## Follow-on plans (NOT this plan)

Per the design spec, Phases 2 and 3 are separate PRs and get their own plans once Phase 1 lands:

- **Phase 2 — register real SPI parts:** `configs/devices/adxl345.yaml` (rw + auto-increment) and `configs/devices/max31855.yaml` (`command_bytes: 0`, read-only), wired through `embedded_device_yaml` + a `LazyLock<DeclarativeSpiKit>` static each + one line in `registry::KITS`. Manifest snapshot updates here (intentionally).
- **Phase 3 — fleet matrix:** an L3 SPI sensor-sample firmware across Arduino + Zephyr systems on F1/nRF/RP2040/C3, turning SPI L3 cells green in the Tier-1 matrix — mirroring `4ff81b13`→`2c55eaf7`.

---

## Self-Review

- **Spec coverage:** schema (Task 1) ✓; shared-machinery reuse (Task 2) ✓; CS-framed engine incl. read-only `command_bytes: 0`, rw writes, auto-increment, scale_from (Task 3) ✓; kit + `Transport::Spi` (Task 4) ✓; manifest-unchanged constraint (Task 5) ✓; struct rename+alias decision (Task 1) ✓. Phases 2/3 explicitly deferred to their own plans, matching the spec's phasing.
- **Placeholder scan:** no TBD/TODO; every code step carries complete, compilable code. Two "verify before writing" notes (`config_str`, clippy dead-branch) point at an exact existing reference (`max7219.rs`) rather than leaving a gap.
- **Type consistency:** `RegisterSpec`/`RegisterAccess` (+ `I2cRegister`/`I2cAccess` aliases), `SpiSpec`/`SpiFraming`, `GenericSpiDevice::from_descriptor(&DeviceDescriptor, String, &'static [InputChannel])` and `from_yaml(&str, &str)`, `DeclarativeSpiKit::from_yaml(&str)`, shared `declarative_regs::{register_read_bytes, unpack, pack, encode_raw}` and `declarative_i2c::leak_channels` (made `pub(crate)`) are named identically across every task that references them.
