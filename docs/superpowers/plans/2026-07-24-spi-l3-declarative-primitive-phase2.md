# SPI L3 Declarative Primitive — Phase 2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Register the first two real declarative SPI parts — **ADXL345** accelerometer and **MAX31855** thermocouple — as datasheet-shaped YAML kits, adding the two engine capabilities their honest models require: signed two's-complement register words and bit-field-within-word assembly.

**Architecture:** Builds on Phase 1 (declarative `spi_device` primitive, branch `feat/spi-l3-declarative`, this branch `feat/spi-l3-parts` is stacked on it). Two engine additions land first (signed words, then multi-field bit packing) in the shared `declarative_regs.rs`/schema so both the I²C and SPI engines gain them; then each part is a `configs/devices/*.yaml` + an `embedded_device_yaml` arm + a `LazyLock<DeclarativeSpiKit>` static + one `registry::KITS` line, mirroring the I²C `SHT31_KIT`/`BH1750_KIT` pattern. Registering parts changes the peripherals manifest — that is intended here (unlike Phase 1) and the vendored snapshot is regenerated.

**Tech Stack:** Rust (`labwired_config`, `labwired-core`), serde/serde_yaml, anyhow.

## Global Constraints

- **License header** on any new `.rs` file (none expected — engine edits are in existing files):
  `// LabWired - Firmware Simulation Platform` / `// Copyright (C) 2026 Andrii Shylenko` / `// SPDX-License-Identifier: MIT`.
- **Honesty (FIDELITY.md):** a modeled register value must be byte-faithful to the datasheet. No clamping a signed quantity to zero, no fabricated fields. Where a nuance is deliberately not modeled (ADXL345 MB auto-increment bit; fixed-resolution LSB/g scaling; MAX31855 internal-temp as a second input), say so in a YAML comment.
- **Backward compatibility:** the new schema fields (`signed`, `fields`) are `#[serde(default)]` and OFF by default, so every existing I²C descriptor and the Phase-1 SPI fixture parse and behave identically. The full I²C suite (~26 tests) and Phase-1 SPI suite (12 tests) are the regression guard and MUST stay green after every task.
- **Contiguous-block rule (Phase-1 finding N1):** ADXL345's data registers are contiguous by width (0x32–0x37); keep them so. Do not register a part with address gaps inside a burst-read block.
- **Manifest regen:** after registering parts, regenerate the vendored snapshot with
  `cargo run -p labwired-cli --bin gen-peripherals-manifest -- --out crates/core/tests/fixtures/peripherals/manifest.json`
  and commit it. The gate is `crates/core/tests/peripheral_kit_gate.rs::manifest_json_matches_registry`.
- No new dependencies. Conventional-commit subjects, no AI/Claude references or trailers. TDD throughout.
- Work in worktree `../labwired-core-spi-parts` on branch `feat/spi-l3-parts` (stacked on `feat/spi-l3-declarative`).

**Reference (read before starting):**
- `crates/core/src/peripherals/components/declarative_regs.rs` — shared `encode_raw`/`pack`/`register_read_bytes` to extend.
- `crates/core/src/peripherals/components/declarative_spi.rs` — Phase-1 SPI engine + `DeclarativeSpiKit`; the `SHT31_KIT`-style `LazyLock` pattern lives in `declarative_i2c.rs` (copy it for SPI).
- `configs/devices/sht31.yaml` — exemplar declarative descriptor + `embedded_device_yaml` registration + `registry::KITS` line for I²C parts.

---

## File Structure

| Path | Responsibility | Task |
|------|----------------|------|
| `crates/config/src/lib.rs` | Add `signed: bool` to `RegisterSpec`; add `FieldSpec` + `fields: Vec<FieldSpec>` to `RegisterSpec`. | 1, 2 |
| `crates/core/src/peripherals/components/declarative_regs.rs` | Signed two's-complement in `encode_raw`; multi-field word assembly in `register_read_bytes`. | 1, 2 |
| `configs/devices/adxl345.yaml` (new) | ADXL345 declarative SPI descriptor. | 3 |
| `configs/devices/max31855.yaml` (new) | MAX31855 declarative SPI descriptor (command_bytes:0, bit-fields). | 4 |
| `crates/config/src/lib.rs` (`embedded_device_yaml`) | `include_str!` arms for both parts. | 3, 4 |
| `crates/core/src/peripherals/components/declarative_spi.rs` | `ADXL345_KIT` + `MAX31855_KIT` `LazyLock<DeclarativeSpiKit>` statics. | 3, 4 |
| `crates/core/src/peripherals/kit/registry.rs` | Register both kits in `KITS`. | 3, 4 |
| `crates/core/tests/fixtures/peripherals/manifest.json` | Regenerated snapshot. | 3, 4 |

---

## Task 1: Signed two's-complement register words

**Files:** `crates/config/src/lib.rs`, `crates/core/src/peripherals/components/declarative_regs.rs`

**Interfaces:**
- Produces: `RegisterSpec.signed: bool` (`#[serde(default)]`, default false); `encode_raw(value, enc, extra_scale, width, signed)` gains a `signed: bool` param.

- [ ] **Step 1: Failing test** — in `declarative_regs.rs` tests, add:

```rust
#[test]
fn signed_negative_value_packs_twos_complement_le() {
    use labwired_config::{Endian, RegisterAccess, RegisterSpec};
    use std::collections::HashMap;
    let r = RegisterSpec {
        name: "DATAX".into(), addr: 0x32, width: 2, endian: Endian::Le,
        access: RegisterAccess::R, reset: 0, source: Some("ax".into()),
        encode: Some(labwired_config::Encode { scale: 256.0, offset: 0.0, clamp_min: None, clamp_max: None }),
        scale_from: None, signed: true, fields: vec![],
    };
    let mut slots = HashMap::new();
    slots.insert("ax".to_string(), -1.0); // -1 g × 256 = -256 = 0xFF00 two's-complement, LE
    assert_eq!(register_read_bytes(&r, &slots, &HashMap::new()), vec![0x00, 0xFF]);
}
```

- [ ] **Step 2: Run → fail** (`signed`/`fields` fields don't exist). `cargo test -p labwired-core signed_negative`.

- [ ] **Step 3: Schema** — add to `RegisterSpec` (after `scale_from`):

```rust
    /// The register word is a signed two's-complement quantity of `width`
    /// bytes. A negative sourced measurement is encoded as its two's-complement
    /// bit pattern rather than clamped to zero. Default false (unsigned).
    #[serde(default)]
    pub signed: bool,
    /// Bit-field composition: when non-empty, the register word is ASSEMBLED
    /// from these fields (each a sourced measurement placed at a bit offset)
    /// rather than from the single top-level `source`. Used by parts like the
    /// MAX31855 whose 32-bit frame packs temperature + status sub-fields.
    #[serde(default)]
    pub fields: Vec<FieldSpec>,
```

(Task 2 defines `FieldSpec`; add the `fields` field now with a temporary `Vec<FieldSpec>` — define `FieldSpec` as an empty-for-now struct in this task so it compiles, then flesh it out in Task 2. Alternatively define the full `FieldSpec` here; the test above needs `fields: vec![]` to construct.) Define the full `FieldSpec` struct now to avoid churn:

```rust
/// One sourced bit-field within a composite register word (see
/// [`RegisterSpec::fields`]). The encoded value occupies `width_bits` bits at
/// bit offset `shift`; `signed` packs negatives as two's-complement within
/// those bits.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FieldSpec {
    pub source: String,
    pub shift: u8,
    pub width_bits: u8,
    #[serde(default)]
    pub signed: bool,
    #[serde(default)]
    pub encode: Option<Encode>,
}
```

- [ ] **Step 4: Engine** — in `declarative_regs.rs`, change `encode_raw` to take `signed: bool` and, when signed and the rounded value is negative, mask it into `width` bytes as two's-complement instead of clamping to 0:

```rust
pub(crate) fn encode_raw(
    value: f64, enc: Option<&Encode>, extra_scale: f64, width: u8, signed: bool,
) -> u32 {
    let scale = enc.map(|e| e.scale).unwrap_or(1.0) * extra_scale;
    let offset = enc.map(|e| e.offset).unwrap_or(0.0);
    let mut raw = value * scale + offset;
    if let Some(e) = enc {
        if let Some(lo) = e.clamp_min { raw = raw.max(lo); }
        if let Some(hi) = e.clamp_max { raw = raw.min(hi); }
    }
    let bits = 8 * width as u32;
    let mask = if bits >= 32 { u32::MAX } else { (1u32 << bits) - 1 };
    if signed {
        let lo = -(2f64.powi((bits - 1) as i32));
        let hi = 2f64.powi((bits - 1) as i32) - 1.0;
        let v = raw.round().clamp(lo, hi) as i64;
        (v as u32) & mask
    } else {
        raw.round().clamp(0.0, width_max(width)) as u32
    }
}
```

Update `register_read_bytes` to pass `reg.signed` and update the I²C engine's other `encode_raw` call sites (in `declarative_i2c.rs`: `register_read_bytes` is now shared, but `response_word_raw` calls `encode_raw` directly — pass `false` there, commands are unsigned). Grep for every `encode_raw(` call and add the `signed` arg (`false` unless it's the register path).

- [ ] **Step 5: Run → pass.** `cargo test -p labwired-core signed_negative` then the guards: `cargo test -p labwired-core declarative_i2c declarative_spi declarative_regs` (all green — existing tests unaffected because `signed=false` preserves the old clamp path).

- [ ] **Step 6: Commit** — `git add` the two files; `feat(core): signed two's-complement register words for declarative engines`.

---

## Task 2: Bit-field word assembly

**Files:** `crates/core/src/peripherals/components/declarative_regs.rs` (schema `FieldSpec` already added in Task 1)

**Interfaces:** Consumes `RegisterSpec.fields`. When `fields` is non-empty, `register_read_bytes` assembles the word by OR-ing each field's encoded, sign-masked value shifted to its bit offset onto the register's `reset` base; the top-level `source`/`encode` is ignored (assert this in a validator or document it).

- [ ] **Step 1: Failing test** — MAX31855-shaped composite word:

```rust
#[test]
fn composite_fields_assemble_into_word() {
    use labwired_config::{Encode, Endian, FieldSpec, RegisterAccess, RegisterSpec};
    use std::collections::HashMap;
    // 32-bit BE frame: thermocouple °C at bits[31:18] signed 14-bit, 0.25°C/LSB
    // (scale 4.0); internal °C at bits[15:4] signed 12-bit, 0.0625°C/LSB (16.0).
    let r = RegisterSpec {
        name: "OUT".into(), addr: 0, width: 4, endian: Endian::Be,
        access: RegisterAccess::R, reset: 0, source: None, encode: None, scale_from: None,
        signed: false,
        fields: vec![
            FieldSpec { source: "tc".into(), shift: 18, width_bits: 14, signed: true,
                        encode: Some(Encode { scale: 4.0, offset: 0.0, clamp_min: None, clamp_max: None }) },
            FieldSpec { source: "internal".into(), shift: 4, width_bits: 12, signed: true,
                        encode: Some(Encode { scale: 16.0, offset: 0.0, clamp_min: None, clamp_max: None }) },
        ],
    };
    let mut slots = HashMap::new();
    slots.insert("tc".to_string(), 100.0);      // 100°C → 400 = 0x190 in bits[31:18]
    slots.insert("internal".to_string(), 25.0); // 25°C → 400 = 0x190 in bits[15:4]
    let b = register_read_bytes(&r, &slots, &HashMap::new());
    // word = (400 << 18) | (400 << 4) = 0x06400000 | 0x00001900 = 0x06401900, BE.
    assert_eq!(b, vec![0x06, 0x40, 0x19, 0x00]);
}

#[test]
fn composite_field_negative_temperature() {
    use labwired_config::{Encode, Endian, FieldSpec, RegisterAccess, RegisterSpec};
    use std::collections::HashMap;
    let r = RegisterSpec {
        name: "OUT".into(), addr: 0, width: 4, endian: Endian::Be,
        access: RegisterAccess::R, reset: 0, source: None, encode: None, scale_from: None,
        signed: false,
        fields: vec![FieldSpec { source: "tc".into(), shift: 18, width_bits: 14, signed: true,
            encode: Some(Encode { scale: 4.0, offset: 0.0, clamp_min: None, clamp_max: None }) }],
    };
    let mut slots = HashMap::new();
    slots.insert("tc".to_string(), -25.0); // -25°C → -100 → 14-bit two's-comp = 0x3F9C, <<18
    let b = register_read_bytes(&r, &slots, &HashMap::new());
    let word = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    assert_eq!((word >> 18) & 0x3FFF, 0x3F9C);
}
```

- [ ] **Step 2: Run → fail.**

- [ ] **Step 3: Engine** — in `register_read_bytes`, branch on `!reg.fields.is_empty()`:

```rust
pub(crate) fn register_read_bytes(
    reg: &RegisterSpec, slots: &HashMap<String, f64>, reg_values: &HashMap<String, u32>,
) -> Vec<u8> {
    if !reg.fields.is_empty() {
        let mut word = reg.reset;
        for f in &reg.fields {
            let value = slots.get(&f.source).copied().unwrap_or(0.0);
            // Encode into `width_bits` bits (byte-width ceil for the helper), then mask.
            let byte_w = f.width_bits.div_ceil(8);
            let raw = encode_raw(value, f.encode.as_ref(), 1.0, byte_w, f.signed);
            let mask = if f.width_bits >= 32 { u32::MAX } else { (1u32 << f.width_bits) - 1 };
            word |= (raw & mask) << f.shift;
        }
        return pack(word, reg.width, reg.endian);
    }
    let raw = if let Some(src) = &reg.source {
        let value = slots.get(src).copied().unwrap_or(0.0);
        let extra = scale_from_factor(reg, reg_values);
        encode_raw(value, reg.encode.as_ref(), extra, reg.width, reg.signed)
    } else {
        reg_values.get(&reg.name).copied().unwrap_or(reg.reset)
    };
    pack(raw, reg.width, reg.endian)
}
```

Note: `encode_raw` with `byte_w` may allow more bits than `width_bits` for signed clamping; the subsequent `& mask` truncates to `width_bits`, and the signed range clamp inside `encode_raw` uses `byte_w*8` bits. For a 14-bit field packed via a 2-byte (16-bit) `encode_raw`, a value in 14-bit range is unaffected; a value exceeding 14-bit range would be masked (acceptable — real parts saturate). If exact 14-bit saturation matters, clamp before: acceptable to leave for now (document).

- [ ] **Step 4: Run → pass**, then guards (`declarative_i2c`, `declarative_spi`, `declarative_regs`).

- [ ] **Step 5: Commit** — `feat(core): composite bit-field register word assembly`.

---

## Task 3: Register ADXL345

**Files:** `configs/devices/adxl345.yaml` (new), `crates/config/src/lib.rs` (embedded arm), `crates/core/src/peripherals/components/declarative_spi.rs` (static), `crates/core/src/peripherals/kit/registry.rs`, manifest snapshot.

- [ ] **Step 1: Descriptor** — `configs/devices/adxl345.yaml`:

```yaml
# Analog Devices ADXL345 3-axis accelerometer — declarative SPI register device.
# 4-wire SPI: command byte = [R/W(bit7) | MB(bit6) | addr(bits5:0)]; data is
# little-endian per axis. Modeled in full-resolution mode (256 LSB/g, 3.9 mg/LSB,
# constant across ranges) — the common driver default. NOT modeled: the MB
# multi-byte bit (our engine auto-increments unconditionally, which matches a
# burst read); fixed-10-bit range scaling; interrupts/FIFO.
type: adxl345

behavior:
  primitive: spi_device
  spi:
    framing: { command_bytes: 1, rw_bit: 7, rw_read_high: true, addr_mask: 0x3F, auto_increment: true }
    registers:
      - { name: DEVID,       addr: 0x00, width: 1, endian: le, access: r,  reset: 0xE5 }
      - { name: POWER_CTL,   addr: 0x2D, width: 1, endian: le, access: rw, reset: 0x00 }
      - { name: DATA_FORMAT, addr: 0x31, width: 1, endian: le, access: rw, reset: 0x00 }
      - { name: DATAX0, addr: 0x32, width: 2, endian: le, access: r, signed: true, source: accel_x, encode: { scale: 256.0 } }
      - { name: DATAY0, addr: 0x34, width: 2, endian: le, access: r, signed: true, source: accel_y, encode: { scale: 256.0 } }
      - { name: DATAZ0, addr: 0x36, width: 2, endian: le, access: r, signed: true, source: accel_z, encode: { scale: 256.0 } }

metadata:
  label: "ADXL345 accelerometer"
  summary: "3-axis ±16 g accelerometer over SPI (full-resolution, 256 LSB/g)."
  category: spi
  inputs:
    - { key: accel_x, label: "Accel X", unit: g, min: -16, max: 16, default: 0 }
    - { key: accel_y, label: "Accel Y", unit: g, min: -16, max: 16, default: 0 }
    - { key: accel_z, label: "Accel Z", unit: g, min: -16, max: 16, default: 1 }
```

- [ ] **Step 2: Embed** — add to `embedded_device_yaml` match in `lib.rs`:
`"adxl345" => Some(include_str!("../../../configs/devices/adxl345.yaml")),`
(verify the relative path matches the sht31 arm's depth.)

- [ ] **Step 3: Failing kit test** — in `declarative_spi.rs` tests:

```rust
#[test]
fn adxl345_kit_reads_devid_and_signed_axis() {
    let kit = DeclarativeSpiKit::from_yaml(
        labwired_config::embedded_device_yaml("adxl345").unwrap()).unwrap();
    assert_eq!(kit.metadata().device_type, "adxl345");
    // Build the device and read DEVID + a negative Z.
    let mut d = crate::peripherals::components::declarative_spi::GenericSpiDevice::from_yaml(
        labwired_config::embedded_device_yaml("adxl345").unwrap(), "PA4").unwrap();
    d.cs_select(); d.transfer(0x80); // read DEVID (0x00)
    assert_eq!(d.transfer(0x00), 0xE5); d.cs_release();
    d.set_input("accel_z", -1.0).unwrap();
    d.cs_select(); d.transfer(0x80 | 0x36); // read DATAZ0
    let lo = d.transfer(0x00); let hi = d.transfer(0x00); d.cs_release();
    assert_eq!(u16::from_le_bytes([lo, hi]), 0xFF00); // -256 two's-complement
}
```

- [ ] **Step 4: Static** — in `declarative_spi.rs`, mirror the I²C `SHT31_KIT` pattern:

```rust
use std::sync::LazyLock;

impl PeripheralKit for LazyLock<DeclarativeSpiKit> {
    fn metadata(&self) -> &'static KitMetadata { LazyLock::force(self).metadata() }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> Result<()> { LazyLock::force(self).attach(ctx) }
}

/// Analog Devices ADXL345 accelerometer (declarative `adxl345.yaml`).
pub static ADXL345_KIT: LazyLock<DeclarativeSpiKit> = LazyLock::new(|| {
    DeclarativeSpiKit::from_yaml(
        labwired_config::embedded_device_yaml("adxl345").expect("adxl345 descriptor embedded"))
        .expect("adxl345.yaml is a valid declarative spi descriptor")
});
```

- [ ] **Step 5: Register** — in `registry.rs`, add under a new "Declarative SPI devices" comment:
`&components::declarative_spi::ADXL345_KIT,`

- [ ] **Step 6: Regenerate manifest** —
`cargo run -p labwired-cli --bin gen-peripherals-manifest -- --out crates/core/tests/fixtures/peripherals/manifest.json`

- [ ] **Step 7: Run** — `cargo test -p labwired-core adxl345 peripheral_kit_gate` (kit test passes; manifest gate passes with the regenerated snapshot). Guards: `declarative_i2c declarative_spi`.

- [ ] **Step 8: Commit** — `feat(core): register ADXL345 declarative SPI accelerometer`.

---

## Task 4: Register MAX31855

**Files:** `configs/devices/max31855.yaml` (new), `lib.rs` embedded arm, `declarative_spi.rs` static, `registry.rs`, manifest snapshot.

- [ ] **Step 1: Descriptor** — `configs/devices/max31855.yaml`:

```yaml
# Maxim MAX31855 cold-junction-compensated K-type thermocouple-to-digital
# converter — declarative SPI, read-only (command_bytes: 0): CS↓ clocks out a
# 32-bit big-endian frame, no command byte. Frame layout (datasheet):
#   [31:18] 14-bit signed thermocouple °C, 0.25 °C/LSB   (scale 4.0)
#   [17]    reserved (0)
#   [16]    fault (0 = OK)
#   [15:4]  12-bit signed internal °C, 0.0625 °C/LSB     (scale 16.0)
#   [3]     reserved (0);  [2] SCV  [1] SCG  [0] OC faults (0 = OK)
# Modeled fault-free (fault + SCV/SCG/OC = 0). Internal (cold-junction) temp is
# a second stimulus input defaulting to 25 °C.
type: max31855

behavior:
  primitive: spi_device
  spi:
    framing: { command_bytes: 0 }
    registers:
      - name: OUTPUT
        addr: 0x00
        width: 4
        endian: be
        access: r
        fields:
          - { source: thermocouple, shift: 18, width_bits: 14, signed: true, encode: { scale: 4.0 } }
          - { source: internal,     shift: 4,  width_bits: 12, signed: true, encode: { scale: 16.0 } }

metadata:
  label: "MAX31855 thermocouple"
  summary: "K-type thermocouple-to-digital converter over SPI (read-only 32-bit frame)."
  category: spi
  inputs:
    - { key: thermocouple, label: "Thermocouple temp", unit: "°C", min: -270, max: 1372, default: 25 }
    - { key: internal,     label: "Internal (cold-junction) temp", unit: "°C", min: -40, max: 125, default: 25 }
```

- [ ] **Step 2: Embed** — `"max31855" => Some(include_str!("../../../configs/devices/max31855.yaml")),`

- [ ] **Step 3: Failing kit test** — in `declarative_spi.rs` tests:

```rust
#[test]
fn max31855_reads_composite_frame_no_command() {
    let mut d = crate::peripherals::components::declarative_spi::GenericSpiDevice::from_yaml(
        labwired_config::embedded_device_yaml("max31855").unwrap(), "PA4").unwrap();
    d.set_input("thermocouple", 100.0).unwrap(); // 400 = 0x190 @ [31:18]
    d.set_input("internal", 25.0).unwrap();       // 400 = 0x190 @ [15:4]
    d.cs_select(); // command_bytes:0 → data phase immediately
    let b: Vec<u8> = (0..4).map(|_| d.transfer(0x00)).collect();
    d.cs_release();
    assert_eq!(b, vec![0x06, 0x40, 0x19, 0x00]);
    // A negative thermocouple reading sets the sign bits.
    d.set_input("thermocouple", -25.0).unwrap();
    d.cs_select();
    let n: Vec<u8> = (0..4).map(|_| d.transfer(0x00)).collect();
    d.cs_release();
    let word = u32::from_be_bytes([n[0], n[1], n[2], n[3]]);
    assert_eq!((word >> 18) & 0x3FFF, 0x3F9C); // -100 in 14-bit two's-complement
}
```

- [ ] **Step 4: Static** — add `MAX31855_KIT` `LazyLock<DeclarativeSpiKit>` (the `impl PeripheralKit for LazyLock<DeclarativeSpiKit>` already landed in Task 3):

```rust
/// Maxim MAX31855 thermocouple converter (declarative `max31855.yaml`).
pub static MAX31855_KIT: LazyLock<DeclarativeSpiKit> = LazyLock::new(|| {
    DeclarativeSpiKit::from_yaml(
        labwired_config::embedded_device_yaml("max31855").expect("max31855 descriptor embedded"))
        .expect("max31855.yaml is a valid declarative spi descriptor")
});
```

- [ ] **Step 5: Register** — `registry.rs`: `&components::declarative_spi::MAX31855_KIT,`

- [ ] **Step 6: Regenerate manifest** (same command as Task 3, Step 6).

- [ ] **Step 7: Run** — `cargo test -p labwired-core max31855 peripheral_kit_gate` + guards.

- [ ] **Step 8: Commit** — `feat(core): register MAX31855 declarative SPI thermocouple`.

---

## Task 5: Workspace gate

- [ ] **Step 1: Manifest fresh** — re-run the generator; `git diff --stat crates/core/tests/fixtures/peripherals/manifest.json` should be empty (Tasks 3/4 already regenerated). Confirm the two new device_types (`adxl345`, `max31855`) are present in the committed manifest.
- [ ] **Step 2: Full suite** — `cargo test -p labwired-config -p labwired-core` → 0 failures.
- [ ] **Step 3: fmt** — `cargo fmt --all`; commit any change. If `docs/coverage/chip-conformance.md` (a test-generated doc unrelated to this work) appears dirty, `git checkout --` it — do not commit it.
- [ ] **Step 4: clippy** — `cargo clippy -p labwired-config -p labwired-core --all-targets -- -D warnings` → clean.
- [ ] **Step 5: Commit** (only if fmt/clippy changed anything) — `style(core): fmt + clippy for declarative SPI parts`.

---

## Self-Review

- **Spec coverage:** signed words (Task 1) ✓; bit-field assembly (Task 2) ✓; ADXL345 register device (Task 3) ✓; MAX31855 read-only composite (Task 4) ✓; manifest regen + gate + full suite (Tasks 3/4/5) ✓. Both engine additions are `#[serde(default)]`-gated so I²C/Phase-1 behavior is unchanged (guard suites in every task).
- **Placeholder scan:** none — every code + YAML step is complete with real datasheet values.
- **Type consistency:** `RegisterSpec.signed`/`.fields`, `FieldSpec { source, shift, width_bits, signed, encode }`, `encode_raw(value, enc, extra_scale, width, signed)`, `ADXL345_KIT`/`MAX31855_KIT`, `embedded_device_yaml` arms `"adxl345"`/`"max31855"` are consistent across tasks.
- **Honesty notes:** ADXL345 MB-bit / fixed-resolution scaling and MAX31855 fault-free assumption are documented in the YAML comments per the FIDELITY constraint.
