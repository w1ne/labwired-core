// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The `onewire_device` primitive: a 1-Wire slave as data.
//!
//! What is engine: the bus signalling every 1-Wire slave shares (reset and
//! presence pulse, write and read time slots, LSB-first bytes) and the ROM
//! layer (READ ROM 0x33, MATCH ROM 0x55, SKIP ROM 0xCC, SEARCH ROM 0xF0),
//! from Maxim's "1-Wire Communication Through Software" (AN126) and the
//! DS18B20 datasheet's "1-Wire bus system" section. What is data: the family
//! code and serial number of the ROM, the part's memory as named byte
//! registers, and its function commands: what each one does to the registers,
//! which bytes the host writes after it, which bytes the part answers with,
//! and how long the part stays busy.

use std::collections::BTreeSet;

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};

/// `behavior.onewire`.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct OneWireSpec {
    /// ROM byte 0.
    pub family_code: u8,
    /// ROM bytes 1..=6, the 48-bit serial number. A `serial_number` config
    /// key on the placement overrides it, so two parts on a bench differ.
    #[serde(default)]
    pub serial_number: u64,
    #[serde(default)]
    pub timing: OneWireTiming,
    /// The part's memory, one byte per register, at its power-on value.
    #[serde(default)]
    pub registers: Vec<OneWireRegister>,
    pub functions: Vec<OneWireFunction>,
}

/// Standard-speed slot timing, in microseconds. The defaults are the
/// datasheet values; a part that differs states its own.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(deny_unknown_fields, default)]
pub struct OneWireTiming {
    /// A host low pulse at least this long is a reset (tRSTL, 480 µs).
    pub reset_min_us: u64,
    /// The part waits this long after the reset pulse ends (tPDHIGH, 15-60).
    pub presence_wait_us: u64,
    /// Then holds the line low this long: the presence pulse (tPDLOW, 60-240).
    pub presence_us: u64,
    /// A host low pulse shorter than this writes a 1; longer writes a 0. The
    /// part samples the line 15 µs into the slot (tSLOT sample window).
    pub write_one_max_us: u64,
    /// In a read slot that returns 0 the part holds the line low this long
    /// from the falling edge that opened the slot (tRDV, 15 µs, plus margin).
    pub read_zero_hold_us: u64,
}

impl Default for OneWireTiming {
    fn default() -> Self {
        Self {
            reset_min_us: 480,
            presence_wait_us: 30,
            presence_us: 120,
            write_one_max_us: 15,
            read_zero_hold_us: 30,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct OneWireRegister {
    pub name: String,
    #[serde(default)]
    pub reset: u8,
}

/// One function command, sent after the ROM layer selected the part.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct OneWireFunction {
    pub opcode: u8,
    /// Datasheet mnemonic, for messages.
    #[serde(default)]
    pub name: Option<String>,
    /// Register updates when the command byte arrives, in order.
    #[serde(default, rename = "do")]
    pub actions: Vec<OneWireSet>,
    /// Registers the host's next bytes are stored into, in order.
    #[serde(default)]
    pub receive: Vec<String>,
    /// What the part answers, byte by byte: a register name, or `crc8` for
    /// the Maxim CRC-8 (x^8 + x^5 + x^4 + 1) of the answer bytes before it.
    #[serde(default)]
    pub reply: Vec<String>,
    /// Busy time the command starts, an expression in microseconds. Read
    /// slots return 0 while busy and 1 after. A command with no `reply`
    /// answers read slots with this status either way.
    #[serde(default)]
    pub busy_us: Option<String>,
    /// Register updates when the busy time ends, in order: the result of a
    /// conversion appears when the conversion is done.
    #[serde(default)]
    pub on_done: Vec<OneWireSet>,
    /// Register updates after the last `receive` byte is stored, in order: a
    /// part that keeps only some bits of a written byte says so here.
    #[serde(default)]
    pub on_received: Vec<OneWireSet>,
}

/// `{ register: NAME, value: EXPR }`. The expression reads `reg(NAME)` and
/// `input(KEY)` (scaled by the channel's `expr_scale`); the stored value is
/// its low byte.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct OneWireSet {
    pub register: String,
    pub value: String,
}

/// The `reply` entry that stands for the CRC-8 of the bytes before it.
pub const ONEWIRE_REPLY_CRC8: &str = "crc8";

/// ROM commands the engine owns. A function opcode cannot reuse one: the
/// part could not tell which layer a byte belongs to.
pub const ONEWIRE_ROM_COMMANDS: &[u8] = &[0x33, 0x55, 0xCC, 0xF0, 0xEC];

impl OneWireSpec {
    /// The static contract: every name a command uses is a declared register,
    /// every expression parses, opcodes are unique and not ROM commands.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.serial_number < (1 << 48),
            "onewire.serial_number 0x{:X} does not fit the 48-bit ROM serial",
            self.serial_number
        );
        let t = &self.timing;
        ensure!(
            t.write_one_max_us > 0
                && t.read_zero_hold_us > 0
                && t.presence_us > 0
                && t.reset_min_us > t.presence_wait_us + t.presence_us
                && t.reset_min_us > t.read_zero_hold_us,
            "onewire.timing must keep every interval positive and a reset longer than a slot"
        );
        let mut names = BTreeSet::new();
        for register in &self.registers {
            ensure!(
                !register.name.is_empty() && register.name != ONEWIRE_REPLY_CRC8,
                "onewire register name '{}' is empty or reserved",
                register.name
            );
            ensure!(
                names.insert(register.name.as_str()),
                "onewire register '{}' is declared twice",
                register.name
            );
        }
        let known = |name: &str, path: &str| -> Result<()> {
            ensure!(
                names.contains(name),
                "{path} names register '{name}', which onewire.registers does not declare"
            );
            Ok(())
        };
        let mut opcodes = BTreeSet::new();
        ensure!(!self.functions.is_empty(), "onewire.functions is empty");
        for function in &self.functions {
            let path = format!(
                "onewire function 0x{:02X}{}",
                function.opcode,
                function
                    .name
                    .as_deref()
                    .map(|n| format!(" ({n})"))
                    .unwrap_or_default()
            );
            ensure!(
                !ONEWIRE_ROM_COMMANDS.contains(&function.opcode),
                "{path} reuses a ROM command opcode"
            );
            ensure!(opcodes.insert(function.opcode), "{path} is declared twice");
            ensure!(
                function.receive.is_empty() || function.reply.is_empty(),
                "{path} both receives and replies; a function moves bytes one way"
            );
            ensure!(
                function.on_received.is_empty() || !function.receive.is_empty(),
                "{path} has on_received but receives nothing"
            );
            for set in function
                .actions
                .iter()
                .chain(&function.on_done)
                .chain(&function.on_received)
            {
                known(&set.register, &path)?;
                crate::expr::Expr::parse(&set.value)
                    .map_err(|error| anyhow::anyhow!("{path} value `{}`: {error}", set.value))?;
            }
            ensure!(
                function.on_done.is_empty() || function.busy_us.is_some(),
                "{path} has on_done but no busy_us to end"
            );
            for name in &function.receive {
                known(name, &path)?;
            }
            for (index, name) in function.reply.iter().enumerate() {
                if name == ONEWIRE_REPLY_CRC8 {
                    ensure!(index > 0, "{path} reply starts with crc8 of nothing");
                } else {
                    known(name, &path)?;
                }
            }
            if let Some(busy) = &function.busy_us {
                crate::expr::Expr::parse(busy)
                    .map_err(|error| anyhow::anyhow!("{path} busy_us `{busy}`: {error}"))
                    .context("busy_us is an expression in microseconds")?;
            }
        }
        if self.family_code == 0 {
            bail!("onewire.family_code 0 is not a family");
        }
        Ok(())
    }
}

/// Maxim/Dallas CRC-8 (polynomial x^8 + x^5 + x^4 + 1, LSB first), the
/// checksum of a ROM code and of a DS18B20 scratchpad.
pub fn onewire_crc8(bytes: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &byte in bytes {
        let mut b = byte;
        for _ in 0..8 {
            let mix = (crc ^ b) & 1;
            crc >>= 1;
            if mix != 0 {
                crc ^= 0x8C;
            }
            b >>= 1;
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The worked example of Maxim application note 27: ROM 02 1C B8 01 00 00 00
    /// has CRC A2.
    #[test]
    fn crc8_matches_the_application_note_example() {
        assert_eq!(
            onewire_crc8(&[0x02, 0x1C, 0xB8, 0x01, 0x00, 0x00, 0x00]),
            0xA2
        );
    }
}
