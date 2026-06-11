# Component Model IR (Slice 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Agents define off-chip I2C devices as declarative IR specs that a deterministic interpreter executes byte-for-byte identically to the hand-written Rust models.

**Architecture:** New `IrComponent` types in `core/crates/ir` (sibling to `IrDevice`); a pure-state-machine interpreter in `core/crates/core/src/peripherals/components/ir_component.rs` implementing the existing 5-method `I2cDevice` trait; wired into `build_i2c_device()` as `type: ir`; surfaced via `labwired asset validate-component` CLI and a `labwired_define_component` MCP tool. PCA9685 and TMP102 specs gate equivalence.

**Tech Stack:** Rust (serde, serde_yaml, clap), TypeScript (MCP server in `packages/mcp`), vitest.

**Spec:** `docs/superpowers/specs/2026-06-11-hw-substrate-sota-design.md` (Slice 1)

---

### Task 1: IrComponent data model

**Files:**
- Create: `core/crates/ir/src/component.rs`
- Modify: `core/crates/ir/src/lib.rs` (add `pub mod component;`)

- [ ] **Step 1: Write the failing round-trip test**

In `core/crates/ir/src/component.rs` (module skeleton + test):

```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Declarative IR for off-chip components (sensors, drivers, expanders).
//!
//! Sibling to [`crate::IrDevice`] (on-chip, memory-mapped). An `IrComponent`
//! describes a bus-attached device behaviorally: register file, pointer
//! semantics, wide-register read phasing, update rules, and observables.
//! A deterministic interpreter in the simulator core executes these specs;
//! see `labwired_core::peripherals::components::ir_component`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pca9685_shaped_spec_round_trips_yaml() {
        let yaml = r#"
name: PCA9685
vendor: NXP
datasheet: "PCA9685 Rev. 4"
kind: declarative
interface:
  i2c:
    default_address: 0x40
register_file:
  size: 256
  reset:
    0x00: 0x11
pointer:
  first_write_after_start_sets_pointer: true
  auto_increment:
    when_field_set: { reg: 0x00, mask: 0x20 }
observables:
  - name: servo_angle
    channels: 16
    base: 0x06
    stride: 4
    value:
      u12_compose: { lo_rel: 2, hi_rel: 3, hi_mask: 0x0F }
    map:
      linear: { scale: 0.46258224, offset: -47.368423, clamp: [0.0, 180.0] }
      none_when_raw_zero: true
"#;
        let spec: IrComponent = serde_yaml::from_str(yaml).expect("parse");
        assert_eq!(spec.name, "PCA9685");
        let IrComponentInterface::I2c { default_address } = &spec.interface;
        assert_eq!(*default_address, 0x40);
        assert_eq!(spec.register_file.size, 256);
        assert_eq!(spec.register_file.reset.get(&0x00), Some(&0x11));
        let obs = &spec.observables[0];
        assert_eq!(obs.channels, 16);
        // Round-trip: serialize and re-parse equal.
        let re: IrComponent =
            serde_yaml::from_str(&serde_yaml::to_string(&spec).unwrap()).unwrap();
        assert_eq!(re.name, spec.name);
        assert_eq!(re.register_file.reset, spec.register_file.reset);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd core && cargo test -p labwired-ir component -- --nocapture`
Expected: compile FAIL — `IrComponent` not defined.

- [ ] **Step 3: Implement the data model**

Add above the test module in `component.rs`:

```rust
/// Top-level declarative component spec.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrComponent {
    /// Part number, e.g. "PCA9685".
    pub name: String,
    /// Manufacturer, e.g. "NXP".
    #[serde(default)]
    pub vendor: Option<String>,
    /// Datasheet revision the behavior was derived from.
    #[serde(default)]
    pub datasheet: Option<String>,
    /// Execution kind. `wasm` is reserved by the design and rejected by
    /// validation until the WASM slice ships.
    #[serde(default)]
    pub kind: IrComponentKind,
    /// Bus interface. I2C only in Slice 1.
    pub interface: IrComponentInterface,
    /// Backing register file.
    pub register_file: IrRegisterFile,
    /// Pointer (control-register) semantics. Optional: pointerless devices.
    #[serde(default)]
    pub pointer: Option<IrPointerRule>,
    /// Registers read as multi-byte values with a read phase (e.g. 16-bit BE).
    #[serde(default)]
    pub wide_registers: Vec<IrWideRegister>,
    /// State-update rules triggered by bus activity.
    #[serde(default)]
    pub updates: Vec<IrUpdateRule>,
    /// Named values derived from register state, readable by tests/run loop.
    #[serde(default)]
    pub observables: Vec<IrObservable>,
}

/// Execution kind for a component spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum IrComponentKind {
    /// Interpreted declaratively by the core (Slice 1).
    #[default]
    Declarative,
    /// Sandboxed WASM module (reserved; not yet supported).
    Wasm,
}

/// Bus interface binding.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IrComponentInterface {
    /// I2C target device.
    I2c {
        /// 7-bit address when the manifest does not override it.
        default_address: u8,
    },
}

/// Flat byte-addressed register file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrRegisterFile {
    /// Number of 8-bit registers (1..=65536).
    pub size: usize,
    /// Sparse non-zero reset values, keyed by register offset.
    #[serde(default)]
    pub reset: BTreeMap<u64, u8>,
}

/// Write-pointer (control register) semantics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrPointerRule {
    /// The first byte written after START selects the register pointer.
    pub first_write_after_start_sets_pointer: bool,
    /// Mask applied to the pointer byte (e.g. TMP102 uses 0x03).
    #[serde(default = "default_pointer_mask")]
    pub pointer_mask: u8,
    /// When the pointer advances after a data byte.
    #[serde(default)]
    pub auto_increment: IrAutoIncrement,
}

fn default_pointer_mask() -> u8 {
    0xFF
}

/// Auto-increment policy for the register pointer.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum IrAutoIncrement {
    /// Pointer never advances automatically.
    #[default]
    Never,
    /// Pointer advances after every data byte.
    Always,
    /// Pointer advances when `regs[reg] & mask != 0` (e.g. PCA9685 MODE1.AI).
    WhenFieldSet {
        /// Register holding the enable field.
        reg: u64,
        /// Bit mask of the enable field.
        mask: u8,
    },
}

/// A logical register wider than 8 bits, read big-endian over phased reads.
/// The read phase resets to MSB on START.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrWideRegister {
    /// Pointer value that selects this register.
    pub pointer: u8,
    /// Width in bits. Only 16 is supported in Slice 1.
    pub bits: u32,
    /// Initial value (big-endian semantics).
    #[serde(default)]
    pub reset: u16,
}

/// A deterministic state update fired by bus activity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrUpdateRule {
    /// What fires the rule.
    pub trigger: IrUpdateTrigger,
    /// What happens.
    pub action: IrUpdateAction,
}

/// Update triggers.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IrUpdateTrigger {
    /// A full read of the wide register selected by this pointer completed
    /// (both bytes of a 16-bit value were read).
    WideReadComplete {
        /// Pointer value of the wide register.
        pointer: u8,
    },
}

/// Update actions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IrUpdateAction {
    /// `value = value.wrapping_add(add); if value > max { value = reset }`
    /// applied to the triggering wide register (signed compare, as i16).
    AddWrap {
        /// Amount added per trigger (two's-complement i16).
        add: i16,
        /// Exclusive upper bound (i16 compare).
        max: i16,
        /// Value assigned on wrap.
        reset: i16,
    },
}

/// A named, channel-indexed value derived from the register file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrObservable {
    /// Observable name, e.g. "servo_angle".
    pub name: String,
    /// Number of channels (1 for scalar observables).
    pub channels: u8,
    /// Register offset of channel 0's block.
    pub base: u64,
    /// Offset between consecutive channel blocks.
    pub stride: u64,
    /// How the raw value is composed from the channel block.
    pub value: IrObservableValue,
    /// Optional mapping from raw value to engineering units.
    #[serde(default)]
    pub map: Option<IrObservableMap>,
}

/// Raw-value composition.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IrObservableValue {
    /// `((regs[base+hi_rel] & hi_mask) << 8) | regs[base+lo_rel]`
    U12Compose {
        /// Low-byte offset within the channel block.
        lo_rel: u64,
        /// High-byte offset within the channel block.
        hi_rel: u64,
        /// Mask applied to the high byte before shifting.
        hi_mask: u8,
    },
}

/// Raw → engineering-units mapping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrObservableMap {
    /// `eng = clamp(raw * scale + offset)`.
    pub linear: IrLinearMap,
    /// Return `None` while the raw value is exactly 0 (channel never written).
    #[serde(default)]
    pub none_when_raw_zero: bool,
}

/// Linear map coefficients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrLinearMap {
    /// Multiplier.
    pub scale: f32,
    /// Added after scaling.
    pub offset: f32,
    /// Optional inclusive clamp range.
    #[serde(default)]
    pub clamp: Option<(f32, f32)>,
}
```

In `core/crates/ir/src/lib.rs`, after `pub mod svd_transform;`:

```rust
pub mod component;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd core && cargo test -p labwired-ir component -- --nocapture`
Expected: PASS (1 test).

- [ ] **Step 5: Commit**

```bash
git -C core add crates/ir/src/component.rs crates/ir/src/lib.rs
git -C core commit -m "feat(ir): declarative IrComponent model for off-chip devices"
```

*(If `core` is a submodule of the root repo, commit inside it; bump the submodule pointer in the final task.)*

---

### Task 2: Spec validation with machine-readable diagnostics

**Files:**
- Modify: `core/crates/ir/src/component.rs`

- [ ] **Step 1: Write the failing tests**

Append to the `tests` module in `component.rs`:

```rust
    fn minimal_spec() -> IrComponent {
        serde_yaml::from_str(
            r#"
name: X
kind: declarative
interface: { i2c: { default_address: 0x40 } }
register_file: { size: 256 }
"#,
        )
        .unwrap()
    }

    #[test]
    fn valid_minimal_spec_has_no_diagnostics() {
        assert!(minimal_spec().validate().is_empty());
    }

    #[test]
    fn wasm_kind_is_rejected_until_supported() {
        let mut s = minimal_spec();
        s.kind = IrComponentKind::Wasm;
        let d = s.validate();
        assert!(d.iter().any(|d| d.code == "ICOMP_WASM_UNSUPPORTED"), "{d:?}");
    }

    #[test]
    fn reset_offset_out_of_range_is_diagnosed() {
        let mut s = minimal_spec();
        s.register_file.reset.insert(300, 1); // size is 256
        let d = s.validate();
        assert!(d.iter().any(|d| d.code == "ICOMP_RESET_OUT_OF_RANGE"), "{d:?}");
    }

    #[test]
    fn observable_block_must_fit_register_file() {
        let mut s = minimal_spec();
        s.observables.push(IrObservable {
            name: "x".into(),
            channels: 16,
            base: 0xF8, // 0xF8 + 15*4 + 3 > 255
            stride: 4,
            value: IrObservableValue::U12Compose { lo_rel: 2, hi_rel: 3, hi_mask: 0x0F },
            map: None,
        });
        let d = s.validate();
        assert!(d.iter().any(|d| d.code == "ICOMP_OBS_OUT_OF_RANGE"), "{d:?}");
    }

    #[test]
    fn wide_register_only_16_bits() {
        let mut s = minimal_spec();
        s.wide_registers.push(IrWideRegister { pointer: 0, bits: 24, reset: 0 });
        let d = s.validate();
        assert!(d.iter().any(|d| d.code == "ICOMP_WIDE_BITS_UNSUPPORTED"), "{d:?}");
    }

    #[test]
    fn update_trigger_must_reference_declared_wide_register() {
        let mut s = minimal_spec();
        s.updates.push(IrUpdateRule {
            trigger: IrUpdateTrigger::WideReadComplete { pointer: 7 },
            action: IrUpdateAction::AddWrap { add: 1, max: 10, reset: 0 },
        });
        let d = s.validate();
        assert!(d.iter().any(|d| d.code == "ICOMP_UPDATE_DANGLING"), "{d:?}");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd core && cargo test -p labwired-ir component`
Expected: compile FAIL — `validate`/`IrComponentDiag` not defined.

- [ ] **Step 3: Implement validation**

Add to `component.rs` (above tests):

```rust
/// A validation finding. Mirrors the diagram-diagnostic shape used by the
/// MCP surface: stable code + human message + suggested fix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrComponentDiag {
    /// Stable machine-readable code, e.g. "ICOMP_RESET_OUT_OF_RANGE".
    pub code: String,
    /// Human-readable message.
    pub message: String,
    /// Suggested fix.
    pub hint: String,
}

impl IrComponentDiag {
    fn new(code: &str, message: String, hint: &str) -> Self {
        Self { code: code.into(), message, hint: hint.into() }
    }
}

impl IrComponent {
    /// Validate the spec. Empty result means executable. Every condition the
    /// interpreter cannot execute deterministically is rejected here, never
    /// at run time.
    pub fn validate(&self) -> Vec<IrComponentDiag> {
        let mut out = Vec::new();
        if self.kind == IrComponentKind::Wasm {
            out.push(IrComponentDiag::new(
                "ICOMP_WASM_UNSUPPORTED",
                "kind: wasm is reserved and not yet executable".into(),
                "Use kind: declarative",
            ));
        }
        let size = self.register_file.size as u64;
        if self.register_file.size == 0 || self.register_file.size > 65536 {
            out.push(IrComponentDiag::new(
                "ICOMP_REGFILE_SIZE",
                format!("register_file.size {} outside 1..=65536", self.register_file.size),
                "Choose the device's real register-file size (often 256)",
            ));
        }
        for (&off, _) in &self.register_file.reset {
            if off >= size {
                out.push(IrComponentDiag::new(
                    "ICOMP_RESET_OUT_OF_RANGE",
                    format!("reset value at offset {off:#x} outside register file (size {size:#x})"),
                    "Remove the entry or grow register_file.size",
                ));
            }
        }
        if let Some(p) = &self.pointer {
            if let IrAutoIncrement::WhenFieldSet { reg, .. } = p.auto_increment {
                if reg >= size {
                    out.push(IrComponentDiag::new(
                        "ICOMP_AI_REG_OUT_OF_RANGE",
                        format!("auto_increment enable register {reg:#x} outside register file"),
                        "Point at a register inside the file",
                    ));
                }
            }
        }
        for w in &self.wide_registers {
            if w.bits != 16 {
                out.push(IrComponentDiag::new(
                    "ICOMP_WIDE_BITS_UNSUPPORTED",
                    format!("wide register at pointer {:#x} has bits={}, only 16 supported", w.pointer, w.bits),
                    "Use bits: 16",
                ));
            }
        }
        for u in &self.updates {
            let IrUpdateTrigger::WideReadComplete { pointer } = u.trigger;
            if !self.wide_registers.iter().any(|w| w.pointer == pointer) {
                out.push(IrComponentDiag::new(
                    "ICOMP_UPDATE_DANGLING",
                    format!("update trigger references undeclared wide register pointer {pointer:#x}"),
                    "Declare the wide register or fix the pointer",
                ));
            }
        }
        for o in &self.observables {
            let IrObservableValue::U12Compose { lo_rel, hi_rel, .. } = o.value;
            let span = lo_rel.max(hi_rel);
            let last = o.base + o.stride * (o.channels.max(1) as u64 - 1) + span;
            if last >= size {
                out.push(IrComponentDiag::new(
                    "ICOMP_OBS_OUT_OF_RANGE",
                    format!("observable '{}' channel block ends at {last:#x}, outside register file (size {size:#x})", o.name),
                    "Reduce channels/base/stride or grow register_file.size",
                ));
            }
        }
        out
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd core && cargo test -p labwired-ir component`
Expected: PASS (7 tests).

- [ ] **Step 5: Commit**

```bash
git -C core add crates/ir/src/component.rs
git -C core commit -m "feat(ir): IrComponent validation with stable diagnostic codes"
```

---

### Task 3: Interpreter — pointer, register file, auto-increment

**Files:**
- Create: `core/crates/core/src/peripherals/components/ir_component.rs`
- Modify: `core/crates/core/src/peripherals/components/mod.rs`

- [ ] **Step 1: Write the failing tests**

`ir_component.rs` skeleton with tests mirroring the PCA9685 unit tests:

```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Deterministic interpreter executing an [`labwired_ir::component::IrComponent`]
//! as an [`I2cDevice`]. Pure state machine over the spec: no host time, no
//! randomness, no I/O — determinism is preserved by construction.

use crate::peripherals::i2c::I2cDevice;
use labwired_ir::component::{
    IrAutoIncrement, IrComponent, IrComponentInterface, IrObservableValue, IrUpdateAction,
    IrUpdateTrigger,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn pca_like_spec() -> IrComponent {
        serde_yaml::from_str(
            r#"
name: PCA-like
interface: { i2c: { default_address: 0x40 } }
register_file:
  size: 256
  reset: { 0x00: 0x11 }
pointer:
  first_write_after_start_sets_pointer: true
  auto_increment:
    when_field_set: { reg: 0x00, mask: 0x20 }
"#,
        )
        .unwrap()
    }

    #[test]
    fn rejects_invalid_spec() {
        let mut s = pca_like_spec();
        s.register_file.reset.insert(999, 1);
        assert!(IrI2cComponent::new(s, None).is_err());
    }

    #[test]
    fn address_default_and_override() {
        let d = IrI2cComponent::new(pca_like_spec(), None).unwrap();
        assert_eq!(d.address(), 0x40);
        let d = IrI2cComponent::new(pca_like_spec(), Some(0x41)).unwrap();
        assert_eq!(d.address(), 0x41);
    }

    #[test]
    fn reset_value_reads_back_via_pointer() {
        let mut d = IrI2cComponent::new(pca_like_spec(), None).unwrap();
        d.start();
        d.write(0x00); // pointer = MODE1
        assert_eq!(d.read(), 0x11);
    }

    #[test]
    fn auto_increment_block_write_lands_consecutively() {
        let mut d = IrI2cComponent::new(pca_like_spec(), None).unwrap();
        // Enable AI: MODE1 |= 0x20 (write 0xA1 like the firmware does).
        d.start();
        d.write(0x00);
        d.write(0xA1);
        // 4-byte block write at 0x06.
        d.start();
        d.write(0x06);
        for v in [0xDE, 0xAD, 0xBE, 0xEF] {
            d.write(v);
        }
        d.start();
        d.write(0x06);
        assert_eq!([d.read(), d.read(), d.read(), d.read()], [0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn without_ai_pointer_stays_put() {
        let mut d = IrI2cComponent::new(pca_like_spec(), None).unwrap();
        d.start();
        d.write(0x06);
        d.write(0x01);
        d.write(0x02); // overwrites same register: AI is off (MODE1=0x11)
        d.start();
        d.write(0x06);
        assert_eq!(d.read(), 0x02);
    }
}
```

In `mod.rs` add `pub mod ir_component;` (alphabetical position, after `ili9341`) and `pub use ir_component::IrI2cComponent;` in the re-export block.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd core && cargo test -p labwired-core ir_component`
Expected: compile FAIL — `IrI2cComponent` not defined.

- [ ] **Step 3: Implement the interpreter core**

Add above the tests in `ir_component.rs`:

```rust
/// Interpreter state for one component instance.
pub struct IrI2cComponent {
    spec: IrComponent,
    addr: u8,
    regs: Vec<u8>,
    wide: Vec<i16>, // parallel to spec.wide_registers
    pointer: u8,
    read_phase: u8, // 0 = MSB next (wide reads); reset on START
    writes_since_start: u32,
}

impl IrI2cComponent {
    /// Build an interpreter from a validated spec. `address_override` comes
    /// from the manifest's `i2c_address` config when present.
    pub fn new(spec: IrComponent, address_override: Option<u8>) -> Result<Self, String> {
        let diags = spec.validate();
        if !diags.is_empty() {
            return Err(diags
                .iter()
                .map(|d| format!("{}: {}", d.code, d.message))
                .collect::<Vec<_>>()
                .join("; "));
        }
        let IrComponentInterface::I2c { default_address } = &spec.interface;
        let default_address = *default_address;
        let mut regs = vec![0u8; spec.register_file.size];
        for (&off, &v) in &spec.register_file.reset {
            regs[off as usize] = v;
        }
        let wide = spec.wide_registers.iter().map(|w| w.reset as i16).collect();
        Ok(Self {
            addr: address_override.unwrap_or(default_address),
            regs,
            wide,
            pointer: 0,
            read_phase: 0,
            writes_since_start: 0,
            spec,
        })
    }

    fn auto_increment_enabled(&self) -> bool {
        match self.spec.pointer.as_ref().map(|p| &p.auto_increment) {
            Some(IrAutoIncrement::Always) => true,
            Some(IrAutoIncrement::WhenFieldSet { reg, mask }) => {
                self.regs[*reg as usize] & mask != 0
            }
            Some(IrAutoIncrement::Never) | None => false,
        }
    }

    fn wide_index(&self, pointer: u8) -> Option<usize> {
        self.spec.wide_registers.iter().position(|w| w.pointer == pointer)
    }
}

impl I2cDevice for IrI2cComponent {
    fn address(&self) -> u8 {
        self.addr
    }

    fn start(&mut self) {
        self.writes_since_start = 0;
        self.read_phase = 0;
    }

    fn write(&mut self, data: u8) {
        let pointered = self
            .spec
            .pointer
            .as_ref()
            .map(|p| p.first_write_after_start_sets_pointer)
            .unwrap_or(false);
        if pointered && self.writes_since_start == 0 {
            let mask = self.spec.pointer.as_ref().unwrap().pointer_mask;
            self.pointer = data & mask;
        } else {
            let idx = self.pointer as usize % self.regs.len();
            self.regs[idx] = data;
            if self.auto_increment_enabled() {
                self.pointer = self.pointer.wrapping_add(1);
            }
        }
        self.writes_since_start = self.writes_since_start.saturating_add(1);
    }

    fn read(&mut self) -> u8 {
        if let Some(wi) = self.wide_index(self.pointer) {
            let value = self.wide[wi] as u16;
            let byte = if self.read_phase == 0 { (value >> 8) as u8 } else { (value & 0xFF) as u8 };
            self.read_phase ^= 1;
            if self.read_phase == 0 {
                self.apply_updates_on_wide_read_complete(self.pointer);
            }
            return byte;
        }
        let idx = self.pointer as usize % self.regs.len();
        let v = self.regs[idx];
        if self.auto_increment_enabled() {
            self.pointer = self.pointer.wrapping_add(1);
        }
        v
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

impl IrI2cComponent {
    fn apply_updates_on_wide_read_complete(&mut self, pointer: u8) {
        // Collect first to satisfy the borrow checker; specs are small.
        let actions: Vec<IrUpdateAction> = self
            .spec
            .updates
            .iter()
            .filter(|u| matches!(u.trigger, IrUpdateTrigger::WideReadComplete { pointer: p } if p == pointer))
            .map(|u| u.action.clone())
            .collect();
        if actions.is_empty() {
            return;
        }
        let wi = self.wide_index(pointer).expect("validated");
        for a in actions {
            let IrUpdateAction::AddWrap { add, max, reset } = a;
            let mut v = self.wide[wi].wrapping_add(add);
            if v > max {
                v = reset;
            }
            self.wide[wi] = v;
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd core && cargo test -p labwired-core ir_component`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git -C core add crates/core/src/peripherals/components/ir_component.rs crates/core/src/peripherals/components/mod.rs
git -C core commit -m "feat(core): IrComponent interpreter as I2cDevice (pointer/AI/wide-read core)"
```

---

### Task 4: Observables

**Files:**
- Modify: `core/crates/core/src/peripherals/components/ir_component.rs`

- [ ] **Step 1: Write the failing tests**

Append to tests in `ir_component.rs`:

```rust
    fn pca_like_with_observable() -> IrComponent {
        let mut s = pca_like_spec();
        s.observables = serde_yaml::from_str(
            r#"
- name: servo_angle
  channels: 16
  base: 0x06
  stride: 4
  value: { u12_compose: { lo_rel: 2, hi_rel: 3, hi_mask: 0x0F } }
  map:
    linear: { scale: 0.46258224, offset: -47.368423, clamp: [0.0, 180.0] }
    none_when_raw_zero: true
"#,
        )
        .unwrap();
        s
    }

    fn ir_set_angle(d: &mut IrI2cComponent, ch: u8, deg: f64) {
        let us = 500.0 + (deg / 180.0) * 1900.0;
        let ticks = (us / 20000.0 * 4096.0) as u16;
        let base = 0x06 + 4 * ch;
        d.start();
        d.write(base);
        d.write(0x00);
        d.write(0x00);
        d.write((ticks & 0xFF) as u8);
        d.write(((ticks >> 8) & 0x0F) as u8);
    }

    #[test]
    fn observable_none_before_write_then_tracks_angle() {
        let mut d = IrI2cComponent::new(pca_like_with_observable(), None).unwrap();
        assert_eq!(d.observable("servo_angle", 8), None);
        d.start();
        d.write(0x00);
        d.write(0xA1); // enable AI
        ir_set_angle(&mut d, 8, 15.0);
        let deg = d.observable("servo_angle", 8).expect("set");
        assert!((deg - 15.0).abs() < 1.5, "expected ~15°, got {deg}");
    }

    #[test]
    fn observable_unknown_name_or_channel_is_none() {
        let d = IrI2cComponent::new(pca_like_with_observable(), None).unwrap();
        assert_eq!(d.observable("nope", 0), None);
        assert_eq!(d.observable("servo_angle", 16), None); // channels = 16 → 0..=15
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd core && cargo test -p labwired-core ir_component`
Expected: compile FAIL — `observable` not defined.

- [ ] **Step 3: Implement observables**

Add to the second `impl IrI2cComponent` block:

```rust
    /// Read a named observable for `channel`. `None` when the observable or
    /// channel doesn't exist, or when `none_when_raw_zero` applies.
    pub fn observable(&self, name: &str, channel: u8) -> Option<f32> {
        let o = self.spec.observables.iter().find(|o| o.name == name)?;
        if channel >= o.channels {
            return None;
        }
        let base = (o.base + o.stride * channel as u64) as usize;
        let IrObservableValue::U12Compose { lo_rel, hi_rel, hi_mask } = o.value;
        let lo = self.regs[base + lo_rel as usize] as u16;
        let hi = (self.regs[base + hi_rel as usize] & hi_mask) as u16;
        let raw = (hi << 8) | lo;
        match &o.map {
            None => Some(raw as f32),
            Some(m) => {
                if m.none_when_raw_zero && raw == 0 {
                    return None;
                }
                let mut v = raw as f32 * m.linear.scale + m.linear.offset;
                if let Some((lo, hi)) = m.linear.clamp {
                    v = v.clamp(lo, hi);
                }
                Some(v)
            }
        }
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd core && cargo test -p labwired-core ir_component`
Expected: PASS (7 tests). The linear coefficients derive from the firmware mapping: `deg = (raw/4096*20000 − 500)/1900*180` ⇒ `scale = 20000/4096/1900*180 ≈ 0.46258224`, `offset = −500/1900*180 ≈ −47.368423`.

- [ ] **Step 5: Commit**

```bash
git -C core add crates/core/src/peripherals/components/ir_component.rs
git -C core commit -m "feat(core): named channel observables on IR components"
```

---

### Task 5: PCA9685 spec asset + byte-equivalence gate

**Files:**
- Create: `core/configs/components/pca9685.yaml`
- Create: `core/crates/core/tests/ir_component_equivalence.rs`

- [ ] **Step 1: Write the spec asset**

`core/configs/components/pca9685.yaml`:

```yaml
# NXP PCA9685 16-channel 12-bit I2C PWM controller — declarative IR spec.
# Behavioral twin of crates/core/src/peripherals/components/pca9685.rs;
# the equivalence test in tests/ir_component_equivalence.rs pins them together.
name: PCA9685
vendor: NXP
datasheet: "PCA9685 Rev. 4"
kind: declarative
interface:
  i2c:
    default_address: 0x40
register_file:
  size: 256
  reset:
    0x00: 0x11   # MODE1 power-on: SLEEP | ALLCALL
pointer:
  first_write_after_start_sets_pointer: true
  auto_increment:
    when_field_set: { reg: 0x00, mask: 0x20 }   # MODE1.AI
observables:
  - name: servo_angle
    channels: 16
    base: 0x06       # LED0_ON_L
    stride: 4
    value:
      u12_compose: { lo_rel: 2, hi_rel: 3, hi_mask: 0x0F }   # OFF_L/OFF_H
    map:
      linear: { scale: 0.46258224, offset: -47.368423, clamp: [0.0, 180.0] }
      none_when_raw_zero: true
```

- [ ] **Step 2: Write the failing equivalence test**

`core/crates/core/tests/ir_component_equivalence.rs`:

```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Byte-equivalence gate: the IR-interpreted PCA9685 must be
//! indistinguishable from the hand-written Rust model over the I2cDevice
//! interface — same bytes read, same observables — across a deterministic
//! transaction corpus including the firmware's dispense sequences.

use labwired_core::peripherals::components::{IrI2cComponent, Pca9685};
use labwired_core::peripherals::i2c::I2cDevice;

fn ir_pca() -> IrI2cComponent {
    let yaml = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../configs/components/pca9685.yaml"
    ))
    .expect("spec asset");
    IrI2cComponent::new(serde_yaml::from_str(&yaml).expect("parse"), None).expect("valid")
}

/// One bus op. Deterministic corpus only — no randomness.
enum Op {
    Start,
    Write(u8),
    Read,
}

fn run_corpus(ops: &[Op]) {
    let mut rust: Box<dyn I2cDevice> = Box::new(Pca9685::new());
    let mut ir = ir_pca();
    assert_eq!(rust.address(), ir.address(), "address");
    for (i, op) in ops.iter().enumerate() {
        match op {
            Op::Start => {
                rust.start();
                ir.start();
            }
            Op::Write(b) => {
                rust.write(*b);
                ir.write(*b);
            }
            Op::Read => {
                assert_eq!(rust.read(), ir.read(), "read divergence at op {i}");
            }
        }
    }
    // Observables must agree with the Rust model's accessors on every channel.
    let rust_concrete = rust.as_any().unwrap().downcast_ref::<Pca9685>().unwrap();
    for ch in 0..16u8 {
        let a = rust_concrete.channel_angle_deg(ch);
        let b = ir.observable("servo_angle", ch);
        match (a, b) {
            (None, None) => {}
            (Some(x), Some(y)) => assert!((x - y).abs() < 0.01, "ch {ch}: {x} vs {y}"),
            _ => panic!("ch {ch}: presence mismatch {a:?} vs {b:?}"),
        }
    }
}

fn set_angle_ops(ops: &mut Vec<Op>, ch: u8, deg: f64) {
    let us = 500.0 + (deg / 180.0) * 1900.0;
    let ticks = (us / 20000.0 * 4096.0) as u16;
    ops.push(Op::Start);
    ops.push(Op::Write(0x06 + 4 * ch));
    ops.push(Op::Write(0x00));
    ops.push(Op::Write(0x00));
    ops.push(Op::Write((ticks & 0xFF) as u8));
    ops.push(Op::Write(((ticks >> 8) & 0x0F) as u8));
}

#[test]
fn dispense_sequence_is_byte_equivalent() {
    let mut ops = vec![Op::Start, Op::Write(0x00), Op::Write(0xA1)]; // AI on
    set_angle_ops(&mut ops, 8, 15.0); // revolver → compartment 1
    set_angle_ops(&mut ops, 12, 20.0); // shutter closed
    set_angle_ops(&mut ops, 12, 90.0); // shutter open
    set_angle_ops(&mut ops, 8, 135.0); // revolver → compartment 5
    // Read back the channel-8 block through AI.
    ops.push(Op::Start);
    ops.push(Op::Write(0x06 + 4 * 8));
    for _ in 0..4 {
        ops.push(Op::Read);
    }
    run_corpus(&ops);
}

#[test]
fn pointer_semantics_without_ai_are_byte_equivalent() {
    // AI off (power-on MODE1=0x11): repeated reads hit the same register.
    let ops = vec![
        Op::Start,
        Op::Write(0x00), // pointer = MODE1
        Op::Read,
        Op::Read,
        Op::Start,
        Op::Write(0x06),
        Op::Write(0x55), // data write with AI off
        Op::Write(0x66), // overwrites same register
        Op::Start,
        Op::Write(0x06),
        Op::Read,
    ];
    run_corpus(&ops);
}

#[test]
fn full_register_sweep_is_byte_equivalent() {
    // Walk every register: write a deterministic pattern with AI on, then
    // read the whole file back and compare byte-for-byte.
    let mut ops = vec![Op::Start, Op::Write(0x00), Op::Write(0xA1)];
    ops.push(Op::Start);
    ops.push(Op::Write(0x01)); // start after MODE1 to keep AI set
    for i in 1..=255u32 {
        ops.push(Op::Write((i.wrapping_mul(37) & 0xFF) as u8));
    }
    ops.push(Op::Start);
    ops.push(Op::Write(0x00));
    for _ in 0..=255 {
        ops.push(Op::Read);
    }
    run_corpus(&ops);
}
```

- [ ] **Step 3: Run to verify current state**

Run: `cd core && cargo test -p labwired-core --test ir_component_equivalence`
Expected: PASS if Tasks 3–4 are correct. If any test FAILS, the interpreter diverges from the reference model — fix the interpreter (never the test or the Rust model) until byte-equal. This is the slice's validation gate from the spec.

- [ ] **Step 4: Commit**

```bash
git -C core add configs/components/pca9685.yaml crates/core/tests/ir_component_equivalence.rs
git -C core commit -m "test(core): PCA9685 IR spec byte-equivalence gate vs Rust model"
```

---

### Task 6: TMP102 spec — wide registers + update rules prove generality

**Files:**
- Create: `core/configs/components/tmp102.yaml`
- Modify: `core/crates/core/tests/ir_component_equivalence.rs`

- [ ] **Step 1: Write the spec asset**

`core/configs/components/tmp102.yaml`:

```yaml
# TI TMP102 I2C temperature sensor — declarative IR spec.
# Behavioral twin of crates/core/src/peripherals/esp32s3/tmp102.rs, including
# the demo drift (+0.5 °C per full temperature read, wrap >35 °C → 20 °C).
name: TMP102
vendor: Texas Instruments
datasheet: "TMP102 SBOS397"
kind: declarative
interface:
  i2c:
    default_address: 0x48
register_file:
  size: 4
pointer:
  first_write_after_start_sets_pointer: true
  pointer_mask: 0x03
  auto_increment: never
wide_registers:
  - { pointer: 0, bits: 16, reset: 0x1900 }   # temperature, 25.0 °C
  - { pointer: 1, bits: 16, reset: 0x60A0 }   # config
  - { pointer: 2, bits: 16, reset: 0x4B00 }   # T_LOW
  - { pointer: 3, bits: 16, reset: 0x5000 }   # T_HIGH
updates:
  - trigger: { wide_read_complete: { pointer: 0 } }
    action:  { add_wrap: { add: 0x80, max: 0x2300, reset: 0x1400 } }
```

- [ ] **Step 2: Write the failing equivalence test**

Append to `ir_component_equivalence.rs`:

```rust
use labwired_core::peripherals::esp32s3::tmp102::Tmp102;

fn ir_tmp102() -> IrI2cComponent {
    let yaml = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../configs/components/tmp102.yaml"
    ))
    .expect("spec asset");
    IrI2cComponent::new(serde_yaml::from_str(&yaml).expect("parse"), None).expect("valid")
}

#[test]
fn tmp102_temperature_reads_and_drift_are_byte_equivalent() {
    let mut rust = Tmp102::new();
    let mut ir = ir_tmp102();
    assert_eq!(rust.address(), ir.address());
    // 60 full temperature reads: crosses the 35 °C wrap at least once
    // ((0x2300-0x1900)/0x80 = 20 reads to first wrap).
    for i in 0..60 {
        rust.start();
        ir.start();
        rust.write(0x00);
        ir.write(0x00);
        for half in 0..2 {
            assert_eq!(rust.read(), ir.read(), "read {i}.{half}");
        }
    }
    // Config / T_LOW / T_HIGH read back identically (MSB then LSB).
    for ptr in 1..=3u8 {
        rust.start();
        ir.start();
        rust.write(ptr);
        ir.write(ptr);
        assert_eq!(rust.read(), ir.read(), "ptr {ptr} MSB");
        assert_eq!(rust.read(), ir.read(), "ptr {ptr} LSB");
    }
}
```

Check the import path compiles: `Tmp102` lives at `labwired_core::peripherals::esp32s3::tmp102::Tmp102`. If the module isn't `pub`, make `tmp102` public in `core/crates/core/src/peripherals/esp32s3/mod.rs` rather than re-exporting elsewhere.

- [ ] **Step 3: Run to verify**

Run: `cd core && cargo test -p labwired-core --test ir_component_equivalence`
Expected: PASS (4 tests). Divergence rule as in Task 5: fix the interpreter, never the reference.

- [ ] **Step 4: Commit**

```bash
git -C core add configs/components/tmp102.yaml crates/core/tests/ir_component_equivalence.rs
git -C core commit -m "test(core): TMP102 IR spec equivalence (wide registers + drift update rule)"
```

---

### Task 7: Manifest integration — `type: ir` in the I2C factory

**Files:**
- Modify: `core/crates/core/src/peripherals/components/i2c_factory.rs`

- [ ] **Step 1: Write the failing tests**

Append to the tests module in `i2c_factory.rs`:

```rust
    #[test]
    fn ir_type_builds_from_spec_path() {
        let mut cfg = HashMap::new();
        cfg.insert(
            "spec_path".to_string(),
            serde_yaml::Value::String(
                concat!(env!("CARGO_MANIFEST_DIR"), "/../../configs/components/pca9685.yaml")
                    .to_string(),
            ),
        );
        let dev = build_i2c_device("ir", &cfg).expect("ir device should build");
        assert_eq!(dev.address(), 0x40);
    }

    #[test]
    fn ir_type_address_override_from_config() {
        let mut cfg = HashMap::new();
        cfg.insert(
            "spec_path".to_string(),
            serde_yaml::Value::String(
                concat!(env!("CARGO_MANIFEST_DIR"), "/../../configs/components/pca9685.yaml")
                    .to_string(),
            ),
        );
        cfg.insert(
            "i2c_address".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(0x41)),
        );
        let dev = build_i2c_device("ir", &cfg).expect("ir device should build");
        assert_eq!(dev.address(), 0x41);
    }

    #[test]
    fn ir_type_missing_or_bad_spec_returns_none() {
        // Missing spec_path.
        assert!(build_i2c_device("ir", &HashMap::new()).is_none());
        // Nonexistent file.
        let mut cfg = HashMap::new();
        cfg.insert(
            "spec_path".to_string(),
            serde_yaml::Value::String("/nonexistent/spec.yaml".to_string()),
        );
        assert!(build_i2c_device("ir", &cfg).is_none());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd core && cargo test -p labwired-core i2c_factory`
Expected: new tests FAIL — `"ir"` falls through to `None` (first two) / third passes vacuously; verify the first two fail.

- [ ] **Step 3: Implement the `ir` arm**

In `build_i2c_device`, before the `_ => None` arm:

```rust
        "ir" => {
            let spec_path = config.get("spec_path").and_then(|v| v.as_str())?;
            let yaml = match std::fs::read_to_string(spec_path) {
                Ok(y) => y,
                Err(e) => {
                    eprintln!("ir component: cannot read {spec_path}: {e}");
                    return None;
                }
            };
            let spec: labwired_ir::component::IrComponent = match serde_yaml::from_str(&yaml) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("ir component: {spec_path} parse error: {e}");
                    return None;
                }
            };
            let address = config
                .get("i2c_address")
                .and_then(|v| v.as_u64())
                .map(|a| a as u8);
            match crate::peripherals::components::IrI2cComponent::new(spec, address) {
                Ok(d) => Some(Box::new(d)),
                Err(e) => {
                    eprintln!("ir component: {spec_path} invalid: {e}");
                    None
                }
            }
        }
```

(The factory's existing contract is `Option`, with `eprintln!` for diagnosis — match it; richer diagnostics surface through the CLI/MCP path in Tasks 8–9.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd core && cargo test -p labwired-core i2c_factory`
Expected: PASS (8 tests: 5 existing + 3 new).

- [ ] **Step 5: Commit**

```bash
git -C core add crates/core/src/peripherals/components/i2c_factory.rs
git -C core commit -m "feat(core): manifests instantiate IR components via type: ir + spec_path"
```

---

### Task 8: CLI — `labwired asset validate-component`

**Files:**
- Create: `core/crates/cli/src/component_validation.rs`
- Modify: `core/crates/cli/src/main.rs` (`AssetCommands` enum ~line 316, dispatch in `run_asset` ~line 2586, `mod` list ~line 19)

- [ ] **Step 1: Write the failing test**

`component_validation.rs`:

```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `labwired asset validate-component <spec.yaml> [--json]`
//!
//! Validates an IrComponent spec and prints diagnostics. Exit code 0 when
//! clean, 1 when the file is unreadable/unparsable or has diagnostics.
//! `--json` emits `{ "ok": bool, "name": string|null, "diagnostics": [...] }`
//! on stdout for the MCP server.

use clap::Args;
use labwired_ir::component::{IrComponent, IrComponentDiag};
use std::process::ExitCode;

#[derive(Args, Debug)]
pub struct ValidateComponentArgs {
    /// Path to the component spec YAML.
    pub spec: std::path::PathBuf,
    /// Emit machine-readable JSON on stdout.
    #[arg(long)]
    pub json: bool,
}

#[derive(serde::Serialize)]
struct JsonReport {
    ok: bool,
    name: Option<String>,
    diagnostics: Vec<IrComponentDiag>,
}

pub fn run_validate_component(args: ValidateComponentArgs) -> ExitCode {
    let (report, code) = build_report(&args.spec);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report).expect("serialize"));
    } else if report.ok {
        println!("OK: {}", report.name.as_deref().unwrap_or("?"));
    } else {
        for d in &report.diagnostics {
            eprintln!("{}: {} (hint: {})", d.code, d.message, d.hint);
        }
    }
    code
}

fn build_report(path: &std::path::Path) -> (JsonReport, ExitCode) {
    let io_diag = |code: &str, message: String| JsonReport {
        ok: false,
        name: None,
        diagnostics: vec![IrComponentDiag {
            code: code.into(),
            message,
            hint: "Check the file path and YAML syntax".into(),
        }],
    };
    let yaml = match std::fs::read_to_string(path) {
        Ok(y) => y,
        Err(e) => return (io_diag("ICOMP_READ_ERROR", e.to_string()), ExitCode::from(1)),
    };
    let spec: IrComponent = match serde_yaml::from_str(&yaml) {
        Ok(s) => s,
        Err(e) => return (io_diag("ICOMP_PARSE_ERROR", e.to_string()), ExitCode::from(1)),
    };
    let diagnostics = spec.validate();
    let ok = diagnostics.is_empty();
    (
        JsonReport { ok, name: Some(spec.name), diagnostics },
        if ok { ExitCode::SUCCESS } else { ExitCode::from(1) },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_spec_reports_ok() {
        let (r, _) = build_report(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../configs/components/pca9685.yaml"
        )));
        assert!(r.ok);
        assert_eq!(r.name.as_deref(), Some("PCA9685"));
        assert!(r.diagnostics.is_empty());
    }

    #[test]
    fn missing_file_reports_read_error() {
        let (r, _) = build_report(std::path::Path::new("/nonexistent/spec.yaml"));
        assert!(!r.ok);
        assert_eq!(r.diagnostics[0].code, "ICOMP_READ_ERROR");
    }

    #[test]
    fn invalid_spec_reports_diagnostics() {
        let dir = std::env::temp_dir().join("labwired_icomp_test");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("bad.yaml");
        std::fs::write(
            &p,
            "name: Bad\nkind: wasm\ninterface: { i2c: { default_address: 0x40 } }\nregister_file: { size: 256 }\n",
        )
        .unwrap();
        let (r, _) = build_report(&p);
        assert!(!r.ok);
        assert!(r.diagnostics.iter().any(|d| d.code == "ICOMP_WASM_UNSUPPORTED"));
    }
}
```

Note: `IrComponentDiag` must derive `Serialize`/`Deserialize` — it already does (Task 2). If `serde_json` is not yet a `labwired-cli` dependency, add `serde_json = "1"` to `core/crates/cli/Cargo.toml` (it almost certainly is — check first).

- [ ] **Step 2: Wire into clap and run tests**

In `main.rs`: add `mod component_validation;` next to `mod asset_validation;` (~line 19); add to `AssetCommands` (~line 316):

```rust
    /// Validate an off-chip component IR spec (YAML).
    ValidateComponent(component_validation::ValidateComponentArgs),
```

and to the `run_asset` dispatch (~line 2590):

```rust
        AssetCommands::ValidateComponent(a) => component_validation::run_validate_component(a),
```

Run: `cd core && cargo test -p labwired-cli component_validation`
Expected: PASS (3 tests).

- [ ] **Step 3: Smoke the binary end-to-end**

Run: `cd core && cargo run -p labwired-cli -- asset validate-component configs/components/pca9685.yaml --json`
Expected: JSON with `"ok": true`, `"name": "PCA9685"`, empty diagnostics, exit 0.

- [ ] **Step 4: Commit**

```bash
git -C core add crates/cli/src/component_validation.rs crates/cli/src/main.rs crates/cli/Cargo.toml
git -C core commit -m "feat(cli): asset validate-component with JSON diagnostics"
```

---

### Task 9: MCP — `labwired_define_component`

**Files:**
- Modify: `packages/mcp/src/index.ts` (tool registration + handler)
- Modify: `packages/mcp/src/tool-metadata.ts` (title/annotations)
- Modify: `packages/mcp/src/cli.test.ts` (coverage)

Follow the registration pattern of the existing tools in `index.ts` exactly (schema shape, CLI shell-out via the existing exec helper, error JSON style). The tool contract:

- **Name:** `labwired_define_component` — title "Define Component", `readOnlyHint: false`, `destructiveHint: false`.
- **Input schema:**

```json
{
  "type": "object",
  "required": ["spec_yaml"],
  "properties": {
    "spec_yaml": { "type": "string", "description": "IrComponent spec as YAML" },
    "name": { "type": "string", "description": "Override file name (defaults to spec name, kebab-cased)" }
  }
}
```

- **Behavior:** write `spec_yaml` to a temp file; run `labwired asset validate-component <tmp> --json`; on `ok: false` return the diagnostics JSON as a tool error (same style as `labwired_validate_diagram`). On `ok: true`, persist to `<workspace>/.labwired/components/<name>.yaml` (workspace = `LABWIRED_REPO_ROOT` or cwd, matching how `boards.ts` resolves the repo root) and return:

```json
{
  "ok": true,
  "name": "PCA9685",
  "spec_path": "/abs/path/.labwired/components/pca9685.yaml",
  "usage": {
    "manifest_external_device": {
      "type": "ir",
      "connection": "<i2c peripheral id>",
      "config": { "spec_path": "/abs/path/.labwired/components/pca9685.yaml" }
    }
  }
}
```

- [ ] **Step 1: Write the failing test**

In `cli.test.ts`, following the existing stdio round-trip test pattern in that file:

```typescript
describe("labwired_define_component", () => {
  it("is advertised with title and annotations", async () => {
    const tools = await listToolsViaStdio(); // reuse the file's existing helper
    const tool = tools.find((t: any) => t.name === "labwired_define_component");
    expect(tool).toBeDefined();
    expect(tool.title).toBe("Define Component");
    expect(tool.annotations.readOnlyHint).toBe(false);
  });

  it("rejects an invalid spec with machine-readable diagnostics", async () => {
    const result = await callToolViaStdio("labwired_define_component", {
      spec_yaml:
        "name: Bad\nkind: wasm\ninterface: { i2c: { default_address: 0x40 } }\nregister_file: { size: 256 }\n",
    });
    expect(result.isError).toBe(true);
    expect(JSON.stringify(result.content)).toContain("ICOMP_WASM_UNSUPPORTED");
  });

  it("persists a valid spec and returns spec_path + manifest usage", async () => {
    const specYaml = [
      "name: TCA9999",
      "kind: declarative",
      "interface: { i2c: { default_address: 0x20 } }",
      "register_file: { size: 8 }",
    ].join("\n");
    const result = await callToolViaStdio("labwired_define_component", {
      spec_yaml: specYaml,
    });
    expect(result.isError).toBeFalsy();
    const body = JSON.parse(result.content[0].text);
    expect(body.ok).toBe(true);
    expect(body.spec_path).toMatch(/\.labwired\/components\/tca9999\.yaml$/);
    expect(body.usage.manifest_external_device.type).toBe("ir");
  });
});
```

Adapt the two helper names to whatever `cli.test.ts` actually uses for stdio round trips (it has existing list/call helpers — reuse them; do not spawn your own server plumbing). These tests shell out to the real CLI, consistent with how the existing run-tool tests work; they require `cargo build -p labwired-cli` first.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd packages/mcp && npm test -- --run`
Expected: the three new tests FAIL (tool not registered).

- [ ] **Step 3: Implement**

Register the tool in `index.ts` and add metadata in `tool-metadata.ts`, exactly mirroring the structure used for `labwired_validate_diagram` (the closest existing tool: validation + JSON diagnostics). Implementation steps inside the handler:

1. `mkdtemp` + write `spec_yaml`.
2. Spawn the CLI: `labwired asset validate-component <tmp> --json` (reuse the existing CLI exec helper and `LABWIRED_CLI` env resolution).
3. Parse stdout JSON. If `ok === false` → tool error with the diagnostics array as JSON text.
4. Else compute file name: `name` arg, else `report.name` lower-cased with non-alphanumerics → `-`. Write to `<repoRoot>/.labwired/components/<file>.yaml` (create dirs). Return the success body shown in the contract above.
5. Always clean up the temp file in a `finally`.

Also add `labwired_define_component` to the search-tool corpus in `search-tools.ts` (same keyword-listing pattern as the other entries: "define component", "ir spec", "custom device", "sensor model").

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd core && cargo build -p labwired-cli && cd ../packages/mcp && npm test -- --run`
Expected: PASS including the three new tests.

- [ ] **Step 5: Commit**

```bash
git add packages/mcp/src/index.ts packages/mcp/src/tool-metadata.ts packages/mcp/src/search-tools.ts packages/mcp/src/cli.test.ts
git commit -m "feat(mcp): labwired_define_component — agent-authored IR device specs"
```

---

### Task 10: Surface updates, docs, full verification

**Files:**
- Modify: `packages/mcp/src/resources/labwired-agent-hardware-loop.md`
- Modify: `packages/mcp/src/component-meta.ts` (only if it enumerates component types — check; if it lists the factory's `type:` strings, add `ir`)
- Modify: `packages/mcp/README.md` (tool table + status)
- Modify: `CHANGELOG.md`
- Modify: root repo submodule pointer for `core` (if applicable)

- [ ] **Step 1: Update the agent guide resource**

In `labwired-agent-hardware-loop.md`, after the "Discover modeled components" step, add a "Define missing components" section:

```markdown
## Define missing components

If a part is not in `labwired_list_components`, define it yourself with
`labwired_define_component`: submit a declarative IR spec (register file,
pointer rule, observables) derived from the part's datasheet. The tool
validates the spec (stable `ICOMP_*` diagnostic codes with hints) and returns
a `spec_path` plus the exact `external_devices` manifest entry to use
(`type: ir`, `config.spec_path`). Reference specs:
`core/configs/components/pca9685.yaml` (pointer + auto-increment + observables)
and `core/configs/components/tmp102.yaml` (16-bit phased reads + update rules).
```

Mirror the same edit in `packages/api/src/mcp/resources/labwired-agent-hardware-loop.md` so hosted and local guides stay in sync (the hosted tool itself ships later; the guide may note "local MCP only for now").

- [ ] **Step 2: Update README + CHANGELOG**

`packages/mcp/README.md`: add `labwired_define_component` to the core-workflow tool table ("Define a new off-chip device from a declarative IR spec; validated, persisted, ready to wire into a manifest") and a status bullet. `CHANGELOG.md`: one entry — "Component Model IR: agents define off-chip I2C devices declaratively (`labwired_define_component`, `labwired asset validate-component`, `type: ir` manifest devices); PCA9685 and TMP102 reference specs gated by byte-equivalence tests."

- [ ] **Step 3: Full verification**

```bash
cd core && cargo fmt --all --check && cargo clippy --workspace -- -D warnings && cargo test --workspace
cd ../packages/mcp && npm test -- --run && npm run build
```

Expected: all green. Fix anything red before committing (never weaken a gate to pass).

- [ ] **Step 4: Commit (and submodule bump if root tracks core)**

```bash
git add packages/mcp/src/resources/labwired-agent-hardware-loop.md packages/api/src/mcp/resources/labwired-agent-hardware-loop.md packages/mcp/README.md CHANGELOG.md
git add core   # submodule pointer, if root repo tracks core as a submodule
git commit -m "docs: component IR agent guide, README, changelog"
```

---

## Self-review notes

- Spec coverage: data model + validation (Tasks 1–2), deterministic interpreter (3–4), PCA9685 equivalence gate (5), generality via second part (6 — TMP102 instead of BMP280: it exercises two extra primitives, wide reads + updates, which BMP280's calibration blob would not), manifest/factory integration (7), CLI + MCP define_component with persistence and diagnostics (8–9), guide/docs (10). WASM: reserved `kind` enum + `ICOMP_WASM_UNSUPPORTED` only, per spec.
- Determinism: interpreter has no clock/randomness/I/O; VCD-hash gating needs no new work because IR devices only react to bus traffic.
- Known check-at-implementation points are called out inline (module visibility of `tmp102`, `serde_json` in cli Cargo.toml, exact helper names in `cli.test.ts`, whether `component-meta.ts` enumerates types, submodule layout).
- Out of scope (later slices/plans): diagram→manifest compilation, hosted-MCP define_component, SPI/GPIO interfaces, WASM execution, `labwired_list_components` IR-flagging beyond the `.labwired/components/` listing if that file layout makes it automatic.
