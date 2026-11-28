// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic **1-Wire slave** — one engine device that interprets a
//! [`labwired_config::OneWireSpec`], so a 1-Wire part is a YAML file.
//!
//! The device sees the host's level on the DQ line with its time, and says
//! when it pulls the line low itself. The line is the wired-AND of the two:
//! a released line reads high through the pull-up.
//!
//! The wire (AN126, standard speed). A host low pulse of at least
//! `reset_min_us` is a reset; after it ends the part waits `presence_wait_us`
//! and pulls low for `presence_us`. Every other host falling edge opens a
//! time slot. In a slot where the part is receiving, a low pulse shorter
//! than `write_one_max_us` writes a 1, a longer one a 0. In a slot where the
//! part is sending, a 0 is the part holding the line low for
//! `read_zero_hold_us` from the falling edge; a 1 leaves it released. Bytes
//! go least significant bit first.
//!
//! The ROM layer is engine: READ ROM, MATCH ROM, SKIP ROM and SEARCH ROM.
//! ALARM SEARCH is answered by staying silent (no alarm model). After the
//! ROM layer the next byte is a function command from the descriptor.

use std::collections::{BTreeMap, VecDeque};

use anyhow::{Context, Result};
use labwired_config::expr::{EvalCtx, Expr};
use labwired_config::{
    onewire_crc8, DeviceDescriptor, InputSpec, OneWireSet, OneWireSpec, ONEWIRE_REPLY_CRC8,
};

use crate::sim_input::{InputChannel, SimInput, SimInputError};

const NS_PER_US: u64 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Waiting for a reset; slots are ignored.
    Idle,
    /// Receiving the ROM command byte.
    RomCommand,
    /// Sending the ROM code (READ ROM).
    ReadRom,
    /// Receiving a ROM code to compare (MATCH ROM).
    MatchRom,
    /// SEARCH ROM: bit `index`, step 0 = send the bit, 1 = its complement,
    /// 2 = receive the host's direction.
    Search { index: u8, step: u8 },
    /// Receiving a function command byte.
    Function,
    /// Receiving the bytes of function `index`; `stored` of them so far.
    Receive { index: usize, stored: usize },
    /// Sending `tx`; read slots after it return the busy status.
    Reply,
    /// Read slots return the busy status: 0 while busy, 1 after.
    Status,
}

struct Compiled {
    actions: Vec<(usize, Expr)>,
    on_done: Vec<(usize, Expr)>,
    on_received: Vec<(usize, Expr)>,
    receive: Vec<usize>,
    /// `None` is the CRC-8 of the bytes before it.
    reply: Vec<Option<usize>>,
    busy_us: Option<Expr>,
}

/// Values the expressions read: the registers and the scaled inputs.
struct Ctx<'a> {
    names: &'a BTreeMap<String, usize>,
    registers: &'a [u8],
    inputs: &'a BTreeMap<String, f64>,
    scales: &'a BTreeMap<String, f64>,
}

impl EvalCtx for Ctx<'_> {
    fn reg(&self, name: &str) -> i64 {
        self.names
            .get(name)
            .map_or(0, |index| i64::from(self.registers[*index]))
    }
    fn field(&self, _: &str, _: &str) -> i64 {
        0
    }
    fn var(&self, _: &str) -> i64 {
        0
    }
    fn input(&self, key: &str) -> i64 {
        let value = self.inputs.get(key).copied().unwrap_or(0.0);
        let scale = self.scales.get(key).copied().unwrap_or(1.0);
        (value * scale).round() as i64
    }
    fn input_negative(&self, key: &str) -> bool {
        self.inputs.get(key).copied().unwrap_or(0.0) < 0.0
    }
    fn reported(&self, name: &str) -> i64 {
        self.reg(name)
    }
    fn fifo_len(&self, _: &str) -> i64 {
        0
    }
    fn pin(&self, _: &str) -> i64 {
        0
    }
    fn frame_byte(&self, _: usize) -> i64 {
        0
    }
    fn written(&self) -> i64 {
        0
    }
    fn state(&self) -> &str {
        ""
    }
}

/// One 1-Wire slave.
pub struct OneWireDevice {
    id: String,
    timing: labwired_config::OneWireTiming,
    rom: [u8; 8],
    names: BTreeMap<String, usize>,
    registers: Vec<u8>,
    opcodes: BTreeMap<u8, usize>,
    functions: Vec<Compiled>,
    function_names: Vec<String>,
    channels: Vec<InputChannel>,
    inputs: BTreeMap<String, f64>,
    scales: BTreeMap<String, f64>,

    phase: Phase,
    /// Host level as last reported.
    host_high: bool,
    host_fell_ns: u64,
    /// The slot the last falling edge opened was a read slot: its rising
    /// edge carries no bit from the host.
    read_slot: bool,
    bits: u64,
    bit_count: u32,
    tx: VecDeque<bool>,
    /// The part holds the line low over `[from, until)`.
    pull: Option<(u64, u64)>,
    busy_until_ns: u64,
    /// Function whose `on_done` runs when the busy time ends.
    pending_done: Option<usize>,
    /// Bytes the part has answered, and commands it ran, for the artifact.
    transactions: u64,
    last_function: Option<u8>,
}

fn register_index(names: &BTreeMap<String, usize>, name: &str) -> Result<usize> {
    names
        .get(name)
        .copied()
        .with_context(|| format!("register '{name}' is not declared"))
}

fn compile_sets(
    names: &BTreeMap<String, usize>,
    sets: &[OneWireSet],
) -> Result<Vec<(usize, Expr)>> {
    sets.iter()
        .map(|set| {
            let expr = Expr::parse(&set.value)
                .map_err(|error| anyhow::anyhow!("`{}`: {error}", set.value))?;
            Ok((register_index(names, &set.register)?, expr))
        })
        .collect()
}

/// The static contract of the `onewire_device` primitive.
pub(crate) fn validate_descriptor(descriptor: &DeviceDescriptor) -> Result<()> {
    anyhow::ensure!(
        descriptor.behavior.primitive == "onewire_device",
        "declarative 1-Wire device requires behavior.primitive: onewire_device, got '{}'",
        descriptor.behavior.primitive
    );
    let spec = descriptor
        .behavior
        .onewire
        .as_ref()
        .context("an onewire_device needs a behavior.onewire section")?;
    spec.validate()?;
    anyhow::ensure!(
        descriptor.behavior.pins.len() == 1,
        "an onewire_device has exactly one pin role (its DQ line), got {}",
        descriptor.behavior.pins.len()
    );
    Ok(())
}

impl OneWireDevice {
    /// Build the part. `config` is the placement's `config:`; it may set
    /// `serial_number` and the input start values (`config_key`).
    pub fn from_descriptor(
        id: impl Into<String>,
        descriptor: &DeviceDescriptor,
        config: impl Fn(&str) -> Option<f64>,
    ) -> Result<Self> {
        validate_descriptor(descriptor)?;
        let spec: &OneWireSpec = descriptor.behavior.onewire.as_ref().expect("validated");
        let serial = config("serial_number")
            .map(|value| value as u64)
            .unwrap_or(spec.serial_number);
        anyhow::ensure!(
            serial < (1 << 48),
            "serial_number 0x{serial:X} does not fit the 48-bit ROM serial"
        );
        let mut rom = [0u8; 8];
        rom[0] = spec.family_code;
        for (index, byte) in rom[1..7].iter_mut().enumerate() {
            *byte = (serial >> (8 * index)) as u8;
        }
        rom[7] = onewire_crc8(&rom[..7]);

        let names: BTreeMap<String, usize> = spec
            .registers
            .iter()
            .enumerate()
            .map(|(index, register)| (register.name.clone(), index))
            .collect();
        let registers = spec.registers.iter().map(|r| r.reset).collect();
        let mut opcodes = BTreeMap::new();
        let mut functions = Vec::new();
        let mut function_names = Vec::new();
        for (index, function) in spec.functions.iter().enumerate() {
            let path = function
                .name
                .clone()
                .unwrap_or_else(|| format!("0x{:02X}", function.opcode));
            let compiled = (|| -> Result<Compiled> {
                Ok(Compiled {
                    actions: compile_sets(&names, &function.actions)?,
                    on_done: compile_sets(&names, &function.on_done)?,
                    on_received: compile_sets(&names, &function.on_received)?,
                    receive: function
                        .receive
                        .iter()
                        .map(|name| register_index(&names, name))
                        .collect::<Result<_>>()?,
                    reply: function
                        .reply
                        .iter()
                        .map(|name| {
                            if name == ONEWIRE_REPLY_CRC8 {
                                Ok(None)
                            } else {
                                register_index(&names, name).map(Some)
                            }
                        })
                        .collect::<Result<_>>()?,
                    busy_us: function
                        .busy_us
                        .as_deref()
                        .map(Expr::parse)
                        .transpose()
                        .map_err(|error| anyhow::anyhow!("busy_us: {error}"))?,
                })
            })()
            .with_context(|| format!("onewire function {path}"))?;
            opcodes.insert(function.opcode, index);
            functions.push(compiled);
            function_names.push(path);
        }

        let specs: &[InputSpec] = descriptor
            .metadata
            .as_ref()
            .map(|m| m.inputs.as_slice())
            .unwrap_or(&[]);
        let channels: Vec<InputChannel> = specs
            .iter()
            .map(|input| InputChannel {
                key: input.key.clone().into(),
                label: input.label.clone().into(),
                unit: input.unit.clone().into(),
                min: input.min,
                max: input.max,
                default: input.default,
            })
            .collect();
        let scales = specs
            .iter()
            .map(|input| (input.key.clone(), input.expr_scale.unwrap_or(1.0)))
            .collect();
        let inputs = labwired_config::seeded_channel_values(specs, &config)
            .into_iter()
            .collect();
        Ok(Self {
            id: id.into(),
            timing: spec.timing,
            rom,
            names,
            registers,
            opcodes,
            functions,
            function_names,
            channels,
            inputs,
            scales,
            phase: Phase::Idle,
            host_high: true,
            host_fell_ns: 0,
            read_slot: false,
            bits: 0,
            bit_count: 0,
            tx: VecDeque::new(),
            pull: None,
            busy_until_ns: 0,
            pending_done: None,
            transactions: 0,
            last_function: None,
        })
    }

    pub fn rom(&self) -> [u8; 8] {
        self.rom
    }

    pub fn register(&self, name: &str) -> Option<u8> {
        self.names.get(name).map(|index| self.registers[*index])
    }

    fn ctx(&self) -> Ctx<'_> {
        Ctx {
            names: &self.names,
            registers: &self.registers,
            inputs: &self.inputs,
            scales: &self.scales,
        }
    }

    fn run(&mut self, sets: &[(usize, Expr)]) {
        for (index, expr) in sets {
            let value = expr.eval(&self.ctx());
            self.registers[*index] = value as u8;
        }
    }

    /// Finish a conversion whose time has come.
    pub fn service(&mut self, time_ns: u64) {
        if let Some(index) = self.pending_done {
            if time_ns >= self.busy_until_ns {
                self.pending_done = None;
                let sets = std::mem::take(&mut self.functions[index].on_done);
                self.run(&sets);
                self.functions[index].on_done = sets;
            }
        }
    }

    /// Does the part hold the line low at `time_ns`?
    pub fn pulls_low(&self, time_ns: u64) -> bool {
        self.is_connected()
            && self
                .pull
                .is_some_and(|(from, until)| (from..until).contains(&time_ns))
    }

    /// Parts without a `connected` channel are always on the bus. A present
    /// channel is on at 0.5 or above (default 1).
    fn is_connected(&self) -> bool {
        if !self
            .channels
            .iter()
            .any(|channel| channel.key == "connected")
        {
            return true;
        }
        self.inputs.get("connected").copied().unwrap_or(1.0) >= 0.5
    }

    /// Drop any presence or slot pull and return to idle so DQ is released.
    fn detach(&mut self) {
        self.pull = None;
        self.phase = Phase::Idle;
        self.tx.clear();
        self.read_slot = false;
        self.pending_done = None;
    }

    fn sending(&self) -> bool {
        matches!(
            self.phase,
            Phase::ReadRom | Phase::Reply | Phase::Status | Phase::Search { step: 0 | 1, .. }
        )
    }

    fn queue_byte(&mut self, byte: u8) {
        for bit in 0..8 {
            self.tx.push_back(byte >> bit & 1 == 1);
        }
    }

    /// The host drove the line to `high` at `time_ns`.
    pub fn host_level(&mut self, time_ns: u64, high: bool) {
        self.service(time_ns);
        if high == self.host_high {
            return;
        }
        self.host_high = high;
        if !high {
            self.host_fell_ns = time_ns;
            self.read_slot = self.sending();
            if self.read_slot {
                let bit = self.next_tx_bit(time_ns);
                if !bit {
                    self.pull =
                        Some((time_ns, time_ns + self.timing.read_zero_hold_us * NS_PER_US));
                }
            }
            return;
        }
        let low_ns = time_ns.saturating_sub(self.host_fell_ns);
        if low_ns >= self.timing.reset_min_us * NS_PER_US {
            self.reset(time_ns);
            return;
        }
        if !self.read_slot && self.phase != Phase::Idle {
            let bit = low_ns < self.timing.write_one_max_us * NS_PER_US;
            self.receive_bit(bit);
        }
    }

    fn reset(&mut self, time_ns: u64) {
        if !self.is_connected() {
            self.detach();
            return;
        }
        let from = time_ns + self.timing.presence_wait_us * NS_PER_US;
        self.pull = Some((from, from + self.timing.presence_us * NS_PER_US));
        self.phase = Phase::RomCommand;
        self.bits = 0;
        self.bit_count = 0;
        self.tx.clear();
    }

    /// The bit a read slot opened at `time_ns` returns.
    fn next_tx_bit(&mut self, time_ns: u64) -> bool {
        match self.phase {
            Phase::Search { index, step } => {
                let own = self.rom_bit(index);
                self.phase = Phase::Search {
                    index,
                    step: step + 1,
                };
                if step == 0 {
                    own
                } else {
                    !own
                }
            }
            Phase::Status => time_ns >= self.busy_until_ns,
            _ => {
                let bit = self.tx.pop_front();
                if self.tx.is_empty() {
                    // The ROM code is out: the next byte is a function
                    // command. A reply is out: read slots return the status.
                    self.phase = if self.phase == Phase::ReadRom {
                        Phase::Function
                    } else {
                        Phase::Status
                    };
                }
                bit.unwrap_or(time_ns >= self.busy_until_ns)
            }
        }
    }

    fn rom_bit(&self, index: u8) -> bool {
        self.rom[index as usize / 8] >> (index % 8) & 1 == 1
    }

    fn receive_bit(&mut self, bit: bool) {
        if let Phase::Search { index, .. } = self.phase {
            if bit != self.rom_bit(index) {
                self.phase = Phase::Idle;
            } else if index == 63 {
                self.phase = Phase::Function;
            } else {
                self.phase = Phase::Search {
                    index: index + 1,
                    step: 0,
                };
            }
            return;
        }
        self.bits |= u64::from(bit) << self.bit_count;
        self.bit_count += 1;
        let width = if self.phase == Phase::MatchRom { 64 } else { 8 };
        if self.bit_count < width {
            return;
        }
        let value = self.bits;
        self.bits = 0;
        self.bit_count = 0;
        match self.phase {
            Phase::RomCommand => self.rom_command(value as u8),
            Phase::MatchRom => {
                self.phase = if value.to_le_bytes() == self.rom {
                    Phase::Function
                } else {
                    Phase::Idle
                };
            }
            Phase::Function => self.function(value as u8),
            Phase::Receive { index, stored } => {
                let register = self.functions[index].receive[stored];
                self.registers[register] = value as u8;
                if stored + 1 == self.functions[index].receive.len() {
                    let sets = std::mem::take(&mut self.functions[index].on_received);
                    self.run(&sets);
                    self.functions[index].on_received = sets;
                    self.phase = Phase::Status;
                } else {
                    self.phase = Phase::Receive {
                        index,
                        stored: stored + 1,
                    };
                }
            }
            _ => {}
        }
    }

    fn rom_command(&mut self, command: u8) {
        self.phase = match command {
            0x33 => {
                for byte in self.rom {
                    self.queue_byte(byte);
                }
                Phase::ReadRom
            }
            0x55 => Phase::MatchRom,
            0xCC => Phase::Function,
            0xF0 => Phase::Search { index: 0, step: 0 },
            // ALARM SEARCH with no alarm, overdrive commands and unknown
            // bytes: the part stays silent until the next reset.
            _ => Phase::Idle,
        };
    }

    fn function(&mut self, opcode: u8) {
        let Some(&index) = self.opcodes.get(&opcode) else {
            self.phase = Phase::Idle;
            return;
        };
        self.transactions += 1;
        self.last_function = Some(opcode);
        let actions = std::mem::take(&mut self.functions[index].actions);
        self.run(&actions);
        self.functions[index].actions = actions;
        if let Some(busy) = &self.functions[index].busy_us {
            let us = busy.eval(&self.ctx()).max(0) as u64;
            // The function's busy time starts at its last command bit.
            self.busy_until_ns = self.host_fell_ns + us * NS_PER_US;
            if self.functions[index].on_done.is_empty() {
                self.pending_done = None;
            } else {
                self.pending_done = Some(index);
            }
        }
        if !self.functions[index].receive.is_empty() {
            self.phase = Phase::Receive { index, stored: 0 };
            return;
        }
        let mut sent = Vec::new();
        for item in self.functions[index].reply.clone() {
            let byte = match item {
                Some(register) => self.registers[register],
                None => onewire_crc8(&sent),
            };
            sent.push(byte);
        }
        for byte in &sent {
            self.queue_byte(*byte);
        }
        self.phase = if sent.is_empty() {
            Phase::Status
        } else {
            Phase::Reply
        };
    }

    /// What the part holds, for inspection and Flex device state.
    pub fn artifacts(&self, id: &str) -> Vec<crate::inspect::Artifact> {
        let registers: serde_json::Map<String, serde_json::Value> = self
            .names
            .iter()
            .map(|(name, index)| (name.clone(), serde_json::json!(self.registers[*index])))
            .collect();
        vec![crate::inspect::Artifact {
            kind: "onewire_device".to_string(),
            id: id.to_string(),
            meta: serde_json::json!({
                "rom": self.rom.iter().map(|b| format!("{b:02X}")).collect::<String>(),
                "registers": registers,
                "functions_run": self.transactions,
                "last_function": self.last_function.map(|op| {
                    self.opcodes
                        .get(&op)
                        .map(|index| self.function_names[*index].clone())
                        .unwrap_or_default()
                }),
            }),
            bytes: None,
        }]
    }

    pub fn id(&self) -> &str {
        &self.id
    }
}

impl SimInput for OneWireDevice {
    fn input_channels(&self) -> &[InputChannel] {
        &self.channels
    }

    fn set_input(&mut self, key: &str, value: f64) -> std::result::Result<(), SimInputError> {
        let channel = self
            .channels
            .iter()
            .find(|channel| channel.key == key)
            .ok_or_else(|| SimInputError::UnknownChannel(key.to_string()))?;
        if !value.is_finite() || value < channel.min || value > channel.max {
            return Err(SimInputError::OutOfRange {
                key: key.to_string(),
                value,
                min: channel.min,
                max: channel.max,
            });
        }
        self.inputs.insert(key.to_string(), value);
        if key == "connected" && !self.is_connected() {
            self.detach();
        }
        Ok(())
    }

    fn component_id(&self) -> Option<&str> {
        Some(&self.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ds18b20(temperature: f64) -> OneWireDevice {
        let descriptor = DeviceDescriptor::embedded("ds18b20").unwrap().unwrap();
        let mut device = OneWireDevice::from_descriptor("t1", &descriptor, |_| None).unwrap();
        device.set_input("temperature", temperature).unwrap();
        device
    }

    /// The host side of the bus at standard speed, as AN126 states it. Time
    /// in microseconds; returns the line level the host samples.
    struct Host<'a> {
        device: &'a mut OneWireDevice,
        now_us: u64,
    }

    impl Host<'_> {
        fn line(&self) -> bool {
            !self.device.pulls_low(self.now_us * NS_PER_US)
        }
        fn drive(&mut self, high: bool) {
            self.device.host_level(self.now_us * NS_PER_US, high);
        }
        fn wait(&mut self, us: u64) {
            self.now_us += us;
            self.device.service(self.now_us * NS_PER_US);
        }
        fn reset(&mut self) -> bool {
            self.drive(false);
            self.wait(480);
            self.drive(true);
            self.wait(70);
            let presence = !self.line();
            self.wait(410);
            presence
        }
        fn write_bit(&mut self, bit: bool) {
            self.drive(false);
            if bit {
                self.wait(6);
                self.drive(true);
                self.wait(64);
            } else {
                self.wait(60);
                self.drive(true);
                self.wait(10);
            }
        }
        fn read_bit(&mut self) -> bool {
            self.drive(false);
            self.wait(6);
            self.drive(true);
            self.wait(9);
            let bit = self.line();
            self.wait(55);
            bit
        }
        fn write(&mut self, byte: u8) {
            for bit in 0..8 {
                self.write_bit(byte >> bit & 1 == 1);
            }
        }
        fn read(&mut self) -> u8 {
            (0..8).fold(0, |byte, bit| byte | (u8::from(self.read_bit()) << bit))
        }
    }

    #[test]
    fn presence_pulse_answers_a_reset_and_nothing_else() {
        let mut device = ds18b20(25.0);
        let mut host = Host {
            device: &mut device,
            now_us: 0,
        };
        assert!(host.line());
        // A slot-length pulse is not a reset.
        host.drive(false);
        host.wait(60);
        host.drive(true);
        host.wait(70);
        assert!(host.line());
        assert!(host.reset());
    }

    #[test]
    fn disconnected_probe_gives_no_presence_and_releases_dq() {
        let mut device = ds18b20(-18.0);
        let mut host = Host {
            device: &mut device,
            now_us: 0,
        };
        assert!(host.reset(), "connected by default");
        host.device.set_input("connected", 0.0).unwrap();
        assert!(
            host.line(),
            "DQ is released as soon as the probe is removed"
        );
        assert!(!host.device.pulls_low(host.now_us * NS_PER_US));
        assert!(!host.reset(), "no presence pulse while disconnected");
        assert!(host.line());
        // A DallasTemperature master treats a missing presence as
        // DEVICE_DISCONNECTED_C and does not clock a scratchpad.
        host.write(0xCC);
        host.write(0xBE);
        assert_eq!(host.read(), 0xFF);
    }

    #[test]
    fn reconnecting_resumes_presence_and_temperature_reads() {
        let mut device = ds18b20(-18.0);
        let mut host = Host {
            device: &mut device,
            now_us: 0,
        };
        host.device.set_input("connected", 0.0).unwrap();
        assert!(!host.reset());
        host.device.set_input("connected", 1.0).unwrap();
        assert!(host.reset(), "presence returns after reconnect");
        host.write(0xCC);
        host.write(0x44);
        host.wait(750_000);
        assert!(host.reset());
        host.write(0xCC);
        host.write(0xBE);
        let lsb = host.read();
        let msb = host.read();
        // -18.0 °C = 0xFEE0 at 12-bit (16 × -18).
        assert_eq!(u16::from_le_bytes([lsb, msb]), 0xFEE0);
    }

    #[test]
    fn power_on_scratchpad_is_the_datasheet_state_with_its_crc() {
        let mut device = ds18b20(25.0);
        let mut host = Host {
            device: &mut device,
            now_us: 0,
        };
        assert!(host.reset());
        host.write(0xCC);
        host.write(0xBE);
        let scratchpad: Vec<u8> = (0..9).map(|_| host.read()).collect();
        assert_eq!(
            scratchpad,
            [0x50, 0x05, 0x4B, 0x46, 0x7F, 0xFF, 0x0C, 0x10, 0x1C]
        );
    }

    #[test]
    fn conversion_takes_the_resolution_time_then_reports_the_temperature() {
        let mut device = ds18b20(-10.125);
        let mut host = Host {
            device: &mut device,
            now_us: 0,
        };
        assert!(host.reset());
        host.write(0xCC);
        host.write(0x44);
        // Busy at 12 bits for 750 ms: read slots return 0.
        assert!(!host.read_bit());
        host.wait(700_000);
        assert!(!host.read_bit());
        host.wait(60_000);
        assert!(host.read_bit());
        assert!(host.reset());
        host.write(0xCC);
        host.write(0xBE);
        let lsb = host.read();
        let msb = host.read();
        // -10.125 °C = 0xFF5E (datasheet Table 1).
        assert_eq!(u16::from_le_bytes([lsb, msb]), 0xFF5E);

        // 9-bit resolution: 93.75 ms and the low three bits undefined (0).
        assert!(host.reset());
        host.write(0xCC);
        host.write(0x4E);
        host.write(0x4B);
        host.write(0x46);
        host.write(0x1F);
        host.device.set_input("temperature", 25.0625).unwrap();
        assert!(host.reset());
        host.write(0xCC);
        host.write(0x44);
        host.wait(90_000);
        assert!(!host.read_bit());
        host.wait(5_000);
        assert!(host.read_bit());
        assert!(host.reset());
        host.write(0xCC);
        host.write(0xBE);
        let lsb = host.read();
        let msb = host.read();
        assert_eq!(u16::from_le_bytes([lsb, msb]), 0x0190);
        let _th = host.read();
        let _tl = host.read();
        assert_eq!(host.read(), 0x1F, "only R1/R0 of a written CONFIG are kept");
    }

    #[test]
    fn rom_layer_reads_matches_and_searches_the_rom_code() {
        let descriptor = DeviceDescriptor::embedded("ds18b20").unwrap().unwrap();
        let mut device = OneWireDevice::from_descriptor("t1", &descriptor, |key| {
            (key == "serial_number").then_some(0x0000_B81C_0102_u64 as f64)
        })
        .unwrap();
        let rom = device.rom();
        assert_eq!(rom[0], 0x28);
        assert_eq!(rom[7], onewire_crc8(&rom[..7]));
        let mut host = Host {
            device: &mut device,
            now_us: 0,
        };
        assert!(host.reset());
        host.write(0x33);
        let read: Vec<u8> = (0..8).map(|_| host.read()).collect();
        assert_eq!(read, rom);

        // A MATCH ROM for another part leaves this one silent.
        assert!(host.reset());
        host.write(0x55);
        let mut other = rom;
        other[1] ^= 1;
        for byte in other {
            host.write(byte);
        }
        host.write(0xBE);
        assert_eq!(host.read(), 0xFF);
        // Its own ROM selects it.
        assert!(host.reset());
        host.write(0x55);
        for byte in rom {
            host.write(byte);
        }
        host.write(0xBE);
        assert_eq!(host.read(), 0x50);

        // SEARCH ROM: one part, so every bit answers (bit, !bit) and the host
        // follows it.
        assert!(host.reset());
        host.write(0xF0);
        let mut found = [0u8; 8];
        for index in 0..64 {
            let bit = host.read_bit();
            let complement = host.read_bit();
            assert_ne!(bit, complement, "bit {index}: a lone part never collides");
            if bit {
                found[index / 8] |= 1 << (index % 8);
            }
            host.write_bit(bit);
        }
        assert_eq!(found, rom);
        host.write(0xBE);
        assert_eq!(host.read(), 0x50, "a part found by search is selected");
    }
}
