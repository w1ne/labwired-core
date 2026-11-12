// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Native BMI270 four-wire SPI transport and nominal sampled register interface.
//!
//! Source: Bosch BST-BMI270-DS000-08, sections 4.4, 4.6.6, 4.6.13, 4.9,
//! 5.2.65-67 and 6.3-6.4. The exact two supported initialization images are
//! identified in tests/fixtures/bmi270/README.md. Initialization validates their
//! bytes; the feature engine is not executed. FIFO, auxiliary bus, self-test,
//! feature outputs, filter/noise behavior and open-drain interrupt electrical
//! behavior are unsupported. Sampling uses the documented nominal ODR without
//! analog startup/filter settling. Existing I2C behavior is unchanged.

use std::{any::Any, borrow::Cow, collections::BTreeSet, sync::LazyLock};

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

use super::declarative_i2c::{GenericI2cDevice, BMI270_KIT};
use super::supply::{powered_from_config, POWERED_CONFIG_KEY};
use crate::peripherals::device::{I2cDevice, SpiDevice, SpiSampling};
use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};
use crate::sim_input::{InputChannel, SimInput, SimInputError};

const RAM_SIZE: usize = 8192;
const BASE_SHA: &str = "2d75e68e343a13ff99be98261dfbd99d9e8c6f267da9e3c5aeb883276ad178db";
const FIFO_SHA: &str = "e17bf316de8139afa8f9b27cd3520845c7809fb418cdd3f8439dcdcdea683ffe";
const POR_NS: u64 = 450_000;
const SPI_SELECT_NS: u64 = 200_000;
const INIT_NS: u64 = 20_000_000;
const DRDY_PULSE_NS: u64 = 156_250; // 1 / 6400 Hz

#[derive(Clone, Copy)]
enum Phase {
    Command,
    Dummy,
    Read,
    Write,
    Ignore,
}

pub struct Bmi270Spi {
    registers: GenericI2cDevice,
    channels: Vec<InputChannel>,
    cs_pin: String,
    interrupt_pins: [Option<String>; 2],
    sampling: SpiSampling,
    powered: bool,
    selected: bool,
    phase: Phase,
    address: u8,
    now_ns: u64,
    reset_ns: u64,
    spi_ready_ns: Option<u64>,
    aps_disabled_ns: Option<u64>,
    init_address: u16,
    stream_offset: usize,
    ram: [u8; RAM_SIZE],
    written: [bool; RAM_SIZE],
    upload_overflow: bool,
    init_ctrl: u8,
    init_status: u8,
    init_attempted: bool,
    init_due_ns: Option<u64>,
    config_image: Option<&'static str>,
    io_ctrl: [u8; 2],
    int_latch: u8,
    int_map: u8,
    status: u8,
    interrupt_status: u8,
    pulse_due_ns: Option<u64>,
    next_sample_ns: [Option<u64>; 2],
    samples: [u64; 2],
    sample_bytes: [u8; 12],
    pin_levels: [Option<bool>; 2],
    pin_drives: Vec<(String, bool)>,
    unsupported: BTreeSet<&'static str>,
}

impl Bmi270Spi {
    pub fn new(cs_pin: impl Into<String>) -> Result<Self> {
        let registers = GenericI2cDevice::from_yaml(
            labwired_config::embedded_device_yaml("bmi270").context("missing BMI270 descriptor")?,
            0x68,
        )?;
        let channels = registers
            .input_channels()
            .iter()
            .filter(|c| c.key != "steps")
            .cloned()
            .collect();
        Ok(Self {
            registers,
            channels,
            cs_pin: cs_pin.into(),
            interrupt_pins: [None, None],
            sampling: SpiSampling::Byte,
            powered: true,
            selected: false,
            phase: Phase::Command,
            address: 0,
            now_ns: 0,
            reset_ns: 0,
            spi_ready_ns: None,
            aps_disabled_ns: None,
            init_address: 0,
            stream_offset: 0,
            ram: [0; RAM_SIZE],
            written: [false; RAM_SIZE],
            upload_overflow: false,
            init_ctrl: 0,
            init_status: 0,
            init_attempted: false,
            init_due_ns: None,
            config_image: None,
            io_ctrl: [0; 2],
            int_latch: 0,
            int_map: 0,
            status: 0,
            interrupt_status: 0,
            pulse_due_ns: None,
            next_sample_ns: [None; 2],
            samples: [0; 2],
            sample_bytes: [0; 12],
            pin_levels: [None; 2],
            pin_drives: Vec::new(),
            unsupported: BTreeSet::new(),
        })
    }

    pub fn set_interrupt_pins(&mut self, int1: Option<String>, int2: Option<String>) {
        self.interrupt_pins = [int1, int2];
    }

    pub fn set_powered(&mut self, powered: bool) {
        if self.powered != powered {
            self.reset();
        }
        self.powered = powered;
    }

    pub fn set_spi_mode(&mut self, mode: Option<u8>) -> Result<()> {
        self.sampling = match mode {
            None => SpiSampling::Byte,
            Some(mode @ (0 | 3)) => SpiSampling::edge_mode(mode),
            _ => bail!("BMI270 supports SPI modes 0 and 3 only"),
        };
        Ok(())
    }

    fn register(&mut self, address: u8) -> u8 {
        self.registers.start();
        self.registers.write(address);
        let value = self.registers.read();
        self.registers.stop();
        value
    }

    fn store_register(&mut self, address: u8, value: u8) {
        self.registers.start();
        self.registers.write(address);
        self.registers.write(value);
        self.registers.stop();
    }

    fn reset(&mut self) {
        for index in 0..2 {
            self.park_pin(index);
        }
        self.store_register(0x7e, 0xb6);
        self.reset_ns = self.now_ns;
        self.spi_ready_ns = None;
        self.aps_disabled_ns = None;
        self.init_address = 0;
        self.stream_offset = 0;
        self.ram.fill(0);
        self.written.fill(false);
        self.upload_overflow = false;
        self.init_ctrl = 0;
        self.init_status = 0;
        self.init_attempted = false;
        self.init_due_ns = None;
        self.config_image = None;
        self.io_ctrl = [0; 2];
        self.int_latch = 0;
        self.int_map = 0;
        self.status = 0;
        self.interrupt_status = 0;
        self.pulse_due_ns = None;
        self.next_sample_ns = [None; 2];
        self.samples = [0; 2];
        self.sample_bytes.fill(0);
        self.unsupported.clear();
        self.phase = Phase::Ignore;
        self.update_pins();
    }

    fn ready(&self) -> bool {
        self.powered
            && self.now_ns >= self.reset_ns.saturating_add(POR_NS)
            && self.spi_ready_ns.is_some_and(|ready| self.now_ns >= ready)
    }

    fn image(&self) -> Option<&'static str> {
        if self.upload_overflow {
            return None;
        }
        for (length, expected, name) in [
            (RAM_SIZE, BASE_SHA, "base"),
            (328, FIFO_SHA, "maximum-fifo"),
        ] {
            if self.written[..length].iter().all(|b| *b)
                && self.written[length..].iter().all(|b| !*b)
                && format!("{:x}", Sha256::digest(&self.ram[..length])) == expected
            {
                return Some(name);
            }
        }
        None
    }

    fn period_ns(&mut self, sensor: usize) -> Option<u64> {
        if self.init_status != 1 || !self.powered {
            return None;
        }
        if sensor == 1 && self.register(0x43) & 7 > 4 {
            self.unsupported.insert("reserved gyroscope range");
            return None;
        }
        let enabled = self.register(0x7d) & if sensor == 0 { 4 } else { 2 } != 0;
        if !enabled {
            return None;
        }
        let config = self.register(if sensor == 0 { 0x40 } else { 0x42 });
        let odr = config & 15;
        match (sensor, odr) {
            (0, 1..=12) => Some(1_280_000_000 >> (odr - 1)),
            (1, 6..=13) => Some(40_000_000 >> (odr - 6)),
            _ => {
                self.unsupported.insert("reserved ODR configuration");
                None
            }
        }
    }

    fn restart_sampling(&mut self) {
        for sensor in 0..2 {
            self.next_sample_ns[sensor] = self
                .period_ns(sensor)
                .map(|p| self.now_ns.saturating_add(p));
        }
        let power = self.register(0x7d);
        if power & 4 == 0 {
            self.status &= !0x80;
        }
        if power & 2 == 0 {
            self.status &= !0x40;
        }
        if power & 6 == 0 {
            self.pulse_due_ns = None;
            self.interrupt_status = 0;
        }
        self.update_pins();
    }

    // The current pin-drive seam cannot release a pin to high impedance.
    // Park a previously driven line at its OLD inactive level, then stop
    // driving it. This avoids fabricating an interrupt when reset clears the
    // polarity bit. The limitation is explicit in the model's scope.
    fn park_pin(&mut self, index: usize) {
        let inactive = self.io_ctrl[index] & 2 == 0;
        if self.pin_levels[index].is_some_and(|level| level != inactive) {
            if let Some(pin) = self.interrupt_pins[index].clone() {
                self.pin_drives.push((pin, inactive));
            }
        }
        self.pin_levels[index] = None;
    }

    fn update_pins(&mut self) {
        let asserted = if self.int_latch & 1 != 0 {
            self.interrupt_status & 0xc0 != 0
        } else {
            self.pulse_due_ns.is_some_and(|end| end > self.now_ns)
        };
        for index in 0..2 {
            let ctrl = self.io_ctrl[index];
            if ctrl & 4 != 0 {
                self.unsupported.insert("open-drain INT output");
            }
            let Some(pin) = self.interrupt_pins[index].clone() else {
                continue;
            };
            let enabled = ctrl & 8 != 0 && ctrl & 4 == 0;
            if !enabled {
                continue;
            }
            let mapped = self.int_map & (4 << (index * 4)) != 0;
            let active_high = ctrl & 2 != 0;
            let level = if enabled && mapped && asserted {
                active_high
            } else {
                !active_high
            };
            if self.pin_levels[index] != Some(level) {
                self.pin_levels[index] = Some(level);
                self.pin_drives.push((pin, level));
            }
        }
    }

    fn read_register(&mut self) -> u8 {
        let address = self.address;
        let value = match address {
            0x03 => 0x10 | self.status,
            0x0c..=0x17 => {
                let value = self.sample_bytes[(address - 0x0c) as usize];
                self.status &= if address < 0x12 { !0x80 } else { !0x40 };
                value
            }
            0x18..=0x1a => {
                let ticks = ((self.now_ns - self.reset_ns) as u128 * 2 / 78_125) as u32;
                (ticks >> (8 * (address - 0x18))) as u8
            }
            0x1d => {
                let value = self.interrupt_status;
                self.interrupt_status = 0;
                self.update_pins();
                value
            }
            0x21 => self.init_status,
            0x24..=0x26 => {
                self.unsupported.insert("FIFO readout");
                0
            }
            0x30..=0x3f => {
                self.unsupported.insert("feature-engine output");
                0
            }
            0x53..=0x54 => self.io_ctrl[(address - 0x53) as usize],
            0x55 => self.int_latch,
            0x58 => self.int_map,
            0x59 => self.init_ctrl,
            0x5b => (self.init_address & 15) as u8,
            0x5c => (self.init_address >> 4) as u8,
            0x5e => {
                let value = self.ram.get(self.stream_offset).copied().unwrap_or(0);
                self.stream_offset = self.stream_offset.saturating_add(1);
                value
            }
            _ => self.register(address),
        };
        if address != 0x5e {
            self.address = self.address.wrapping_add(1) & 0x7f;
        }
        value
    }

    fn write_register(&mut self, value: u8) {
        let address = self.address;
        match address {
            0x53..=0x54 => {
                let index = (address - 0x53) as usize;
                if value & 8 == 0 || value & 4 != 0 {
                    self.park_pin(index);
                }
                self.io_ctrl[index] = value & 0x1e;
                self.update_pins();
            }
            0x55 => {
                self.int_latch = value & 1;
                self.update_pins();
            }
            0x58 => {
                self.int_map = value;
                self.update_pins();
            }
            0x59 => {
                self.init_ctrl = value & 1;
                if value & 1 != 0 {
                    let prepared = self
                        .aps_disabled_ns
                        .is_some_and(|at| self.now_ns >= at.saturating_add(POR_NS));
                    self.config_image = if prepared && !self.init_attempted {
                        self.image()
                    } else {
                        None
                    };
                    self.init_attempted = true;
                    if self.config_image.is_some() {
                        self.init_due_ns = Some(self.now_ns.saturating_add(INIT_NS));
                    } else {
                        self.init_status = 2;
                        self.init_due_ns = None;
                    }
                    self.restart_sampling();
                }
            }
            0x5b => self.init_address = (self.init_address & 0xff0) | u16::from(value & 15),
            0x5c => self.init_address = (self.init_address & 15) | (u16::from(value) << 4),
            0x5e => {
                if self.stream_offset >= RAM_SIZE {
                    self.upload_overflow = true;
                }
                if self.init_ctrl == 0
                    && !self.init_attempted
                    && self.stream_offset < RAM_SIZE
                    && self
                        .aps_disabled_ns
                        .is_some_and(|at| self.now_ns >= at.saturating_add(POR_NS))
                {
                    self.ram[self.stream_offset] = value;
                    self.written[self.stream_offset] = true;
                }
                self.stream_offset = self.stream_offset.saturating_add(1);
            }
            0x7e if value == 0xb6 => self.reset(),
            0x7c => {
                self.store_register(address, value);
                self.aps_disabled_ns = if value & 1 == 0 {
                    Some(self.now_ns)
                } else {
                    None
                };
            }
            0x40..=0x43 | 0x7d => {
                self.store_register(address, value);
                self.restart_sampling();
            }
            0x45..=0x49 => {
                self.unsupported.insert("FIFO configuration");
            }
            0x4b..=0x4f => {
                self.unsupported.insert("auxiliary interface");
            }
            0x56..=0x57 => {
                self.unsupported.insert("feature interrupts");
            }
            0x6b if value & 1 != 0 => {
                self.unsupported.insert("three-wire SPI");
                self.phase = Phase::Ignore;
            }
            0x6d..=0x6e => {
                self.unsupported.insert("sensor self-test");
            }
            _ => self.store_register(address, value),
        }
        if address != 0x5e {
            self.address = self.address.wrapping_add(1) & 0x7f;
        }
    }
}

impl SpiDevice for Bmi270Spi {
    fn sampling(&self) -> SpiSampling {
        self.sampling
    }
    fn component_id(&self) -> Option<&str> {
        SimInput::component_id(&self.registers)
    }
    fn cs_pin(&self) -> &str {
        &self.cs_pin
    }
    fn cs_select(&mut self) {
        self.selected = true;
        self.phase = if self.ready() {
            Phase::Command
        } else {
            Phase::Ignore
        };
    }
    fn cs_release(&mut self) {
        if self.selected && self.powered && self.spi_ready_ns.is_none() {
            self.spi_ready_ns = Some(self.now_ns.saturating_add(SPI_SELECT_NS));
        }
        self.selected = false;
        self.phase = Phase::Command;
    }
    fn transfer(&mut self, byte: u8) -> u8 {
        if !self.selected || !self.ready() {
            return 0xff;
        }
        match self.phase {
            Phase::Command => {
                self.address = byte & 0x7f;
                self.stream_offset = usize::from(self.init_address) * 2;
                self.phase = if byte & 0x80 != 0 {
                    Phase::Dummy
                } else {
                    Phase::Write
                };
                0
            }
            Phase::Dummy => {
                self.phase = Phase::Read;
                0
            }
            Phase::Read => self.read_register(),
            Phase::Write => {
                self.write_register(byte);
                0
            }
            Phase::Ignore => 0xff,
        }
    }
    fn advance_time_us(&mut self, us: u64) {
        let target = self.now_ns.saturating_add(us.saturating_mul(1000));
        if self.powered {
            loop {
                let next = [
                    self.init_due_ns,
                    self.next_sample_ns[0],
                    self.next_sample_ns[1],
                    self.pulse_due_ns,
                ]
                .into_iter()
                .flatten()
                .min();
                let Some(next) = next.filter(|next| *next <= target) else {
                    break;
                };
                self.now_ns = next;
                if self.init_due_ns == Some(next) {
                    self.init_due_ns = None;
                    self.init_status = 1;
                    self.restart_sampling();
                }
                if self.pulse_due_ns == Some(next) {
                    self.pulse_due_ns = None;
                    self.update_pins();
                }
                for sensor in 0..2 {
                    if self.next_sample_ns[sensor] != Some(next) {
                        continue;
                    }
                    let base = sensor * 6;
                    for index in 0..6 {
                        self.sample_bytes[base + index] =
                            self.register(0x0c + (base + index) as u8);
                    }
                    let bit = if sensor == 0 { 0x80 } else { 0x40 };
                    self.status |= bit;
                    self.interrupt_status |= bit;
                    self.samples[sensor] += 1;
                    self.pulse_due_ns = Some(next.saturating_add(DRDY_PULSE_NS));
                    self.next_sample_ns[sensor] =
                        self.period_ns(sensor).map(|p| next.saturating_add(p));
                }
                self.update_pins();
            }
        }
        self.now_ns = target;
        self.registers.advance_time_us(us);
        self.update_pins();
    }
    fn take_pin_drives(&mut self) -> Vec<(String, bool)> {
        std::mem::take(&mut self.pin_drives)
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
    fn artifacts(
        &self,
        id: &str,
        _opts: &crate::inspect::InspectOpts,
    ) -> Vec<crate::inspect::Artifact> {
        vec![crate::inspect::Artifact {
            kind: "spi_registers".into(),
            id: id.into(),
            bytes: None,
            meta: serde_json::json!({"powered": self.powered, "transport": "spi", "init_status": self.init_status,
                "config_image": self.config_image, "initialized_bytes": self.written.iter().filter(|b| **b).count(),
                "accel_samples": self.samples[0], "gyro_samples": self.samples[1],
                "unsupported_requests": self.unsupported,
                "scope": "Nominal sampled accel/gyro, temperature and push-pull DRDY. No FIFO, auxiliary bus, feature microcode execution, noise/filter response, open-drain or high-impedance INT output. Runtime snapshot is not implemented."}),
        }]
    }
}

impl SimInput for Bmi270Spi {
    fn input_channels(&self) -> &[InputChannel] {
        &self.channels
    }
    fn set_input(&mut self, key: &str, value: f64) -> std::result::Result<(), SimInputError> {
        self.require_channel(key, value)?;
        self.registers.set_input(key, value)
    }
    fn component_id(&self) -> Option<&str> {
        SimInput::component_id(&self.registers)
    }
    fn set_component_id(&mut self, id: String) {
        self.registers.set_component_id(id);
    }
}

pub struct Bmi270SpiKit {
    metadata: KitMetadata,
}
impl PeripheralKit for LazyLock<Bmi270SpiKit> {
    fn metadata(&self) -> &KitMetadata {
        &self.metadata
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> Result<()> {
        let cs = ctx
            .config_str("cs_pin")
            .filter(|s| !s.trim().is_empty())
            .context("BMI270 SPI requires an explicit cs_pin")?;
        let mut device = Bmi270Spi::new(cs)?;
        device.set_powered(powered_from_config(ctx));
        // The bus binds device output roles to the placed MCU pads. Passing
        // GPIO labels to take_pin_drives would leave every edge undelivered.
        let outputs = ["INT1".to_owned(), "INT2".to_owned()];
        let output_pins = std::collections::BTreeMap::from([
            ("INT1".to_owned(), "int1_pin".to_owned()),
            ("INT2".to_owned(), "int2_pin".to_owned()),
        ]);
        for key in output_pins.values() {
            if let Some(value) = ctx.ext.config.get(key) {
                anyhow::ensure!(
                    value.as_str().is_some_and(|s| !s.trim().is_empty()),
                    "BMI270 {key} must be a nonempty GPIO pad label"
                );
            }
        }
        ctx.bind_output_roles(&outputs, &output_pins)?;
        let bound: BTreeSet<_> = ctx.bound_output_roles().collect();
        device.set_interrupt_pins(
            bound.contains("INT1").then(|| "INT1".to_owned()),
            bound.contains("INT2").then(|| "INT2".to_owned()),
        );
        let mode = ctx
            .ext
            .config
            .get("spi_mode")
            .map(|value| {
                value
                    .as_u64()
                    .and_then(|n| u8::try_from(n).ok())
                    .context("invalid BMI270 spi_mode: expected an integer 0 or 3")
            })
            .transpose()?;
        device.set_spi_mode(mode)?;
        device.registers.seed_from_config(|key| ctx.config_f64(key));
        ctx.attach_spi_device(Box::new(device))
    }
}

pub static BMI270_SPI_KIT: LazyLock<Bmi270SpiKit> = LazyLock::new(|| {
    let mut metadata = BMI270_KIT.metadata().clone();
    metadata.device_type = Cow::Borrowed("bmi270_spi");
    metadata.label = Cow::Borrowed("BMI270 native SPI IMU");
    metadata.summary = Cow::Borrowed("Native four-wire BMI270 with real CS/dummy-byte framing, verified configuration upload and nominal motion sampling.");
    metadata.detail = Cow::Borrowed("Exact initialization images, register readback, nominal ODR and push-pull DRDY. FIFO, auxiliary bus, feature microcode execution, self-test, analog noise/filter response, open-drain/tri-state INT and runtime snapshots are not modeled.");
    metadata.transport = Transport::Spi;
    metadata.category = Category::Spi;
    metadata.inputs = Cow::Owned(
        metadata
            .inputs
            .iter()
            .filter(|c| c.key != "steps")
            .cloned()
            .collect(),
    );
    metadata.labs = Cow::Borrowed(&[]);
    metadata.config_keys = Cow::Owned(vec![
        ConfigKey { name: Cow::Borrowed("cs_pin"), ty: ConfigType::Str, doc: Cow::Borrowed("Required active-low chip-select GPIO.") },
        ConfigKey { name: Cow::Borrowed("int1_pin"), ty: ConfigType::Str, doc: Cow::Borrowed("Optional INT1 GPIO; nominal push-pull DRDY routing.") },
        ConfigKey { name: Cow::Borrowed("int2_pin"), ty: ConfigType::Str, doc: Cow::Borrowed("Optional INT2 GPIO; nominal push-pull DRDY routing.") },
        ConfigKey { name: Cow::Borrowed("spi_mode"), ty: ConfigType::Int, doc: Cow::Borrowed("Optional edge sampling: modes 0 and 3 only. Omit for byte-transaction controllers.") },
        POWERED_CONFIG_KEY,
    ]);
    Bmi270SpiKit { metadata }
});
