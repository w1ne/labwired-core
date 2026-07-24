// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic **declarative I²C device** — one engine device driven entirely by a
//! datasheet-shaped [`labwired_config::I2cSpec`], so a new I²C sensor that fits
//! the two covered wire-protocol shapes is a YAML file with zero Rust.
//!
//! The two shapes mirror the two hand-written reference families already in the
//! tree, and this device is byte-compatible with each:
//!   * **register-pointer** (`registers:`) — the master writes a 1-byte pointer
//!     then streams a fixed-width LE/BE word; rw registers accumulate + echo the
//!     master's writes. This is the VEML7700 protocol
//!     ([`super::veml7700`]).
//!   * **command** (`commands:`) — the master writes a 16-bit big-endian
//!     command, then reads N words each followed by a CRC-8 byte. This is the
//!     Sensirion protocol ([`super::scd41`] / [`super::sensirion`]).
//!
//! A descriptor is exactly one shape (registers XOR commands). Measurements are
//! externally driven through the ONE stimulus contract,
//! [`crate::sim_input::SimInput`]: `metadata.inputs` defines the channels, and
//! register/response `source:` keys read the current slot value and apply the
//! declared linear `encode` (+ optional register-bit-field `scale_from`). No
//! expression language, no per-device code — every YAML field is meaningful to
//! someone reading only the part datasheet.
//!
//! **Delay gating.** A command's `delay_us` gates its response on simulated
//! wall-clock, advanced through the [`crate::peripherals::i2c::I2cDevice::advance_time_us`]
//! hook — the same hook the trait documents ("a bus master that knows the
//! elapsed wall-clock calls this on a slave immediately before servicing it").
//! Of the shipping controllers only the nRF54L TWIM currently drives that hook,
//! so command devices with `delay_us` are faithful on that bus; the reference
//! Sensirion models (scd41) chose always-ready responses for exactly this
//! reason. Reads before the delay elapses return not-ready bytes (`0xFF`),
//! matching how a Sensirion read past an empty response buffer reads.

use std::collections::HashMap;

use anyhow::{bail, Context, Result};
use labwired_config::{
    Crc8Spec, DeviceDescriptor, Endian, I2cAccess, I2cCommand, I2cRegister, I2cSpec, ResponseWord,
};

use super::declarative_regs::{encode_raw, leak_labs, pack, register_read_bytes, unpack};
use crate::peripherals::i2c::I2cDevice;
use crate::sim_input::{InputChannel, SimInput, SimInputError};

/// CRC-8 with an arbitrary polynomial + init, no final XOR. With
/// `poly = 0x31`, `init = 0xFF` this is byte-identical to
/// [`super::sensirion::crc8`] (asserted in tests).
fn crc8(data: &[u8], poly: u8, init: u8) -> u8 {
    let mut crc = init;
    for &byte in data {
        crc ^= byte;
        for _ in 0..8 {
            if (crc & 0x80) != 0 {
                crc = (crc << 1) ^ poly;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

/// The generic device. Constructed from a [`DeviceDescriptor`] whose
/// `behavior.i2c` supplies the wire protocol.
pub struct GenericI2cDevice {
    address: u8,
    /// Register-mode registers (empty in command mode).
    registers: Vec<I2cRegister>,
    /// Command-mode commands (empty in register mode).
    commands: Vec<I2cCommand>,
    /// CRC-8 framing for command responses.
    crc8: Option<Crc8Spec>,
    command_mode: bool,
    /// Command-code width in bytes (1 or 2). A command dispatches once the
    /// master has written this many bytes.
    code_width: usize,

    /// Measurement slots keyed by input-channel key (engineering units).
    slots: HashMap<String, f64>,
    /// Current stored value per register name (rw writes + resets). Also the
    /// source a `scale_from` reads its selecting bit-field from.
    reg_values: HashMap<String, u32>,

    /// Selected register pointer for the current transaction.
    pointer: Option<u8>,
    /// Bytes the master has written this transaction.
    write_buf: Vec<u8>,
    /// Bytes queued for the master to read; drained by `read`.
    read_buf: Vec<u8>,
    read_idx: usize,
    /// Register mode: whether `read_buf` has been latched for this read phase.
    latched: bool,

    /// Accumulated simulated wall-clock (µs) for `delay_us` gating.
    elapsed_us: u64,
    /// A delayed response withheld until `elapsed_us >= ready_at_us`.
    pending: Option<Vec<u8>>,
    ready_at_us: u64,

    /// Discovery channels (leaked to `'static`; see [`DeclarativeI2cKit`]).
    channels: &'static [InputChannel],
    /// system.yaml `external_devices` id, stamped at attach.
    component_id: Option<String>,
}

impl GenericI2cDevice {
    /// Build from a descriptor and pre-leaked channel table.
    pub fn from_descriptor(
        descriptor: &DeviceDescriptor,
        address: u8,
        channels: &'static [InputChannel],
    ) -> Result<Self> {
        let spec = descriptor
            .behavior
            .i2c
            .as_ref()
            .context("declarative i2c device is missing behavior.i2c")?;
        validate_spec(spec)?;

        let address = if address == 0 {
            spec.default_address
        } else {
            address
        };

        // Seed measurement slots from the declared input defaults.
        let mut slots = HashMap::new();
        if let Some(meta) = &descriptor.metadata {
            for input in &meta.inputs {
                slots.insert(input.key.clone(), input.default.unwrap_or(0.0));
            }
        }
        // Seed every register to its reset value so a scale_from / storage read
        // before any write observes the power-on state.
        let reg_values = spec
            .registers
            .iter()
            .map(|r| (r.name.clone(), r.reset))
            .collect();

        Ok(Self {
            address,
            registers: spec.registers.clone(),
            commands: spec.commands.clone(),
            crc8: spec.crc8,
            command_mode: !spec.commands.is_empty(),
            code_width: spec.code_width as usize,
            slots,
            reg_values,
            pointer: None,
            write_buf: Vec::with_capacity(8),
            read_buf: Vec::new(),
            read_idx: 0,
            latched: false,
            elapsed_us: 0,
            pending: None,
            ready_at_us: 0,
            channels,
            component_id: None,
        })
    }

    /// Convenience for tests / standalone use: parse a descriptor YAML and leak
    /// its channel table. (The kit path shares one leaked table across attaches;
    /// this leaks per call, which is fine for the few devices a test builds.)
    pub fn from_yaml(yaml: &str, address: u8) -> Result<Self> {
        let descriptor = DeviceDescriptor::from_yaml(yaml)?;
        let channels = leak_channels(&descriptor);
        Self::from_descriptor(&descriptor, address, channels)
    }

    fn find_register(&self, addr: u8) -> Option<&I2cRegister> {
        self.registers.iter().find(|r| r.addr == addr)
    }

    fn find_command(&self, code: u16) -> Option<&I2cCommand> {
        self.commands.iter().find(|c| c.code == code)
    }

    /// Build the response bytes for a dispatched command (before delay gating).
    fn build_response(&self, cmd: &I2cCommand) -> Vec<u8> {
        let mut out = Vec::new();
        for word in &cmd.response {
            let raw = self.response_word_raw(word);
            let bytes = pack(raw, word.width, Endian::Be); // commands are BE on wire
            match &self.crc8 {
                // CRC framing is per 16-bit word, exactly like the Sensirion
                // read buffer (see super::sensirion::encode_words).
                Some(c) => {
                    for chunk in bytes.chunks(2) {
                        out.extend_from_slice(chunk);
                        out.push(crc8(chunk, c.poly, c.init));
                    }
                }
                None => out.extend_from_slice(&bytes),
            }
        }
        out
    }

    fn response_word_raw(&self, word: &ResponseWord) -> u32 {
        if let Some(src) = &word.source {
            let value = self.slots.get(src).copied().unwrap_or(0.0);
            encode_raw(value, word.encode.as_ref(), 1.0, word.width, false)
        } else {
            word.const_value.unwrap_or(0)
        }
    }

    fn dispatch_command(&mut self, code: u16) {
        self.read_buf.clear();
        self.read_idx = 0;
        self.pending = None;
        let Some(cmd) = self.find_command(code) else {
            // Unknown command: no response queued (reads return 0xFF), matching
            // the Sensirion reference (scd41).
            return;
        };
        let cmd = cmd.clone();
        let resp = self.build_response(&cmd);
        match cmd.delay_us {
            Some(us) if us > 0 => {
                self.pending = Some(resp);
                self.ready_at_us = self.elapsed_us + us;
            }
            _ => self.read_buf = resp,
        }
    }
}

impl I2cDevice for GenericI2cDevice {
    fn address(&self) -> u8 {
        self.address
    }

    fn start(&mut self) {
        // (Re)START frames a new phase within the transaction: rewind the read
        // cursor and clear the register latch and the write accumulator. The
        // pointer (register mode) and any pending delayed response survive.
        self.write_buf.clear();
        self.read_idx = 0;
        self.latched = false;
    }

    fn stop(&mut self) {
        // End of transaction: clear the write accumulator so the next command /
        // pointer starts fresh (the C3 controller only calls start() on a
        // repeated START, so the real reset happens here — same as veml7700 /
        // scd41).
        self.write_buf.clear();
    }

    fn write(&mut self, data: u8) {
        self.write_buf.push(data);
        if self.command_mode {
            // A command completes once `code_width` bytes have arrived (a
            // 16-bit big-endian Sensirion opcode, or a single-byte BH1750-style
            // opcode). Parameter words follow but are accepted and ignored
            // (params_words); write_buf keeps growing so this never re-fires.
            if self.write_buf.len() == self.code_width {
                let code = self.write_buf[..self.code_width]
                    .iter()
                    .fold(0u16, |acc, &b| (acc << 8) | b as u16);
                self.dispatch_command(code);
            }
            return;
        }
        // Register mode: first byte is the pointer; the rest are a data write
        // into the pointed rw register.
        if self.write_buf.len() == 1 {
            self.pointer = Some(data);
            return;
        }
        let Some(ptr) = self.pointer else { return };
        if let Some(reg) = self.find_register(ptr) {
            if reg.access == I2cAccess::Rw && self.write_buf.len() == 1 + reg.width as usize {
                let val = unpack(&self.write_buf[1..], reg.endian);
                self.reg_values.insert(reg.name.clone(), val);
            }
        }
    }

    fn read(&mut self) -> u8 {
        if self.command_mode {
            if self.pending.is_some() && self.elapsed_us >= self.ready_at_us {
                self.read_buf = self.pending.take().unwrap();
                self.read_idx = 0;
            }
            let byte = self.read_buf.get(self.read_idx).copied().unwrap_or(0xFF);
            self.read_idx += 1;
            return byte;
        }
        // Register mode: latch the pointed register's bytes on the first read.
        if !self.latched {
            self.read_buf = match self.pointer.and_then(|p| self.find_register(p)) {
                Some(reg) => register_read_bytes(reg, &self.slots, &self.reg_values),
                // Unknown pointer reads a zero word, matching veml7700.
                None => vec![0, 0],
            };
            self.latched = true;
        }
        let byte = self.read_buf.get(self.read_idx).copied().unwrap_or(0xFF);
        self.read_idx += 1;
        byte
    }

    fn advance_time_us(&mut self, us: u64) {
        self.elapsed_us = self.elapsed_us.saturating_add(us);
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
    fn as_sim_input_mut(&mut self) -> Option<&mut dyn SimInput> {
        Some(self)
    }
}

impl SimInput for GenericI2cDevice {
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

/// A descriptor is exactly one shape, and command devices with CRC framing must
/// use even-width words (CRC is computed per 16-bit word).
fn validate_spec(spec: &I2cSpec) -> Result<()> {
    match (spec.registers.is_empty(), spec.commands.is_empty()) {
        (true, true) => bail!("behavior.i2c declares neither registers nor commands"),
        (false, false) => {
            bail!("behavior.i2c declares both registers and commands (a device is exactly one)")
        }
        _ => {}
    }
    if !spec.commands.is_empty() && !matches!(spec.code_width, 1 | 2) {
        bail!(
            "command device has code_width {} — only 1 (single-byte opcode) or 2 \
             (16-bit opcode) are supported",
            spec.code_width
        );
    }
    if spec.crc8.is_some() {
        for cmd in &spec.commands {
            for word in &cmd.response {
                if word.width % 2 != 0 {
                    bail!(
                        "command '{}' has an odd-width response word ({}); CRC-8 framing is \
                         computed per 16-bit word",
                        cmd.name,
                        word.width
                    );
                }
            }
        }
    }
    Ok(())
}

// ─── Discovery-channel leaking ─────────────────────────────────────────────

/// Leak the descriptor's `metadata.inputs` into a `'static` channel table
/// (`InputChannel` requires `'static` strings). One table per call — the kit
/// leaks once and shares it; tests leak per device.
pub(crate) fn leak_channels(descriptor: &DeviceDescriptor) -> &'static [InputChannel] {
    let inputs = descriptor
        .metadata
        .as_ref()
        .map(|m| m.inputs.as_slice())
        .unwrap_or(&[]);
    let channels: Vec<InputChannel> = inputs
        .iter()
        .map(|i| InputChannel {
            key: Box::leak(i.key.clone().into_boxed_str()),
            label: Box::leak(i.label.clone().into_boxed_str()),
            unit: Box::leak(i.unit.clone().into_boxed_str()),
            min: i.min,
            max: i.max,
        })
        .collect();
    Box::leak(channels.into_boxed_slice())
}

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

/// A [`PeripheralKit`] backed by a declarative `i2c_device` descriptor — one
/// instance per YAML device. `metadata()` must hand back a `&'static
/// KitMetadata`, so `from_yaml` builds it once and leaks it (the kit is itself
/// a long-lived registry entry, so the leak is bounded by the device count).
///
/// Phase 1 ships the machinery but registers no real parts: no instance is
/// added to [`crate::peripherals::kit::registry::KITS`], so the offline
/// peripherals manifest is unchanged.
pub struct DeclarativeI2cKit {
    descriptor: DeviceDescriptor,
    channels: &'static [InputChannel],
    metadata: &'static KitMetadata,
}

impl DeclarativeI2cKit {
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        let descriptor = DeviceDescriptor::from_yaml(yaml)?;
        if descriptor.behavior.primitive != "i2c_device" {
            bail!(
                "declarative i2c kit requires behavior.primitive: i2c_device, got '{}'",
                descriptor.behavior.primitive
            );
        }
        let spec = descriptor
            .behavior
            .i2c
            .as_ref()
            .context("declarative i2c kit is missing behavior.i2c")?;
        validate_spec(spec)?;
        let default_address = spec.default_address;

        let channels = leak_channels(&descriptor);
        let metadata = leak_metadata(&descriptor, channels, default_address);
        Ok(Self {
            descriptor,
            channels,
            metadata,
        })
    }
}

/// Derive a `&'static KitMetadata` from the descriptor's display metadata.
fn leak_metadata(
    descriptor: &DeviceDescriptor,
    channels: &'static [InputChannel],
    default_address: u8,
) -> &'static KitMetadata {
    let meta = descriptor.metadata.as_ref();
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    let label = meta
        .and_then(|m| m.label.clone())
        .unwrap_or_else(|| descriptor.r#type.clone());
    let summary = meta
        .and_then(|m| m.summary.clone())
        .unwrap_or_else(|| "Declarative I²C device.".to_string());

    let config_keys: &'static [ConfigKey] = Box::leak(
        vec![ConfigKey {
            name: "i2c_address",
            ty: ConfigType::Int,
            doc: leak(format!(
                "7-bit slave address. Defaults to 0x{default_address:02x}."
            )),
        }]
        .into_boxed_slice(),
    );

    Box::leak(Box::new(KitMetadata {
        device_type: leak(descriptor.r#type.clone()),
        label: leak(label),
        summary: leak(summary.clone()),
        detail: leak(summary),
        transport: Transport::I2c,
        category: Category::I2c,
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

impl PeripheralKit for DeclarativeI2cKit {
    fn metadata(&self) -> &'static KitMetadata {
        self.metadata
    }

    fn attach(&self, ctx: &mut AttachCtx<'_>) -> Result<()> {
        let spec = self
            .descriptor
            .behavior
            .i2c
            .as_ref()
            .context("declarative i2c kit is missing behavior.i2c")?;
        let address = ctx.i2c_address_or(spec.default_address)?;
        let device = GenericI2cDevice::from_descriptor(&self.descriptor, address, self.channels)?;
        ctx.attach_i2c_device(Box::new(device))
    }
}

// ─── Registry statics ──────────────────────────────────────────────────────
//
// A `DeclarativeI2cKit` is parsed from YAML at runtime, but the registry
// (`registry::KITS`) is a const slice of `&'static dyn PeripheralKit`. A
// `static LazyLock<DeclarativeI2cKit>` is the const-initialisable cell that
// bridges the two: the descriptor is parsed once on first access, and the
// `PeripheralKit` impl below forwards through it. Real parts get one static
// each here and one line in `registry::KITS`; the descriptor lives entirely in
// `configs/devices/*.yaml`.

use std::sync::LazyLock;

impl PeripheralKit for LazyLock<DeclarativeI2cKit> {
    fn metadata(&self) -> &'static KitMetadata {
        LazyLock::force(self).metadata()
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> Result<()> {
        LazyLock::force(self).attach(ctx)
    }
}

/// Sensirion SHT31 temperature + humidity sensor (declarative `sht31.yaml`).
pub static SHT31_KIT: LazyLock<DeclarativeI2cKit> = LazyLock::new(|| {
    DeclarativeI2cKit::from_yaml(
        labwired_config::embedded_device_yaml("sht31").expect("sht31 descriptor is embedded"),
    )
    .expect("sht31.yaml is a valid declarative i2c descriptor")
});

/// ROHM BH1750 ambient-light sensor (declarative `bh1750.yaml`).
pub static BH1750_KIT: LazyLock<DeclarativeI2cKit> = LazyLock::new(|| {
    DeclarativeI2cKit::from_yaml(
        labwired_config::embedded_device_yaml("bh1750").expect("bh1750 descriptor is embedded"),
    )
    .expect("bh1750.yaml is a valid declarative i2c descriptor")
});

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::components::sensirion::{crc8 as sensirion_crc8, encode_words};

    /// Register-mode fixture: a fictional light + temperature sensor exercising
    /// LE + BE words, an rw config register, source+encode, and scale_from.
    const REGISTER_FIXTURE: &str = include_str!("declarative_i2c_fixture.yaml");

    /// Command-mode fixture (inline): a Sensirion-shaped device. Kept inline
    /// because a descriptor YAML is exactly one device (register XOR command),
    /// and the on-disk fixture demonstrates the register schema.
    const COMMAND_FIXTURE: &str = r#"
type: test_i2c_command_fixture
behavior:
  primitive: i2c_device
  i2c:
    default_address: 0x62
    crc8: { poly: 0x31, init: 0xFF }
    commands:
      - name: start_periodic
        code: 0x21B1
      - name: get_data_ready
        code: 0xE4B8
        response:
          - { const: 0x8006 }
      - name: read_measurement
        code: 0xEC05
        response:
          - { source: co2, width: 2 }
          - { source: temperature, width: 2, encode: { scale: 372.771428, offset: 16776.75 } }
      - name: set_offset
        code: 0x241D
        params_words: 1
      - name: measure_single_shot
        code: 0x219D
        delay_us: 5000
        response:
          - { source: co2, width: 2 }
metadata:
  inputs:
    - { key: co2, label: "CO2", unit: ppm, min: 0, max: 40000, default: 450 }
    - { key: temperature, label: "Temperature", unit: "°C", min: -45, max: 130, default: 22 }
"#;

    /// Single-byte-opcode command fixture (inline): a BH1750-shaped device.
    /// `code_width: 1`, no CRC, one 16-bit BE response word, plus write-only
    /// power/reset opcodes that queue no response.
    const CODE_WIDTH_1_FIXTURE: &str = r#"
type: test_i2c_code_width_1_fixture
behavior:
  primitive: i2c_device
  i2c:
    default_address: 0x23
    code_width: 1
    commands:
      - name: power_on
        code: 0x01
      - name: reset
        code: 0x07
      - name: cont_hres
        code: 0x10
        response:
          - { source: lux, width: 2, encode: { scale: 1.2 } }
metadata:
  inputs:
    - { key: lux, label: "Illuminance", unit: lx, min: 0, max: 100000, default: 600 }
"#;

    fn reg_dev() -> GenericI2cDevice {
        GenericI2cDevice::from_yaml(REGISTER_FIXTURE, 0).unwrap()
    }
    fn cmd_dev() -> GenericI2cDevice {
        GenericI2cDevice::from_yaml(COMMAND_FIXTURE, 0).unwrap()
    }
    fn cw1_dev() -> GenericI2cDevice {
        GenericI2cDevice::from_yaml(CODE_WIDTH_1_FIXTURE, 0).unwrap()
    }

    /// Send a single-byte opcode.
    fn send_byte_cmd(d: &mut GenericI2cDevice, code: u8) {
        d.start();
        d.write(code);
    }

    /// Point at `reg` and read `width` bytes.
    fn read_reg(d: &mut GenericI2cDevice, reg: u8, width: usize) -> Vec<u8> {
        d.start();
        d.write(reg);
        d.start(); // repeated START into the read phase
        (0..width).map(|_| d.read()).collect()
    }

    fn send_cmd(d: &mut GenericI2cDevice, code: u16) {
        d.start();
        d.write((code >> 8) as u8);
        d.write((code & 0xFF) as u8);
    }

    fn read_bytes(d: &mut GenericI2cDevice, n: usize) -> Vec<u8> {
        d.start();
        (0..n).map(|_| d.read()).collect()
    }

    // ── addresses / mode ───────────────────────────────────────────────────

    #[test]
    fn register_fixture_defaults_to_declared_address() {
        assert_eq!(reg_dev().address(), 0x40);
    }

    #[test]
    fn command_fixture_defaults_to_declared_address() {
        assert_eq!(cmd_dev().address(), 0x62);
    }

    #[test]
    fn explicit_address_overrides_default() {
        let d = GenericI2cDevice::from_yaml(REGISTER_FIXTURE, 0x55).unwrap();
        assert_eq!(d.address(), 0x55);
    }

    // ── register mode: streaming reads LE and BE ───────────────────────────

    #[test]
    fn light_register_reads_little_endian() {
        // LIGHT (0x01) sources `lux` (default 450), gain 1× ⇒ 450 counts, LE.
        let mut d = reg_dev();
        let b = read_reg(&mut d, 0x01, 2);
        let word = (b[0] as u16) | ((b[1] as u16) << 8); // LE decode
        assert_eq!(word, 450, "LE low byte first: {b:02x?}");
        assert_eq!(b, vec![0xC2, 0x01]);
    }

    #[test]
    fn temp_register_reads_big_endian() {
        // TEMP (0x02) sources `temperature` (default 22) with scale 100 ⇒ 2200
        // centi-°C, big-endian.
        let mut d = reg_dev();
        let b = read_reg(&mut d, 0x02, 2);
        let word = ((b[0] as u16) << 8) | b[1] as u16; // BE decode
        assert_eq!(word, 2200, "BE high byte first: {b:02x?}");
        assert_eq!(b, vec![0x08, 0x98]);
    }

    // ── register mode: rw write accumulation + read-back ────────────────────

    #[test]
    fn rw_config_register_accumulates_and_reads_back() {
        let mut d = reg_dev();
        // Write CONFIG (0x00) = 0x0002 little-endian (low, high).
        d.start();
        d.write(0x00);
        d.write(0x02);
        d.write(0x00);
        d.stop();
        let b = read_reg(&mut d, 0x00, 2);
        let word = (b[0] as u16) | ((b[1] as u16) << 8);
        assert_eq!(word, 0x0002, "rw register round-trips its written value");
    }

    // ── register mode: scale_from bit-field scaling ─────────────────────────

    #[test]
    fn scale_from_field_selects_light_gain() {
        let mut d = reg_dev();
        // Default gain field 0 ⇒ ×1 ⇒ 450 counts.
        let base = {
            let b = read_reg(&mut d, 0x01, 2);
            (b[0] as u16) | ((b[1] as u16) << 8)
        };
        assert_eq!(base, 450);
        // Program CONFIG gain field = 2 (bits [1:0]) ⇒ ×4 ⇒ 1800 counts.
        d.start();
        d.write(0x00);
        d.write(0x02);
        d.write(0x00);
        d.stop();
        let scaled = {
            let b = read_reg(&mut d, 0x01, 2);
            (b[0] as u16) | ((b[1] as u16) << 8)
        };
        assert_eq!(scaled, 1800, "gain field 2 ⇒ ×4 scale");
    }

    // ── register mode: set_input round-trip ────────────────────────────────

    #[test]
    fn set_input_drives_the_light_register() {
        let mut d = reg_dev();
        d.set_input("lux", 1000.0).unwrap();
        let b = read_reg(&mut d, 0x01, 2);
        let word = (b[0] as u16) | ((b[1] as u16) << 8);
        assert_eq!(word, 1000);
    }

    #[test]
    fn out_of_range_and_unknown_channels_are_rejected() {
        let mut d = reg_dev();
        assert!(d.set_input("lux", -1.0).is_err());
        assert!(d.set_input("nope", 1.0).is_err());
    }

    #[test]
    fn unknown_register_reads_a_zero_word() {
        let mut d = reg_dev();
        let b = read_reg(&mut d, 0x7E, 2);
        assert_eq!(b, vec![0x00, 0x00]);
    }

    // ── command mode: dispatch + CRC-8 exactly matches sensirion ────────────

    #[test]
    fn read_measurement_crc_matches_sensirion_encode_words() {
        let mut d = cmd_dev();
        send_cmd(&mut d, 0xEC05);
        let bytes = read_bytes(&mut d, 6);
        // co2 = 450, temperature word = round(22*372.771428 + 16776.75) = 24978.
        let expected = encode_words(&[450, 24978]);
        assert_eq!(bytes, expected, "byte-exact with sensirion framing");
        for chunk in bytes.chunks(3) {
            assert_eq!(chunk[2], sensirion_crc8(&chunk[..2]));
        }
    }

    #[test]
    fn const_response_word_is_served() {
        let mut d = cmd_dev();
        send_cmd(&mut d, 0xE4B8); // get_data_ready
        let b = read_bytes(&mut d, 3);
        assert_eq!(b, vec![0x80, 0x06, sensirion_crc8(&[0x80, 0x06])]);
    }

    #[test]
    fn command_source_reflects_set_input() {
        let mut d = cmd_dev();
        d.set_input("co2", 1400.0).unwrap();
        send_cmd(&mut d, 0xEC05);
        let b = read_bytes(&mut d, 3);
        assert_eq!(((b[0] as u16) << 8) | b[1] as u16, 1400);
    }

    #[test]
    fn write_only_command_queues_no_response() {
        let mut d = cmd_dev();
        send_cmd(&mut d, 0x21B1); // start_periodic, no response
        let b = read_bytes(&mut d, 3);
        assert!(b.iter().all(|&x| x == 0xFF), "no response bytes: {b:02x?}");
    }

    #[test]
    fn unknown_command_queues_no_response() {
        let mut d = cmd_dev();
        send_cmd(&mut d, 0xDEAD);
        let b = read_bytes(&mut d, 3);
        assert!(b.iter().all(|&x| x == 0xFF));
    }

    // ── command mode: params_words accepted and ignored ────────────────────

    #[test]
    fn params_words_are_accepted_and_ignored() {
        let mut d = cmd_dev();
        // set_offset takes 1 parameter word: code then [hi, lo, crc].
        d.start();
        d.write(0x24);
        d.write(0x1D);
        d.write(0x01); // param hi
        d.write(0x2C); // param lo
        d.write(sensirion_crc8(&[0x01, 0x2C])); // param crc
        d.stop();
        // No response queued, and a later command still works.
        let ignored = read_bytes(&mut d, 3);
        assert!(ignored.iter().all(|&x| x == 0xFF));
        send_cmd(&mut d, 0xEC05);
        let b = read_bytes(&mut d, 3);
        assert_eq!(((b[0] as u16) << 8) | b[1] as u16, 450);
    }

    // ── command mode: single-byte opcode dispatch (code_width: 1) ──────────

    #[test]
    fn code_width_1_dispatches_on_first_byte() {
        // cont_hres (0x10) sources lux (default 600) with the datasheet
        // counts-per-lux factor 1.2 ⇒ round(600 * 1.2) = 720, big-endian, no CRC.
        let mut d = cw1_dev();
        assert_eq!(d.address(), 0x23);
        send_byte_cmd(&mut d, 0x10);
        let b = read_bytes(&mut d, 2);
        assert_eq!(
            ((b[0] as u16) << 8) | b[1] as u16,
            720,
            "BE raw = lux * 1.2"
        );
        assert_eq!(b, vec![0x02, 0xD0]);
    }

    #[test]
    fn code_width_1_source_reflects_set_input() {
        let mut d = cw1_dev();
        d.set_input("lux", 1200.0).unwrap();
        send_byte_cmd(&mut d, 0x10);
        let b = read_bytes(&mut d, 2);
        assert_eq!(((b[0] as u16) << 8) | b[1] as u16, 1440);
    }

    #[test]
    fn code_width_1_write_only_opcode_queues_no_response() {
        let mut d = cw1_dev();
        send_byte_cmd(&mut d, 0x01); // power_on, no response
        let b = read_bytes(&mut d, 2);
        assert!(b.iter().all(|&x| x == 0xFF), "no response bytes: {b:02x?}");
    }

    #[test]
    fn code_width_1_unknown_opcode_queues_no_response() {
        let mut d = cw1_dev();
        send_byte_cmd(&mut d, 0xAB);
        let b = read_bytes(&mut d, 2);
        assert!(b.iter().all(|&x| x == 0xFF));
    }

    #[test]
    fn code_width_defaults_to_two() {
        // The command fixture omits code_width ⇒ 16-bit opcode dispatch, so a
        // single written byte must NOT dispatch.
        let mut d = cmd_dev();
        d.start();
        d.write(0xE4); // first byte of get_data_ready (0xE4B8)
        let early = read_bytes(&mut d, 3);
        assert!(early.iter().all(|&x| x == 0xFF), "no dispatch on 1 byte");
    }

    #[test]
    fn invalid_code_width_is_rejected() {
        let yaml = r#"
type: bad_code_width
behavior:
  primitive: i2c_device
  i2c:
    default_address: 0x10
    code_width: 3
    commands:
      - { name: c, code: 0x01 }
"#;
        assert!(GenericI2cDevice::from_yaml(yaml, 0).is_err());
    }

    // ── command mode: delay_us data-ready gating ───────────────────────────

    #[test]
    fn delay_us_gates_response_until_time_elapses() {
        let mut d = cmd_dev();
        send_cmd(&mut d, 0x219D); // measure_single_shot, delay 5000 µs
                                  // Before the delay elapses: not ready ⇒ 0xFF.
        let early = read_bytes(&mut d, 3);
        assert!(
            early.iter().all(|&x| x == 0xFF),
            "not ready yet: {early:02x?}"
        );
        // Advance short of the deadline: still not ready.
        d.advance_time_us(4999);
        let still = read_bytes(&mut d, 3);
        assert!(still.iter().all(|&x| x == 0xFF));
        // Cross the deadline: the response materialises.
        d.advance_time_us(1);
        let ready = read_bytes(&mut d, 3);
        assert_eq!(((ready[0] as u16) << 8) | ready[1] as u16, 450);
        assert_eq!(ready[2], sensirion_crc8(&ready[..2]));
    }

    // ── the generic crc8 helper matches the sensirion one ──────────────────

    #[test]
    fn generic_crc8_matches_sensirion_with_default_params() {
        for data in [&[0xBE, 0xEF][..], &[0x01, 0xC2][..], &[0x80, 0x06][..]] {
            assert_eq!(crc8(data, 0x31, 0xFF), sensirion_crc8(data));
        }
    }

    // ── spec validation ────────────────────────────────────────────────────

    #[test]
    fn a_device_declaring_both_shapes_is_rejected() {
        let yaml = r#"
type: bad
behavior:
  primitive: i2c_device
  i2c:
    default_address: 0x10
    registers:
      - { name: A, addr: 0, width: 2, endian: le, access: r }
    commands:
      - { name: c, code: 0x0001 }
"#;
        assert!(GenericI2cDevice::from_yaml(yaml, 0).is_err());
    }

    #[test]
    fn declarative_kit_builds_metadata_from_descriptor() {
        let kit = DeclarativeI2cKit::from_yaml(REGISTER_FIXTURE).unwrap();
        let m = kit.metadata();
        assert_eq!(m.device_type, "test_i2c_fixture");
        assert_eq!(m.inputs.len(), 2);
        assert!(m.inputs.iter().any(|c| c.key == "lux"));
    }
}
