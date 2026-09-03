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

use super::declarative_regs::{leak_labs, register_read_bytes, unpack};
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
    /// Explicit `cs_select()` is active. Soft-CS auto-restart must not fire
    /// while CS is held, or past-end bytes re-latch the same word (0x00) instead
    /// of returning 0xFF as an open bus.
    cs_held: bool,
    /// Bytes accumulated toward the current write register's width.
    write_acc: Vec<u8>,

    channels: &'static [InputChannel],
    component_id: Option<String>,
}

/// Validate the static descriptor contract for the `spi_device` primitive.
///
/// This stays separate from construction so manifest preflight can reject
/// malformed unused packs without allocating runtime input tables.
pub(crate) fn validate_descriptor(descriptor: &DeviceDescriptor) -> Result<()> {
    if descriptor.behavior.primitive != "spi_device" {
        bail!(
            "declarative spi kit requires behavior.primitive: spi_device, got '{}'",
            descriptor.behavior.primitive
        );
    }
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
    // `zero_when` power-gates work here too (the read math is shared with
    // the I²C engine), so a dangling reference must not silently no-op into
    // "gate never fires". No shipping SPI descriptor uses one yet; this is
    // the guard that keeps the first one from being wrong in silence.
    for reg in &spec.registers {
        if let Some(z) = &reg.zero_when {
            if !spec.registers.iter().any(|r| r.name == z.register) {
                bail!(
                    "register '{}' zero_when register '{}' is not a declared register",
                    reg.name,
                    z.register
                );
            }
            if z.mask == 0 {
                bail!(
                    "register '{}' zero_when mask is 0 — the gate could never fire",
                    reg.name
                );
            }
        }
    }
    Ok(())
}

impl GenericSpiDevice {
    pub fn from_descriptor(
        descriptor: &DeviceDescriptor,
        cs_pin: String,
        channels: &'static [InputChannel],
    ) -> Result<Self> {
        validate_descriptor(descriptor)?;
        let spec = descriptor
            .behavior
            .spi
            .as_ref()
            .context("declarative spi device is missing behavior.spi")?;
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
            cs_held: false,
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

    /// The register whose byte span covers `addr`, which is not always the one
    /// whose base equals it — see `build_read_buf`.
    fn find_register_containing(&self, addr: u8) -> Option<&RegisterSpec> {
        self.registers.iter().find(|r| {
            let base = u16::from(r.addr);
            let a = u16::from(addr);
            a >= base && a < base + u16::from(r.width)
        })
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
    /// Bytes a read beginning at `start` streams back.
    ///
    /// A read may begin *inside* a multi-byte register, because a datasheet
    /// numbers every byte of one as its own address: the ADXL345 declares
    /// DATAX0 at 0x32 and DATAX1 at 0x33 and says the low byte need not be read
    /// when it is not wanted, so pointing at 0x33 must return the high byte.
    ///
    /// Selecting on `addr >= start` did not do that. It skipped the register
    /// containing `start` and answered from the *next* one, so a firmware that
    /// read one axis byte on its own got a different register's value —
    /// 0x37 returned FIFO_CTL — with nothing to distinguish it from real data.
    /// Matching on the span and dropping the bytes before `start` serves the
    /// address the caller actually named.
    fn build_read_buf(&self, start: u8) -> Vec<u8> {
        let mut out = Vec::new();
        if self.framing.auto_increment {
            let mut regs: Vec<&RegisterSpec> = self
                .registers
                .iter()
                .filter(|r| u16::from(r.addr) + u16::from(r.width) > u16::from(start))
                .collect();
            regs.sort_by_key(|r| r.addr);
            for r in regs {
                let skip = usize::from(start.saturating_sub(r.addr));
                out.extend(
                    register_read_bytes(r, &self.slots, &self.reg_values)
                        .into_iter()
                        .skip(skip),
                );
            }
        } else if let Some(r) = self.find_register_containing(start) {
            let skip = usize::from(start - r.addr);
            out.extend(
                register_read_bytes(r, &self.slots, &self.reg_values)
                    .into_iter()
                    .skip(skip),
            );
        }
        out
    }

    /// Current engineering-unit value of a SimInput stimulus channel (the value
    /// last set via `set_input`, or the descriptor's declared default). Returns
    /// `None` if the device has no such channel. Lets consumers (e.g. the wasm
    /// inspector) read a declarative device's state without a concrete-type downcast.
    pub fn input_value(&self, key: &str) -> Option<f64> {
        self.slots.get(key).copied()
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
        self.cs_held = true;
        self.write_acc.clear();
        if self.framing.command_bytes == 0 {
            self.is_read = Some(true);
            self.cur_addr = Some(0);
        }
    }

    fn cs_release(&mut self) {
        self.cs_held = false;
        self.write_acc.clear();
    }

    fn transfer(&mut self, mosi: u8) -> u8 {
        // Soft-CS / matrix path: when CS was never held (or has been released),
        // enter the read-only data phase and re-frame after a full word so a
        // CS-high dummy flush does not permanently desync multi-byte reads.
        // While CS is held, past-end bytes stay 0xFF (open bus) — do not re-latch.
        if self.framing.command_bytes == 0 && !self.cs_held {
            let need_start =
                self.is_read.is_none() || (self.latched && self.read_idx >= self.read_buf.len());
            if need_start {
                // Soft-CS synthetic select: frame start without claiming hard CS.
                self.cmd_consumed = 0;
                self.is_read = Some(true);
                self.cur_addr = Some(0);
                self.read_buf.clear();
                self.read_idx = 0;
                self.latched = false;
                self.write_acc.clear();
            }
        }
        // Command phase.
        if self.framing.command_bytes > 0 && self.cmd_consumed < self.framing.command_bytes {
            self.cmd_consumed += 1;
            if self.cmd_consumed == self.framing.command_bytes {
                if let Some(bit) = self.framing.rw_bit {
                    let set = (mosi >> bit) & 1 == 1;
                    self.is_read = Some(set == self.framing.rw_read_high);
                }
                self.cur_addr = Some((mosi >> self.framing.addr_shift) & self.framing.addr_mask);
            }
            return 0x00;
        }
        // Data phase.
        let addr = self.cur_addr.unwrap_or(0);
        // Writes require an explicit rw_bit in the framing; a part with rw_bit: None never leaves is_read == None, so every data byte is a read.
        let write = matches!(self.is_read, Some(false));
        if write {
            self.write_acc.push(mosi);
            if let Some(reg) = self.find_register(addr) {
                if reg.access == RegisterAccess::Rw && self.write_acc.len() == reg.width as usize {
                    let written = unpack(&self.write_acc, reg.endian);
                    // `write_mask` (shared with the I²C engine) keeps the bits
                    // silicon owns; absent ⇒ the whole word is replaced.
                    let val = match reg.write_mask {
                        Some(mask) => {
                            let prev = self.reg_values.get(&reg.name).copied().unwrap_or(0);
                            (prev & !mask) | (written & mask)
                        }
                        None => written,
                    };
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
        validate_descriptor(&descriptor)?;
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
        labs: leak_labs(
            descriptor
                .metadata
                .as_ref()
                .map(|m| m.labs.as_slice())
                .unwrap_or(&[]),
        ),
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

// ─── Registry statics ──────────────────────────────────────────────────────
//
// A `DeclarativeSpiKit` is parsed from YAML at runtime, but the registry
// (`registry::KITS`) is a const slice of `&'static dyn PeripheralKit`. A
// `static LazyLock<DeclarativeSpiKit>` is the const-initialisable cell that
// bridges the two: the descriptor is parsed once on first access, and the
// `PeripheralKit` impl below forwards through it. Real parts get one static
// each here and one line in `registry::KITS`; the descriptor lives entirely in
// `configs/devices/*.yaml`.

use std::sync::LazyLock;

impl PeripheralKit for LazyLock<DeclarativeSpiKit> {
    fn metadata(&self) -> &'static KitMetadata {
        LazyLock::force(self).metadata()
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> Result<()> {
        LazyLock::force(self).attach(ctx)
    }
}

/// Analog Devices ADXL345 accelerometer (declarative `adxl345_spi.yaml`).
pub static ADXL345_KIT: LazyLock<DeclarativeSpiKit> = LazyLock::new(|| {
    DeclarativeSpiKit::from_yaml(
        labwired_config::embedded_device_yaml("adxl345_spi")
            .expect("adxl345_spi descriptor embedded"),
    )
    .expect("adxl345_spi.yaml is a valid declarative spi descriptor")
});

/// Maxim MAX31855 thermocouple converter (declarative `max31855.yaml`).
pub static MAX31855_KIT: LazyLock<DeclarativeSpiKit> = LazyLock::new(|| {
    DeclarativeSpiKit::from_yaml(
        labwired_config::embedded_device_yaml("max31855").expect("max31855 descriptor embedded"),
    )
    .expect("max31855.yaml is a valid declarative spi descriptor")
});

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

    #[test]
    fn declarative_spi_kit_builds_metadata_from_descriptor() {
        let kit = DeclarativeSpiKit::from_yaml(FIXTURE).unwrap();
        let m = kit.metadata();
        assert_eq!(m.device_type, "test_spi_fixture");
        assert!(matches!(
            m.transport,
            crate::peripherals::kit::Transport::Spi
        ));
        assert_eq!(m.inputs.len(), 1);
        assert!(m.inputs.iter().any(|c| c.key == "accel_x"));
        // No `metadata.labs` in FIXTURE ⇒ no labs advertised.
        assert_eq!(m.labs.len(), 0);
    }

    const LABS_FIXTURE: &str = r#"
type: test_spi_labs_fixture

behavior:
  primitive: spi_device
  spi:
    framing:
      command_bytes: 0
    registers:
      - name: TEMP
        addr: 0
        width: 4
        endian: be
        access: r
        source: temperature
        encode: { scale: 1.0 }

metadata:
  label: "Declarative SPI labs fixture"
  summary: "Test-only fixture asserting metadata.labs round-trips."
  category: spi
  inputs:
    - { key: temperature, label: "Temperature", unit: "°C", min: -50, max: 200, default: 100 }
  labs:
    - board_id: "test-board-lab"
      chip: "stm32f103"
      example_dir: "spi_test_fixture"
      demo_elf: "spi_test_fixture.elf"
"#;

    #[test]
    fn declarative_spi_kit_advertises_labs_from_descriptor_metadata() {
        let kit = DeclarativeSpiKit::from_yaml(LABS_FIXTURE).unwrap();
        let labs = kit.metadata().labs;
        assert_eq!(labs.len(), 1);
        assert_eq!(labs[0].board_id, "test-board-lab");
        assert_eq!(labs[0].chip, "stm32f103");
        assert_eq!(labs[0].example_dir, "spi_test_fixture");
        assert_eq!(labs[0].demo_elf, "spi_test_fixture.elf");
    }

    /// A MAX31855-style read-only part: `command_bytes: 0` means CS↓ clocks
    /// register 0 straight out with no leading command byte.
    const READ_ONLY_FIXTURE: &str = r#"
type: test_spi_readonly_fixture

behavior:
  primitive: spi_device
  spi:
    framing:
      command_bytes: 0
    registers:
      - name: TEMP
        addr: 0
        width: 4
        endian: be
        access: r
        source: temperature
        encode: { scale: 1.0 }

metadata:
  label: "Declarative SPI read-only fixture"
  summary: "Test-only MAX31855-shaped read-only SPI part (command_bytes: 0)."
  category: spi
  inputs:
    - { key: temperature, label: "Temperature", unit: "°C", min: -50, max: 200, default: 100 }
"#;

    fn read_only_dev() -> GenericSpiDevice {
        GenericSpiDevice::from_yaml(READ_ONLY_FIXTURE, "PA4").unwrap()
    }

    #[test]
    fn command_bytes_zero_reads_register_zero_straight_off_cs_select() {
        let mut d = read_only_dev();
        d.cs_select();
        // Default temperature 100.0 × scale 1.0 = 100 = 0x00000064, big-endian.
        let bytes: Vec<u8> = (0..4).map(|_| d.transfer(0x00)).collect();
        assert_eq!(bytes, vec![0x00, 0x00, 0x00, 0x64]);
        // A byte past the 4-byte register width returns 0xFF.
        assert_eq!(d.transfer(0x00), 0xFF);
        d.cs_release();
    }

    #[test]
    fn command_bytes_zero_reasserts_the_same_word_on_a_second_transaction() {
        let mut d = read_only_dev();
        d.cs_select();
        let first: Vec<u8> = (0..4).map(|_| d.transfer(0x00)).collect();
        d.cs_release();
        // Re-select for a second transaction: frame state must reset so the
        // same register-0 word is clocked out again from the start.
        d.cs_select();
        let second: Vec<u8> = (0..4).map(|_| d.transfer(0x00)).collect();
        d.cs_release();
        assert_eq!(first, vec![0x00, 0x00, 0x00, 0x64]);
        assert_eq!(second, vec![0x00, 0x00, 0x00, 0x64]);
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

    #[test]
    fn adxl345_kit_reads_devid_and_signed_axis() {
        let kit = DeclarativeSpiKit::from_yaml(
            labwired_config::embedded_device_yaml("adxl345_spi").unwrap(),
        )
        .unwrap();
        assert_eq!(kit.metadata().device_type, "adxl345_spi");
        // Build the device and read DEVID + a negative Z.
        let mut d = crate::peripherals::components::declarative_spi::GenericSpiDevice::from_yaml(
            labwired_config::embedded_device_yaml("adxl345_spi").unwrap(),
            "PA4",
        )
        .unwrap();
        d.cs_select();
        d.transfer(0x80); // read DEVID (0x00)
        assert_eq!(d.transfer(0x00), 0xE5);
        d.cs_release();
        d.set_input("accel_z", -1.0).unwrap();
        d.cs_select();
        d.transfer(0x80 | 0x36); // read DATAZ0
        let lo = d.transfer(0x00);
        let hi = d.transfer(0x00);
        d.cs_release();
        assert_eq!(u16::from_le_bytes([lo, hi]), 0xFF00); // -256 two's-complement
    }

    #[test]
    fn max31855_reads_composite_frame_no_command() {
        let mut d = crate::peripherals::components::declarative_spi::GenericSpiDevice::from_yaml(
            labwired_config::embedded_device_yaml("max31855").unwrap(),
            "PA4",
        )
        .unwrap();
        d.set_input("temperature", 100.0).unwrap(); // 400 = 0x190 @ [31:18]
        d.set_input("internal", 25.0).unwrap(); // 400 = 0x190 @ [15:4]
        d.cs_select(); // command_bytes:0 → data phase immediately
        let b: Vec<u8> = (0..4).map(|_| d.transfer(0x00)).collect();
        d.cs_release();
        assert_eq!(b, vec![0x06, 0x40, 0x19, 0x00]);
        // A negative thermocouple reading sets the sign bits.
        d.set_input("temperature", -25.0).unwrap();
        d.cs_select();
        let n: Vec<u8> = (0..4).map(|_| d.transfer(0x00)).collect();
        d.cs_release();
        let word = u32::from_be_bytes([n[0], n[1], n[2], n[3]]);
        assert_eq!((word >> 18) & 0x3FFF, 0x3F9C); // -100 in 14-bit two's-complement
    }

    /// Parity anchor: the declarative descriptor must reproduce the
    /// hand-written `Max31855` model's default power-on frame and stimulus
    /// response byte-for-byte. Default word = (100<<18)|(352<<4) = 0x01901600
    /// (tc=25.0°C, internal=22.0°C, fault=0 — see components/max31855.rs).
    #[test]
    fn max31855_parity() {
        let mut d = crate::peripherals::components::declarative_spi::GenericSpiDevice::from_yaml(
            labwired_config::embedded_device_yaml("max31855").unwrap(),
            "PA4",
        )
        .unwrap();

        // Default frame: tc=25.0 -> 100<<18, internal=22.0 -> 352<<4.
        d.cs_select();
        let default_frame: Vec<u8> = (0..4).map(|_| d.transfer(0x00)).collect();
        d.cs_release();
        assert_eq!(default_frame, vec![0x01, 0x90, 0x16, 0x00]);

        // After driving both stimuli: tc=100.0 -> 400<<18, internal=25.0 -> 400<<4.
        d.set_input("temperature", 100.0).unwrap();
        d.set_input("internal", 25.0).unwrap();
        d.cs_select();
        let driven_frame: Vec<u8> = (0..4).map(|_| d.transfer(0x00)).collect();
        d.cs_release();
        assert_eq!(driven_frame, vec![0x06, 0x40, 0x19, 0x00]);

        // Negative thermocouple reading: -100 in 14-bit two's-complement.
        d.set_input("temperature", -25.0).unwrap();
        d.cs_select();
        let neg: Vec<u8> = (0..4).map(|_| d.transfer(0x00)).collect();
        d.cs_release();
        let word = u32::from_be_bytes([neg[0], neg[1], neg[2], neg[3]]);
        assert_eq!((word >> 18) & 0x3FFF, 0x3F9C);
    }

    #[test]
    fn input_value_reads_defaults_and_tracks_set_input() {
        let mut d = crate::peripherals::components::declarative_spi::GenericSpiDevice::from_yaml(
            labwired_config::embedded_device_yaml("max31855").unwrap(),
            "PA4",
        )
        .unwrap();

        assert_eq!(d.input_value("temperature"), Some(25.0));
        assert_eq!(d.input_value("internal"), Some(22.0));
        assert!(d.input_value("nope").is_none());

        d.set_input("temperature", 100.0).unwrap();
        assert_eq!(d.input_value("temperature"), Some(100.0));
    }
}
