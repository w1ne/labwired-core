// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::pins::{External, InputChange, PadDriver, PinPort};
use crate::{Peripheral, PeripheralTickResult, SimResult};
use labwired_config::PeripheralDescriptor;
use sha2::{Digest, Sha256};
use std::any::Any;

use std::cell::RefCell;

#[derive(Debug)]
struct InflightEvent {
    #[allow(dead_code)]
    id: String,
    delay_remaining: u64,
    action: labwired_config::TimingAction,
    interrupt: Option<String>,
    periodic_interval: Option<u64>,
}

/// A bit held at a fixed level by a `stuck_at_bit` fault. Applied to every read
/// so the CPU always observes the stuck level regardless of writes.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct StuckBit {
    byte_offset: u64,
    bit_in_byte: u8,
    level: u8,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct GenericRuntimeSnapshot {
    version: u8,
    descriptor_key: [u8; 32],
    data: Vec<u8>,
    /// Static hook index and remaining delay; action/IRQ come from the checked
    /// descriptor. TimingAction's YAML decoder is not a bincode decoder.
    inflight: Vec<(u32, u64)>,
    stuck_bits: Vec<StuckBit>,
    ext_mask: u32,
    reset_values: Vec<u32>,
}

fn periodic_interval(hook: &labwired_config::TimingDescriptor) -> Option<u64> {
    match &hook.trigger {
        labwired_config::TimingTrigger::Periodic { period_cycles } => Some(*period_cycles),
        _ => None,
    }
}

/// A generic peripheral implementation that uses a `PeripheralDescriptor` to define its
/// register layout and access permissions.
///
/// This allows for rapid modeling of memory-mapped peripherals without writing custom Rust code.
/// Sentinel for "no register covers this byte" in [`GenericPeripheral::reg_at_byte`].
const NO_REG: u32 = u32::MAX;

#[derive(Debug)]
pub struct GenericPeripheral {
    descriptor: PeripheralDescriptor,
    /// Identity of the construction-time topology, before runtime fault
    /// injection can change a register's reset value.
    descriptor_key: [u8; 32],
    data: RefCell<Vec<u8>>,
    /// O(1) offset->register resolution, built once in [`GenericPeripheral::new`].
    ///
    /// Direct-mapped by byte offset: `reg_at_byte[b]` holds the index (into
    /// `descriptor.registers`) of the FIRST register, in declaration order, whose
    /// `[address_offset, address_offset + size/8)` span covers byte `b`, or
    /// [`NO_REG`]. "First match wins" mirrors the former linear scan exactly, so
    /// this is a drop-in replacement for the O(n) per-access scan.
    ///
    /// Byte (not word) granularity is required: some registers are 8/16-bit and
    /// some offsets are not word-aligned. Sized to `max_addr` — the same window
    /// the `data` backing store already allocates — so it never over-allocates
    /// relative to the peripheral's declared address space. Registers are static
    /// after construction, so the table is built once and never rebuilt.
    reg_at_byte: Vec<u32>,
    inflight_events: RefCell<Vec<InflightEvent>>,
    stuck_bits: RefCell<Vec<StuckBit>>,
    /// GPIO descriptors only: pads the outside world holds
    /// ([`PinPort::set_external`]); the level on each is its `IN` bit.
    ext_mask: u32,
}

impl GenericPeripheral {
    /// Creates a new `GenericPeripheral` from a descriptor.
    ///
    /// The backing memory is automatically sized to accommodate the highest register offset,
    /// and all registers are initialized to their specified `reset_value`.
    pub fn new(descriptor: PeripheralDescriptor) -> Self {
        let descriptor_key = Sha256::digest(
            bincode::serialize(&descriptor).expect("serialize declarative descriptor"),
        )
        .into();
        let mut max_addr = 0;
        for reg in &descriptor.registers {
            let end_addr = reg.address_offset + (reg.size as u64 / 8);
            if end_addr > max_addr {
                max_addr = end_addr;
            }
        }

        let mut data = vec![0; max_addr as usize];

        // Initialize with reset values
        for reg in &descriptor.registers {
            let val = reg.reset_value;
            let offset = reg.address_offset as usize;
            match reg.size {
                8 => data[offset] = val as u8,
                16 => {
                    data[offset] = (val & 0xFF) as u8;
                    data[offset + 1] = ((val >> 8) & 0xFF) as u8;
                }
                32 => {
                    data[offset] = (val & 0xFF) as u8;
                    data[offset + 1] = ((val >> 8) & 0xFF) as u8;
                    data[offset + 2] = ((val >> 16) & 0xFF) as u8;
                    data[offset + 3] = ((val >> 24) & 0xFF) as u8;
                }
                _ => {}
            }
        }

        // Build the O(1) offset->register lookup once. Earlier registers claim
        // their bytes first, so an overlap resolves to the same register the old
        // linear scan would have found (first match wins).
        let mut reg_at_byte = vec![NO_REG; max_addr as usize];
        for (idx, reg) in descriptor.registers.iter().enumerate() {
            let start = reg.address_offset as usize;
            let end = start + (reg.size as usize / 8);
            for slot in reg_at_byte[start..end].iter_mut() {
                if *slot == NO_REG {
                    *slot = idx as u32;
                }
            }
        }

        let p = Self {
            descriptor,
            descriptor_key,
            data: RefCell::new(data),
            reg_at_byte,
            inflight_events: RefCell::new(Vec::new()),
            stuck_bits: RefCell::new(Vec::new()),
            ext_mask: 0,
        };

        // Initialize periodic events
        if let Some(timing) = &p.descriptor.timing {
            for hook in timing {
                if let labwired_config::TimingTrigger::Periodic { period_cycles } = &hook.trigger {
                    p.inflight_events.borrow_mut().push(InflightEvent {
                        id: hook.id.clone(),
                        delay_remaining: *period_cycles,
                        action: hook.action.clone(),
                        interrupt: hook.interrupt.clone(),
                        periodic_interval: Some(*period_cycles),
                    });
                }
            }
        }
        p
    }

    pub fn get_descriptor(&self) -> &labwired_config::PeripheralDescriptor {
        &self.descriptor
    }

    /// Resolves `offset` to the index of the register covering that byte in O(1),
    /// or `None` if no register does. Replaces the former O(n) linear scan; see
    /// [`GenericPeripheral::reg_at_byte`] for the "first match wins" semantics.
    #[inline]
    fn reg_index_at(&self, offset: u64) -> Option<usize> {
        match self.reg_at_byte.get(offset as usize) {
            Some(&idx) if idx != NO_REG => Some(idx as usize),
            _ => None,
        }
    }

    pub fn peek_u32_raw(&self, offset: u64) -> Option<u32> {
        let data = self.data.borrow();
        let offset = offset as usize;
        let end = offset.checked_add(4)?;
        if end > data.len() {
            return None;
        }
        let val = data[offset] as u32
            | ((data[offset + 1] as u32) << 8)
            | ((data[offset + 2] as u32) << 16)
            | ((data[offset + 3] as u32) << 24);
        Some(self.apply_stuck_u32(offset as u64, val))
    }

    /// Write `value` straight into the backing storage at `offset`, bypassing
    /// the descriptor's access rules and any side effects — the counterpart of
    /// [`Self::peek_u32_raw`].
    ///
    /// This exists for *hardware-side* status writes: register bits the SoC
    /// itself latches rather than the CPU. The ESP32-C3 PMS is the first user —
    /// a permission violation latches its address/world/operation into
    /// `SENSITIVE_CORE_0_{I,D}RAM0_PMS_MONITOR_*`, which firmware then reads
    /// back through `esp_memprot_get_violate_*`. Unlike
    /// [`Self::force_register_value`] this does NOT touch the declared reset
    /// value, so a reset still clears the latch. Returns false if `offset` is
    /// outside the peripheral.
    pub fn poke_u32_raw(&self, offset: u64, value: u32) -> bool {
        let mut data = self.data.borrow_mut();
        let offset = offset as usize;
        let Some(end) = offset.checked_add(4) else {
            return false;
        };
        if end > data.len() {
            return false;
        }
        data[offset] = value as u8;
        data[offset + 1] = (value >> 8) as u8;
        data[offset + 2] = (value >> 16) as u8;
        data[offset + 3] = (value >> 24) as u8;
        true
    }

    /// Force `reg_id` to `value`, overriding both its live contents and its
    /// declared reset value so the change survives a later reset. This is the
    /// injection point for the `wrong_reset_value` fault. Returns false if no
    /// register has `reg_id`.
    pub fn force_register_value(&mut self, reg_id: &str, value: u32) -> bool {
        let Some(reg) = self
            .descriptor
            .registers
            .iter_mut()
            .find(|r| r.id == reg_id)
        else {
            return false;
        };
        reg.reset_value = value;
        let offset = reg.address_offset as usize;
        let size = reg.size;
        let mut data = self.data.borrow_mut();
        match size {
            8 if offset < data.len() => data[offset] = value as u8,
            16 if offset + 1 < data.len() => {
                data[offset] = (value & 0xFF) as u8;
                data[offset + 1] = ((value >> 8) & 0xFF) as u8;
            }
            32 if offset + 3 < data.len() => {
                data[offset] = (value & 0xFF) as u8;
                data[offset + 1] = ((value >> 8) & 0xFF) as u8;
                data[offset + 2] = ((value >> 16) & 0xFF) as u8;
                data[offset + 3] = ((value >> 24) & 0xFF) as u8;
            }
            _ => {}
        }
        true
    }

    /// Hold `bit` of `reg_id` at `level` (the `stuck_at_bit` fault). The bit is
    /// forced on every read, so writes never change what the CPU observes.
    /// Returns false if the register is absent or the bit is out of range.
    pub fn force_stuck_bit(&mut self, reg_id: &str, bit: u8, level: u8) -> bool {
        let Some(reg) = self.descriptor.registers.iter().find(|r| r.id == reg_id) else {
            return false;
        };
        if (bit as u32) >= reg.size as u32 {
            return false;
        }
        self.stuck_bits.borrow_mut().push(StuckBit {
            byte_offset: reg.address_offset + (bit / 8) as u64,
            bit_in_byte: bit % 8,
            level: level & 1,
        });
        true
    }

    fn gpio_reg_offset(&self, id: &str) -> Option<usize> {
        if !self.descriptor.peripheral.eq_ignore_ascii_case("GPIO") {
            return None;
        }
        self.descriptor
            .registers
            .iter()
            .find(|r| r.id == id && r.size == 32)
            .map(|r| r.address_offset as usize)
    }

    /// Whether the descriptor's `ENABLE` register has the output driver of
    /// `pin` on. `None` when the descriptor has no `ENABLE` register (or the
    /// pin is out of range), i.e. the direction is not derivable.
    fn gpio_pad_driven(&self, pin: u8) -> Option<bool> {
        if pin >= 32 {
            return None;
        }
        let offset = self.gpio_reg_offset("ENABLE")?;
        let value = self.read_register_storage_u32(offset)?;
        Some(value & (1u32 << pin) != 0)
    }

    fn read_register_storage_u32(&self, offset: usize) -> Option<u32> {
        let data = self.data.borrow();
        if offset + 3 >= data.len() {
            return None;
        }
        Some(
            (data[offset] as u32)
                | ((data[offset + 1] as u32) << 8)
                | ((data[offset + 2] as u32) << 16)
                | ((data[offset + 3] as u32) << 24),
        )
    }

    fn write_register_storage_u32(&self, offset: usize, value: u32) -> bool {
        let mut data = self.data.borrow_mut();
        if offset + 3 >= data.len() {
            return false;
        }
        data[offset] = (value & 0xFF) as u8;
        data[offset + 1] = ((value >> 8) & 0xFF) as u8;
        data[offset + 2] = ((value >> 16) & 0xFF) as u8;
        data[offset + 3] = ((value >> 24) & 0xFF) as u8;
        true
    }

    /// Apply any stuck bits that fall on the byte at `offset` to a read value.
    fn apply_stuck_byte(&self, offset: u64, mut byte: u8) -> u8 {
        for s in self.stuck_bits.borrow().iter() {
            if s.byte_offset == offset {
                if s.level == 1 {
                    byte |= 1 << s.bit_in_byte;
                } else {
                    byte &= !(1 << s.bit_in_byte);
                }
            }
        }
        byte
    }

    /// Apply any stuck bits within the 4 bytes at `offset` to a read value.
    fn apply_stuck_u32(&self, offset: u64, mut val: u32) -> u32 {
        for s in self.stuck_bits.borrow().iter() {
            if s.byte_offset >= offset && s.byte_offset < offset + 4 {
                let pos = ((s.byte_offset - offset) as u32) * 8 + s.bit_in_byte as u32;
                if s.level == 1 {
                    val |= 1 << pos;
                } else {
                    val &= !(1 << pos);
                }
            }
        }
        val
    }

    fn check_triggers(&self, register_id: &str, is_write: bool, value: Option<u32>) {
        if let Some(timing) = &self.descriptor.timing {
            for hook in timing {
                let triggered = match &hook.trigger {
                    labwired_config::TimingTrigger::Read { register } => {
                        !is_write && register == register_id
                    }
                    labwired_config::TimingTrigger::Write {
                        register,
                        value: trigger_value,
                        mask,
                    } => {
                        if !is_write || register != register_id {
                            false
                        } else if let Some(tv) = trigger_value {
                            let actual_val = value.unwrap_or(0);
                            if let Some(m) = mask {
                                (actual_val & m) == (*tv & m)
                            } else {
                                actual_val == *tv
                            }
                        } else {
                            true // Any write triggers it
                        }
                    }
                    labwired_config::TimingTrigger::Periodic { .. } => false,
                };

                if triggered {
                    self.inflight_events.borrow_mut().push(InflightEvent {
                        id: hook.id.clone(),
                        delay_remaining: hook.delay_cycles,
                        action: hook.action.clone(),
                        interrupt: hook.interrupt.clone(),
                        periodic_interval: None,
                    });
                }
            }
        }
    }

    fn apply_action(&self, action: &labwired_config::TimingAction) {
        let mut data = self.data.borrow_mut();
        match action {
            labwired_config::TimingAction::SetBits {
                register: reg_id,
                bits,
            } => {
                if let Some(reg) = self.descriptor.registers.iter().find(|r| &r.id == reg_id) {
                    let offset = reg.address_offset as usize;
                    // Apply bits to all bytes of the register based on its size
                    for i in 0..(reg.size / 8) {
                        let shift = i * 8;
                        let byte_bits = ((bits >> shift) & 0xFF) as u8;
                        data[offset + i as usize] |= byte_bits;
                    }
                }
            }
            labwired_config::TimingAction::ClearBits {
                register: reg_id,
                bits,
            } => {
                if let Some(reg) = self.descriptor.registers.iter().find(|r| &r.id == reg_id) {
                    let offset = reg.address_offset as usize;
                    for i in 0..(reg.size / 8) {
                        let shift = i * 8;
                        let byte_bits = ((bits >> shift) & 0xFF) as u8;
                        data[offset + i as usize] &= !byte_bits;
                    }
                }
            }
            labwired_config::TimingAction::WriteValue {
                register: reg_id,
                value,
            } => {
                if let Some(reg) = self.descriptor.registers.iter().find(|r| &r.id == reg_id) {
                    let offset = reg.address_offset as usize;
                    for i in 0..(reg.size / 8) {
                        let shift = i * 8;
                        let byte_val = ((value >> shift) & 0xFF) as u8;
                        data[offset + i as usize] = byte_val;
                    }
                }
            }
        }
    }

    fn has_read_trigger(&self) -> bool {
        self.descriptor.timing.as_ref().is_some_and(|timing| {
            timing
                .iter()
                .any(|hook| matches!(hook.trigger, labwired_config::TimingTrigger::Read { .. }))
        })
    }

    fn has_write_trigger(&self) -> bool {
        self.descriptor.timing.as_ref().is_some_and(|timing| {
            timing
                .iter()
                .any(|hook| matches!(hook.trigger, labwired_config::TimingTrigger::Write { .. }))
        })
    }
}

/// The pads of a `GPIO` descriptor, derived from its own `ENABLE` / `OUT` /
/// `IN` registers. A descriptor carries no pull configuration, so the driver
/// reports none.
impl PinPort for GenericPeripheral {
    fn pin_count(&self) -> u8 {
        if self.gpio_reg_offset("IN").is_some() {
            32
        } else {
            0
        }
    }

    /// `ENABLE` set: driving its `OUT` bit; clear: released. Without an
    /// `ENABLE` register the direction is not derivable: `None`.
    fn driver(&self, pin: u8) -> Option<PadDriver> {
        if self.gpio_pad_driven(pin)? {
            Some(PadDriver::drive(self.read_gpio_output(pin)?))
        } else {
            Some(PadDriver::OFF)
        }
    }

    fn external(&self, pin: u8) -> External {
        if pin < 32 && self.ext_mask & (1 << pin) != 0 {
            match self.read_gpio_input(pin) {
                Some(level) => External::Level(level),
                None => External::Released,
            }
        } else {
            External::Released
        }
    }

    /// A level is stored in the `IN` register; releasing leaves it there.
    fn set_external(&mut self, pin: u8, ext: External) -> Option<InputChange> {
        if pin >= 32 {
            return None;
        }
        let offset = self.gpio_reg_offset("IN")?;
        let mut value = self.read_register_storage_u32(offset)?;
        let before = value & (1u32 << pin) != 0;
        match ext {
            External::Level(level) => {
                if level {
                    value |= 1u32 << pin;
                } else {
                    value &= !(1u32 << pin);
                }
                if !self.write_register_storage_u32(offset, value) {
                    return None;
                }
                self.ext_mask |= 1 << pin;
            }
            External::Released => self.ext_mask &= !(1 << pin),
        }
        Some(InputChange {
            before,
            after: value & (1u32 << pin) != 0,
        })
    }

    fn input(&self, pin: u8) -> Option<bool> {
        self.read_gpio_input(pin)
    }

    /// A driven pad shows its `OUT` bit and any other pad shows `IN`.
    /// Without an `ENABLE` register the direction is not derivable, so the
    /// model reports nothing rather than guessing.
    fn level(&self, pin: u8) -> Option<bool> {
        if self.gpio_pad_driven(pin)? {
            self.read_gpio_output(pin)
        } else {
            self.read_gpio_input(pin)
        }
    }
}

impl Peripheral for GenericPeripheral {
    fn read(&self, offset: u64) -> SimResult<u8> {
        // Resolve the containing register in O(1) (see `reg_at_byte`).
        if let Some(idx) = self.reg_index_at(offset) {
            let reg = &self.descriptor.registers[idx];
            if reg.access == labwired_config::Access::WriteOnly {
                return Ok(0);
            }

            let mut data = self.data.borrow_mut();
            let val = data[offset as usize];

            // Side Effects: ReadAction
            if let Some(side_effects) = &reg.side_effects {
                if let Some(labwired_config::ReadAction::Clear) = side_effects.read_action {
                    data[offset as usize] = 0;
                }
            }

            self.check_triggers(&reg.id, false, None);

            return Ok(self.apply_stuck_byte(offset, val));
        }
        // Census (measurement only; no-op without the `silent-census` feature).
        // No register contains this byte, so the read below fabricates a zero —
        // the same silent path as a hand-written model's `_ => 0` arm, just
        // expressed as an `if let` miss instead of a match arm.
        crate::census::record_undecoded_reg_named(&self.descriptor.peripheral, offset, "read");
        Ok(0)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        // Resolve the containing register in O(1) (see `reg_at_byte`).
        if let Some(idx) = self.reg_index_at(offset) {
            let reg = &self.descriptor.registers[idx];
            let reg_start = reg.address_offset;
            if reg.access == labwired_config::Access::ReadOnly {
                return Ok(());
            }

            let mut data = self.data.borrow_mut();

            // Side Effects: WriteAction
            if let Some(side_effects) = &reg.side_effects {
                match side_effects.write_action {
                    Some(labwired_config::WriteAction::WriteOneToClear) => {
                        data[offset as usize] &= !value;
                    }
                    Some(labwired_config::WriteAction::WriteZeroToClear) => {
                        data[offset as usize] &= value;
                    }
                    _ => {
                        data[offset as usize] = value;
                    }
                }
            } else {
                data[offset as usize] = value;
            }

            // For triggers, we need the full register value being written (ideally).
            // But GenericPeripheral writes byte-by-byte.
            // This is a limitation: multi-byte write triggers might be tricky.
            // However, most SVD tools/emulators assume 32-bit writes for control registers.
            // Let's at least trigger on the byte write.
            // Calculate the shift for this byte within the register
            let byte_offset = (offset - reg_start) * 8;
            let shifted_val = (value as u32) << byte_offset;
            self.check_triggers(&reg.id, true, Some(shifted_val));

            return Ok(());
        }
        // Census (measurement only; no-op without the `silent-census` feature).
        // No register contains this byte, so the value is discarded — the same
        // silent path as a hand-written model's `_ => {}` arm.
        crate::census::record_undecoded_reg_named(&self.descriptor.peripheral, offset, "write");
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        // O(1) resolve, then require the full 32-bit access to fit inside that one
        // register (the old scan's `offset + 3 < reg_end` condition). Registers do
        // not overlap, so the byte-`offset` register is the only candidate: if it
        // does not contain all four bytes, no register does and we fall through to
        // the per-byte path below, byte-identical to the old miss behavior.
        if let Some(idx) = self.reg_index_at(offset) {
            let reg = &self.descriptor.registers[idx];
            let reg_end = reg.address_offset + (reg.size as u64 / 8);
            if offset + 3 < reg_end {
                if reg.access == labwired_config::Access::WriteOnly {
                    return Ok(0);
                }

                let mut data = self.data.borrow_mut();
                let b0 = data[offset as usize] as u32;
                let b1 = data[(offset + 1) as usize] as u32;
                let b2 = data[(offset + 2) as usize] as u32;
                let b3 = data[(offset + 3) as usize] as u32;
                let val = b0 | (b1 << 8) | (b2 << 16) | (b3 << 24);

                // Side Effects: ReadAction
                if let Some(side_effects) = &reg.side_effects {
                    if let Some(labwired_config::ReadAction::Clear) = side_effects.read_action {
                        data[offset as usize] = 0;
                        data[(offset + 1) as usize] = 0;
                        data[(offset + 2) as usize] = 0;
                        data[(offset + 3) as usize] = 0;
                    }
                }

                self.check_triggers(&reg.id, false, None);

                return Ok(self.apply_stuck_u32(offset, val));
            }
        }
        let b0 = self.read(offset)? as u32;
        let b1 = self.read(offset + 1)? as u32;
        let b2 = self.read(offset + 2)? as u32;
        let b3 = self.read(offset + 3)? as u32;
        Ok(b0 | (b1 << 8) | (b2 << 16) | (b3 << 24))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        // O(1) resolve + full-fit check, matching `read_u32`; non-fitting or
        // unmapped accesses fall through to the per-byte path unchanged.
        if let Some(idx) = self.reg_index_at(offset) {
            let reg = &self.descriptor.registers[idx];
            let reg_end = reg.address_offset + (reg.size as u64 / 8);
            if offset + 3 < reg_end {
                if reg.access == labwired_config::Access::ReadOnly {
                    return Ok(());
                }

                let mut data = self.data.borrow_mut();
                let b0 = (value & 0xFF) as u8;
                let b1 = ((value >> 8) & 0xFF) as u8;
                let b2 = ((value >> 16) & 0xFF) as u8;
                let b3 = ((value >> 24) & 0xFF) as u8;

                if let Some(side_effects) = &reg.side_effects {
                    match side_effects.write_action {
                        Some(labwired_config::WriteAction::WriteOneToClear) => {
                            data[offset as usize] &= !b0;
                            data[(offset + 1) as usize] &= !b1;
                            data[(offset + 2) as usize] &= !b2;
                            data[(offset + 3) as usize] &= !b3;
                        }
                        Some(labwired_config::WriteAction::WriteZeroToClear) => {
                            data[offset as usize] &= b0;
                            data[(offset + 1) as usize] &= b1;
                            data[(offset + 2) as usize] &= b2;
                            data[(offset + 3) as usize] &= b3;
                        }
                        _ => {
                            data[offset as usize] = b0;
                            data[(offset + 1) as usize] = b1;
                            data[(offset + 2) as usize] = b2;
                            data[(offset + 3) as usize] = b3;
                        }
                    }
                } else {
                    data[offset as usize] = b0;
                    data[(offset + 1) as usize] = b1;
                    data[(offset + 2) as usize] = b2;
                    data[(offset + 3) as usize] = b3;
                }

                self.check_triggers(&reg.id, true, Some(value));
                return Ok(());
            }
        }
        self.write(offset, (value & 0xFF) as u8)?;
        self.write(offset + 1, ((value >> 8) & 0xFF) as u8)?;
        self.write(offset + 2, ((value >> 16) & 0xFF) as u8)?;
        self.write(offset + 3, ((value >> 24) & 0xFF) as u8)?;
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        // O(1) resolve (see `reg_at_byte`); side-effect-free by contract.
        if let Some(idx) = self.reg_index_at(offset) {
            let reg = &self.descriptor.registers[idx];
            if reg.access == labwired_config::Access::WriteOnly {
                return Some(0);
            }
            return self.data.borrow().get(offset as usize).copied();
        }
        Some(0)
    }

    fn tick(&mut self) -> PeripheralTickResult {
        let mut result = PeripheralTickResult::default();
        let mut events = self.inflight_events.borrow_mut();

        let mut i = 0;
        let mut re_adds = Vec::new();
        while i < events.len() {
            if events[i].delay_remaining > 0 {
                events[i].delay_remaining -= 1;
                i += 1;
            } else {
                let event = events.remove(i);
                self.apply_action(&event.action);
                if let Some(ref int_name) = event.interrupt {
                    if let Some(ints) = &self.descriptor.interrupts {
                        if let Some(&val) = ints.get(int_name) {
                            result.explicit_irqs.get_or_insert_with(Vec::new).push(val);
                        }
                    }
                }

                // If periodic, re-add after the loop to prevent same-tick processing
                if let Some(interval) = event.periodic_interval {
                    re_adds.push(InflightEvent {
                        id: event.id,
                        delay_remaining: interval,
                        action: event.action,
                        interrupt: event.interrupt,
                        periodic_interval: Some(interval),
                    });
                }

                // Do not increment i, as we removed an element
            }
        }
        events.extend(re_adds);

        result.dma_signals = None;
        result.cycles = 0;
        result
    }

    fn legacy_tick_active(&self) -> bool {
        !self.inflight_events.borrow().is_empty() || self.has_read_trigger()
    }

    fn legacy_tick_dynamic(&self) -> bool {
        !self.has_read_trigger() && self.has_write_trigger()
    }

    /// A declarative register bank does walk work only when it can hold an
    /// inflight timed event — which requires a read/write trigger in the
    /// descriptor (or an event armed at construction). A trigger-free bank's
    /// `tick()` loop body can never execute, so it is walk-independent for every
    /// state. Mirrors the `legacy_tick_active`/`legacy_tick_dynamic` reachability
    /// above: no trigger AND no live event ⇒ can never become tick-active.
    fn needs_legacy_walk(&self) -> bool {
        self.has_read_trigger()
            || self.has_write_trigger()
            || !self.inflight_events.borrow().is_empty()
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "peripheral": self.descriptor.peripheral,
            "data": *self.data.borrow()
        })
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        let hooks = self.descriptor.timing.as_deref().unwrap_or(&[]);
        let inflight = self
            .inflight_events
            .borrow()
            .iter()
            .map(|event| {
                let index = hooks
                    .iter()
                    .position(|hook| {
                        hook.id == event.id
                            && hook.action == event.action
                            && hook.interrupt == event.interrupt
                            && periodic_interval(hook) == event.periodic_interval
                            && event.delay_remaining
                                <= event.periodic_interval.unwrap_or(hook.delay_cycles)
                    })
                    .expect("an inflight declarative event belongs to its descriptor");
                (index as u32, event.delay_remaining)
            })
            .collect();
        bincode::serialize(&GenericRuntimeSnapshot {
            version: 1,
            descriptor_key: self.descriptor_key,
            data: self.data.borrow().clone(),
            inflight,
            stuck_bits: self.stuck_bits.borrow().clone(),
            ext_mask: self.ext_mask,
            reset_values: self
                .descriptor
                .registers
                .iter()
                .map(|reg| reg.reset_value)
                .collect(),
        })
        .expect("serialize declarative runtime snapshot")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        use bincode::Options;
        // Older snapshots carried no register-bank state. Keep boot-before-init
        // snapshots loadable; a nonempty payload must pass every check below.
        if bytes.is_empty() {
            return Ok(());
        }
        let invalid = |reason: &str| {
            crate::SimulationError::NotImplemented(format!("GenericPeripheral snapshot: {reason}"))
        };
        let snapshot: GenericRuntimeSnapshot = bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .reject_trailing_bytes()
            .deserialize(bytes)
            .map_err(|error| invalid(&format!("decode failed: {error}")))?;
        if snapshot.version != 1
            || snapshot.descriptor_key != self.descriptor_key
            || snapshot.data.len() != self.data.borrow().len()
            || snapshot.reset_values.len() != self.descriptor.registers.len()
        {
            return Err(invalid("schema or register topology mismatch"));
        }
        if snapshot.stuck_bits.iter().any(|bit| {
            self.reg_index_at(bit.byte_offset).is_none() || bit.bit_in_byte >= 8 || bit.level > 1
        }) || (snapshot.ext_mask != 0 && self.gpio_reg_offset("IN").is_none())
        {
            return Err(invalid("invalid pin or fault state"));
        }
        let hooks = self.descriptor.timing.as_deref().unwrap_or(&[]);
        let mut events = Vec::with_capacity(snapshot.inflight.len());
        for (index, delay_remaining) in snapshot.inflight {
            let hook = hooks
                .get(index as usize)
                .ok_or_else(|| invalid("unknown timing hook"))?;
            let interval = periodic_interval(hook);
            if delay_remaining > interval.unwrap_or(hook.delay_cycles) {
                return Err(invalid("invalid remaining event delay"));
            }
            events.push(InflightEvent {
                id: hook.id.clone(),
                delay_remaining,
                action: hook.action.clone(),
                interrupt: hook.interrupt.clone(),
                periodic_interval: interval,
            });
        }
        // Validate the entire payload before replacing any live state.
        *self.data.get_mut() = snapshot.data;
        *self.inflight_events.get_mut() = events;
        *self.stuck_bits.get_mut() = snapshot.stuck_bits;
        self.ext_mask = snapshot.ext_mask;
        for (reg, reset_value) in self
            .descriptor
            .registers
            .iter_mut()
            .zip(snapshot.reset_values)
        {
            reg.reset_value = reset_value;
        }
        Ok(())
    }

    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        if pin >= 32 {
            return None;
        }
        let offset = self.gpio_reg_offset("IN")?;
        let value = self.read_register_storage_u32(offset)?;
        Some((value & (1u32 << pin)) != 0)
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        if pin >= 32 {
            return None;
        }
        let offset = self.gpio_reg_offset("OUT")?;
        let value = self.read_register_storage_u32(offset)?;
        Some((value & (1u32 << pin)) != 0)
    }

    fn gpio_routing(&self, pin: u8) -> Option<crate::peripherals::gpio::GpioRouting> {
        use crate::peripherals::gpio::{GpioMode, GpioRouting};
        let mode = if self.gpio_pad_driven(pin)? {
            GpioMode::Output
        } else {
            GpioMode::Input
        };
        Some(GpioRouting { mode, func: None })
    }

    /// A `GPIO` descriptor with an `IN` register owns pads; every other
    /// declarative peripheral owns none.
    fn pins(&self) -> Option<&dyn PinPort> {
        self.gpio_reg_offset("IN").map(|_| self as &dyn PinPort)
    }

    fn pins_mut(&mut self) -> Option<&mut dyn PinPort> {
        self.gpio_reg_offset("IN")?;
        Some(self)
    }

    fn peripheral_descriptor(&self) -> Option<PeripheralDescriptor> {
        Some(self.descriptor.clone())
    }

    /// Expose the descriptor's register layout to the universal inspect
    /// interface. This one method makes every declarative peripheral — the
    /// whole ESP32-C3/S3 register wall — decode named registers + bitfields for
    /// free (see [`crate::inspect::default_inspect`]).
    fn describe_registers(&self) -> Option<Vec<crate::inspect::RegisterSchema>> {
        Some(crate::inspect::schema_from_descriptor(&self.descriptor))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use labwired_config::{Access, PeripheralDescriptor, RegisterDescriptor};

    fn mock_descriptor() -> PeripheralDescriptor {
        PeripheralDescriptor {
            peripheral: "Mock".to_string(),
            version: "1.0".to_string(),
            registers: vec![
                RegisterDescriptor {
                    id: "REG1".to_string(),
                    address_offset: 0x00,
                    size: 32,
                    access: Access::ReadWrite,
                    reset_value: 0x12345678,
                    fields: vec![],
                    side_effects: None,
                },
                RegisterDescriptor {
                    id: "RO_REG".to_string(),
                    address_offset: 0x04,
                    size: 8,
                    access: Access::ReadOnly,
                    reset_value: 0xAA,
                    fields: vec![],
                    side_effects: None,
                },
                RegisterDescriptor {
                    id: "WO_REG".to_string(),
                    address_offset: 0x05,
                    size: 8,
                    access: Access::WriteOnly,
                    reset_value: 0x00,
                    fields: vec![],
                    side_effects: None,
                },
                RegisterDescriptor {
                    id: "REG16".to_string(),
                    address_offset: 0x06,
                    size: 16,
                    access: Access::ReadWrite,
                    reset_value: 0xABCD,
                    fields: vec![],
                    side_effects: None,
                },
            ],
            interrupts: None,
            timing: None,
        }
    }

    #[test]
    fn runtime_snapshot_restores_esp32c3_interrupt_register_bank() {
        let descriptor = PeripheralDescriptor::from_yaml(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../configs/peripherals/esp32c3/interrupt_core0.yaml"
        )))
        .unwrap();
        let mut original = GenericPeripheral::new(descriptor.clone());
        // The OLED firmware routes SYSTIMER targets 0/2 to CPU lines 4/2,
        // then enables/prioritizes those lines and sets threshold 1.
        let registers = [
            (0x94, 4),
            (0x9c, 2),
            (0x104, 117_442_204),
            (0x108, 1 << 24),
            (0x11c, 1),
            (0x124, 1),
            (0x194, 1),
        ];
        for (offset, value) in registers {
            original.write_u32(offset, value).unwrap();
        }
        let bytes = original.runtime_snapshot();
        assert!(!bytes.is_empty());
        let mut resumed = GenericPeripheral::new(descriptor);
        assert_eq!(resumed.peek_u32_raw(0x104), Some(0));
        resumed.restore_runtime_snapshot(&bytes).unwrap();
        for (offset, value) in registers {
            assert_eq!(
                resumed.peek_u32_raw(offset),
                Some(value),
                "offset {offset:#x}"
            );
        }
        assert_eq!(resumed.runtime_snapshot(), bytes);
    }

    #[test]
    fn runtime_snapshot_preserves_timing_faults_and_external_gpio_drive() {
        let mut descriptor = gpio_descriptor(true);
        descriptor.interrupts = Some(std::collections::BTreeMap::from([("ready".into(), 7)]));
        descriptor.timing = Some(vec![labwired_config::TimingDescriptor {
            id: "write_done".into(),
            trigger: labwired_config::TimingTrigger::Write {
                register: "OUT".into(),
                value: None,
                mask: None,
            },
            delay_cycles: 2,
            action: labwired_config::TimingAction::SetBits {
                register: "IN".into(),
                bits: 1 << 7,
            },
            interrupt: Some("ready".into()),
        }]);
        let mut original = GenericPeripheral::new(descriptor.clone());
        assert!(original.force_register_value("OUT", 0x55));
        assert!(original.force_stuck_bit("OUT", 4, 1));
        original.write_u32(0x04, 0).unwrap();
        original.write_u32(0x20, 1 << 3).unwrap();
        assert!(original.set_gpio_input(3, true));
        original.tick(); // Keep one pending countdown cycle at the save point.
        let mut resumed = GenericPeripheral::new(descriptor);
        resumed
            .restore_runtime_snapshot(&original.runtime_snapshot())
            .unwrap();
        assert_eq!(resumed.get_descriptor().registers[0].reset_value, 0x55);
        assert_eq!(resumed.read_u32(0x04).unwrap(), 1 << 4);
        assert_eq!(
            resumed.read_gpio_pad_drive(3),
            Some(crate::logic_capture::PadDrive::Contention)
        );
        for _ in 0..3 {
            let expected = original.tick();
            let actual = resumed.tick();
            assert_eq!(actual.explicit_irqs, expected.explicit_irqs);
            assert_eq!(resumed.runtime_snapshot(), original.runtime_snapshot());
        }
        assert!(resumed.read_gpio_input(7).unwrap());
    }

    #[test]
    fn runtime_snapshot_keeps_periodic_event_phase() {
        let mut descriptor = mock_descriptor();
        descriptor.timing = Some(vec![labwired_config::TimingDescriptor {
            id: "periodic".into(),
            trigger: labwired_config::TimingTrigger::Periodic { period_cycles: 3 },
            delay_cycles: 0,
            action: labwired_config::TimingAction::SetBits {
                register: "REG1".into(),
                bits: 1,
            },
            interrupt: None,
        }]);
        let mut original = GenericPeripheral::new(descriptor.clone());
        original.write_u32(0, 0).unwrap();
        original.tick();
        original.tick();
        let mut resumed = GenericPeripheral::new(descriptor);
        resumed
            .restore_runtime_snapshot(&original.runtime_snapshot())
            .unwrap();
        for _ in 0..8 {
            original.tick();
            resumed.tick();
            assert_eq!(resumed.runtime_snapshot(), original.runtime_snapshot());
        }
    }

    #[test]
    fn runtime_snapshot_preserves_duplicate_timing_hook_ids() {
        let mut descriptor = mock_descriptor();
        descriptor.timing = Some(
            [2, 4]
                .into_iter()
                .map(|delay_cycles| labwired_config::TimingDescriptor {
                    id: "shared_id".into(),
                    trigger: labwired_config::TimingTrigger::Write {
                        register: "REG1".into(),
                        value: None,
                        mask: None,
                    },
                    delay_cycles,
                    action: labwired_config::TimingAction::SetBits {
                        register: "REG1".into(),
                        bits: 1,
                    },
                    interrupt: None,
                })
                .collect(),
        );
        let mut original = GenericPeripheral::new(descriptor.clone());
        original.write_u32(0, 0).unwrap();
        let mut resumed = GenericPeripheral::new(descriptor);
        resumed
            .restore_runtime_snapshot(&original.runtime_snapshot())
            .unwrap();
        for _ in 0..6 {
            original.tick();
            resumed.tick();
            assert_eq!(resumed.runtime_snapshot(), original.runtime_snapshot());
        }
    }

    #[test]
    fn runtime_snapshot_rejects_invalid_payloads_without_partial_restore() {
        let mut original = GenericPeripheral::new(mock_descriptor());
        original.write_u32(0, 0x1234).unwrap();
        let bytes = original.runtime_snapshot();
        let mut resumed = GenericPeripheral::new(mock_descriptor());
        resumed.write_u32(0, 0x5678).unwrap();
        let before = resumed.runtime_snapshot();
        let mut snapshot: GenericRuntimeSnapshot = bincode::deserialize(&bytes).unwrap();
        snapshot.data.pop();
        let wrong_length = bincode::serialize(&snapshot).unwrap();
        snapshot = bincode::deserialize(&bytes).unwrap();
        snapshot.version = 2;
        let wrong_version = bincode::serialize(&snapshot).unwrap();
        snapshot = bincode::deserialize(&bytes).unwrap();
        snapshot.inflight.push((0, 0)); // This descriptor has no timing hooks.
        let wrong_hook = bincode::serialize(&snapshot).unwrap();
        let mut trailing = bytes.clone();
        trailing.push(0);
        for invalid in [
            &bytes[..bytes.len() - 1],
            &wrong_length,
            &wrong_version,
            &wrong_hook,
            &trailing,
        ] {
            assert!(resumed.restore_runtime_snapshot(invalid).is_err());
            assert_eq!(resumed.runtime_snapshot(), before);
        }
        let mut wrong_topology = mock_descriptor();
        wrong_topology.registers[0].id = "OTHER".into();
        let mut other = GenericPeripheral::new(wrong_topology);
        assert!(other.restore_runtime_snapshot(&bytes).is_err());
        resumed.restore_runtime_snapshot(&[]).unwrap();
        assert_eq!(
            resumed.runtime_snapshot(),
            before,
            "legacy empty blob leaves live state alone"
        );
    }

    #[test]
    fn force_register_value_overrides_and_persists_through_reset() {
        let mut p = GenericPeripheral::new(mock_descriptor());
        assert_eq!(p.read_u32(0x00).unwrap(), 0x12345678);

        assert!(p.force_register_value("REG1", 0xDEAD_BEEF));
        assert_eq!(
            p.read_u32(0x00).unwrap(),
            0xDEAD_BEEF,
            "live value overridden"
        );

        // The reset value is overridden too, so a fresh peripheral built from
        // the mutated descriptor seeds the faulted value (survives reset).
        let p2 = GenericPeripheral::new(p.get_descriptor().clone());
        assert_eq!(
            p2.read_u32(0x00).unwrap(),
            0xDEAD_BEEF,
            "reset value overridden"
        );

        // An unknown register is reported, not silently ignored.
        assert!(!p.force_register_value("NOPE", 1));
    }

    #[test]
    fn stuck_at_bit_holds_bit_through_writes() {
        let mut p = GenericPeripheral::new(mock_descriptor());

        // Force REG1 bit 5 stuck high; a write of 0 cannot clear it.
        assert!(p.force_stuck_bit("REG1", 5, 1));
        assert_eq!(p.read_u32(0).unwrap() & (1 << 5), 1 << 5);
        p.write_u32(0, 0).unwrap();
        assert_eq!(
            p.read_u32(0).unwrap() & (1 << 5),
            1 << 5,
            "stuck-high bit survives a zero write"
        );

        // Force REG1 bit 3 stuck low (reset 0x12345678 has bit 3 set).
        assert!(p.force_stuck_bit("REG1", 3, 0));
        assert_eq!(
            p.read_u32(0).unwrap() & (1 << 3),
            0,
            "stuck-low bit reads 0 despite a set reset value"
        );

        // Out-of-range bit and unknown register are rejected.
        assert!(!p.force_stuck_bit("REG1", 32, 1));
        assert!(!p.force_stuck_bit("NOPE", 0, 1));
    }

    #[test]
    fn test_initialization() {
        let p = GenericPeripheral::new(mock_descriptor());
        assert_eq!(p.read(0x00).unwrap(), 0x78);
        assert_eq!(p.read(0x01).unwrap(), 0x56);
        assert_eq!(p.read(0x02).unwrap(), 0x34);
        assert_eq!(p.read(0x03).unwrap(), 0x12);
        assert_eq!(p.read(0x04).unwrap(), 0xAA);
        assert_eq!(p.read(0x06).unwrap(), 0xCD);
        assert_eq!(p.read(0x07).unwrap(), 0xAB);
    }

    #[test]
    fn test_write_read() {
        let mut p = GenericPeripheral::new(mock_descriptor());
        p.write(0x00, 0xFF).unwrap();
        assert_eq!(p.read(0x00).unwrap(), 0xFF);
    }

    #[test]
    fn test_read_only() {
        let mut p = GenericPeripheral::new(mock_descriptor());
        p.write(0x04, 0xBB).unwrap();
        assert_eq!(p.read(0x04).unwrap(), 0xAA); // Should not change
    }

    #[test]
    fn test_write_only() {
        let mut p = GenericPeripheral::new(mock_descriptor());
        p.write(0x05, 0xCC).unwrap();
        assert_eq!(p.read(0x05).unwrap(), 0x00); // Reads should return 0
    }

    #[test]
    fn test_16bit_access() {
        let mut p = GenericPeripheral::new(mock_descriptor());
        p.write(0x06, 0x11).unwrap();
        p.write(0x07, 0x22).unwrap();
        assert_eq!(p.read(0x06).unwrap(), 0x11);
        assert_eq!(p.read(0x07).unwrap(), 0x22);
    }
    #[test]
    fn test_side_effects_rtc() {
        let mut desc = mock_descriptor();
        desc.registers[0].side_effects = Some(labwired_config::SideEffectsDescriptor {
            read_action: Some(labwired_config::ReadAction::Clear),
            write_action: None,
            on_read: None,
            on_write: None,
        });

        let p = GenericPeripheral::new(desc);
        // Initial reset value 0x12345678. Byte 0 is 0x78.
        assert_eq!(p.read(0x00).unwrap(), 0x78);
        assert_eq!(p.read(0x00).unwrap(), 0x00); // Cleared on read
    }

    /// The universal inspect interface decodes a REAL esp32c3 declarative
    /// peripheral (SYSTEM) into NAMED registers with decoded bitfields — the
    /// whole register wall for free, no bespoke code. Mirrors the proposal's
    /// worked example: CPU_PER_CONF (offset 8, reset 12) → CPUPERIOD_SEL=0,
    /// PLL_FREQ_SEL=1.
    #[test]
    fn inspect_decodes_named_esp32c3_registers_and_fields() {
        use crate::inspect::InspectOpts;
        use crate::Peripheral;

        let yaml = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../configs/peripherals/esp32c3/system.yaml"
        ));
        let desc = labwired_config::PeripheralDescriptor::from_yaml(yaml).unwrap();
        let p = GenericPeripheral::new(desc);

        let pi = p.inspect(0x600C_0000, "system", &InspectOpts::default());
        assert_eq!(pi.kind, "declarative");
        assert_eq!(pi.base, 0x600C_0000);

        let cpc = pi
            .registers
            .iter()
            .find(|r| r.name == "CPU_PER_CONF")
            .expect("CPU_PER_CONF register decoded by name");
        assert_eq!(cpc.offset, 8);
        assert_eq!(cpc.value, Some(12), "live reset word read via peek");
        assert_eq!(cpc.access, "rw");

        let period = cpc
            .fields
            .iter()
            .find(|f| f.name == "CPUPERIOD_SEL")
            .expect("CPUPERIOD_SEL field decoded by name");
        assert_eq!(period.bits, [1, 0]);
        assert_eq!(period.value, 0);

        let pll = cpc
            .fields
            .iter()
            .find(|f| f.name == "PLL_FREQ_SEL")
            .expect("PLL_FREQ_SEL field decoded by name");
        assert_eq!(pll.bits, [2, 2]);
        assert_eq!(pll.value, 1, "bit 2 of reset value 0b1100 is set");
    }

    /// Decode must be side-effect-free: `default_inspect` reads via `peek`, not
    /// `read`, so inspecting a read-to-clear register never perturbs it.
    #[test]
    fn inspect_uses_peek_not_read_no_side_effects() {
        use crate::inspect::InspectOpts;
        use crate::Peripheral;

        let mut desc = mock_descriptor();
        // REG1 (offset 0, reset 0x12345678) clears itself when read().
        desc.registers[0].side_effects = Some(labwired_config::SideEffectsDescriptor {
            read_action: Some(labwired_config::ReadAction::Clear),
            write_action: None,
            on_read: None,
            on_write: None,
        });
        let p = GenericPeripheral::new(desc);

        let reg_value = |pi: &crate::inspect::PeripheralInspect| {
            pi.registers
                .iter()
                .find(|r| r.name == "REG1")
                .unwrap()
                .value
        };

        // Inspecting twice yields the same value — read() would have cleared it.
        let first = p.inspect(0, "mock", &InspectOpts::default());
        assert_eq!(reg_value(&first), Some(0x1234_5678));
        let second = p.inspect(0, "mock", &InspectOpts::default());
        assert_eq!(
            reg_value(&second),
            Some(0x1234_5678),
            "inspect is side-effect-free: read-to-clear reg unchanged"
        );

        // A genuine read() does clear it — proving the side effect exists and
        // that inspect deliberately avoided it.
        assert_eq!(p.read(0x00).unwrap(), 0x78);
        assert_eq!(p.read(0x00).unwrap(), 0x00);
    }

    #[test]
    fn test_side_effects_w1c() {
        let mut desc = mock_descriptor();
        desc.registers[0].side_effects = Some(labwired_config::SideEffectsDescriptor {
            read_action: None,
            write_action: Some(labwired_config::WriteAction::WriteOneToClear),
            on_read: None,
            on_write: None,
        });

        let mut p = GenericPeripheral::new(desc);
        // Byte 0 is 0x78 (binary: 0111 1000)
        // Write 0x08 (binary: 0000 1000) to clear bit 3.
        p.write(0x00, 0x08).unwrap();
        assert_eq!(p.read(0x00).unwrap(), 0x70); // 0x78 & !0x08 = 0x70
    }

    #[test]
    fn test_timing_hook() {
        let mut desc = mock_descriptor();
        desc.registers.push(RegisterDescriptor {
            id: "STATUS".to_string(),
            address_offset: 0x10,
            size: 8,
            access: Access::ReadOnly,
            reset_value: 0x00,
            fields: vec![],
            side_effects: None,
        });
        desc.interrupts = Some({
            let mut h = std::collections::BTreeMap::new();
            h.insert("INT1".to_string(), 42);
            h
        });
        desc.timing = Some(vec![labwired_config::TimingDescriptor {
            id: "test_evt".to_string(),
            trigger: labwired_config::TimingTrigger::Write {
                register: "REG1".to_string(),
                value: Some(0xAA),
                mask: None,
            },
            delay_cycles: 1,
            action: labwired_config::TimingAction::SetBits {
                register: "STATUS".to_string(),
                bits: 0x01,
            },
            interrupt: Some("INT1".to_string()),
        }]);

        let mut p = GenericPeripheral::new(desc);

        // Byte 0 of REG1 is 0x78 initially.
        // Write 0xAA to trigger.
        p.write(0x00, 0xAA).unwrap();

        // Tick 1: Still 1 cycle left (delay 1 -> 0)
        let res = p.tick();
        assert!(res.explicit_irqs.is_none());
        assert_eq!(p.read(0x10).unwrap(), 0x00);

        // Tick 2: Triggered! (delay 0 -> fired)
        let res = p.tick();
        assert!(res.explicit_irqs.as_ref().is_some_and(|v| v.contains(&42)));
        assert_eq!(p.read(0x10).unwrap(), 0x01);
    }

    #[test]
    fn test_immediate_timing() {
        let mut desc = mock_descriptor();
        desc.registers.push(RegisterDescriptor {
            id: "STATUS".to_string(),
            address_offset: 0x10,
            size: 8,
            access: Access::ReadOnly,
            reset_value: 0x00,
            fields: vec![],
            side_effects: None,
        });
        desc.timing = Some(vec![labwired_config::TimingDescriptor {
            id: "immediate".to_string(),
            trigger: labwired_config::TimingTrigger::Read {
                register: "REG1".to_string(),
            },
            delay_cycles: 0,
            action: labwired_config::TimingAction::WriteValue {
                register: "STATUS".to_string(),
                value: 0x55,
            },
            interrupt: None,
        }]);

        let mut p = GenericPeripheral::new(desc);

        // Read to trigger
        p.read(0x00).unwrap();

        // Tick 1: Triggered immediately (delay 0 -> fired)
        let res = p.tick();
        assert!(res.explicit_irqs.is_none());
        assert_eq!(p.read(0x10).unwrap(), 0x55);
    }

    #[test]
    fn test_periodic_timing() {
        let mut desc = mock_descriptor();
        desc.registers.push(RegisterDescriptor {
            id: "STATUS".to_string(),
            address_offset: 0x10,
            size: 8,
            access: Access::ReadWrite,
            reset_value: 0x00,
            fields: vec![],
            side_effects: None,
        });
        desc.timing = Some(vec![labwired_config::TimingDescriptor {
            id: "heartbeat".to_string(),
            trigger: labwired_config::TimingTrigger::Periodic { period_cycles: 1 },
            delay_cycles: 0,
            action: labwired_config::TimingAction::SetBits {
                register: "STATUS".to_string(),
                bits: 0x01,
            },
            interrupt: None,
        }]);

        let mut p = GenericPeripheral::new(desc);

        // Tick 1: 1 -> 0
        p.tick();
        assert_eq!(p.read(0x10).unwrap(), 0x00);

        // Tick 2: 0 -> fired, re-added with 1
        p.tick();
        assert_eq!(p.read(0x10).unwrap(), 0x01);

        // Clear it
        p.write(0x10, 0x00).unwrap();
        assert_eq!(p.read(0x10).unwrap(), 0x00);

        // Tick 3: 1 -> 0
        p.tick();
        assert_eq!(p.read(0x10).unwrap(), 0x00);

        // Tick 4: 0 -> fired
        p.tick();
        assert_eq!(p.read(0x10).unwrap(), 0x01);
    }

    fn gpio_descriptor(with_enable: bool) -> PeripheralDescriptor {
        let reg = |id: &str, off: u64| RegisterDescriptor {
            id: id.to_string(),
            address_offset: off,
            size: 32,
            access: Access::ReadWrite,
            reset_value: 0,
            fields: vec![],
            side_effects: None,
        };
        let mut registers = vec![reg("OUT", 0x04), reg("IN", 0x3C)];
        if with_enable {
            registers.push(reg("ENABLE", 0x20));
        }
        PeripheralDescriptor {
            peripheral: "GPIO".to_string(),
            version: "1.0".to_string(),
            registers,
            interrupts: None,
            timing: None,
        }
    }

    /// An `ENABLE` register makes the pad derivable: a driven pad shows its
    /// `OUT` bit, any other pad the `IN` level.
    #[test]
    fn declarative_gpio_pad_follows_enable_out_and_in() {
        use crate::logic_capture::PadDrive;
        let mut p = GenericPeripheral::new(gpio_descriptor(true));
        assert_eq!(p.read_gpio_pad(3), Some(false));
        assert_eq!(p.read_gpio_pad_drive(3), Some(PadDrive::HighZ));
        assert!(p.set_gpio_input(3, true));
        assert_eq!(p.read_gpio_pad(3), Some(true), "input shows IN");
        p.write_u32(0x20, 1 << 3).unwrap(); // ENABLE: driver on
        assert_eq!(p.read_gpio_pad(3), Some(false), "driven pad shows OUT");
        assert_eq!(
            p.read_gpio_pad_drive(3),
            Some(PadDrive::Contention),
            "a probe sees the driven 0 fight the outside 1"
        );
        assert_eq!(crate::pins::own_drive(&p, 3), Some(PadDrive::Driven));
        p.write_u32(0x04, 1 << 3).unwrap(); // OUT
        assert_eq!(p.read_gpio_pad(3), Some(true));
        assert_eq!(p.read_gpio_pad_drive(3), Some(PadDrive::Driven));
        let routing = p.gpio_routing(3).unwrap();
        assert_eq!(routing.mode, crate::peripherals::gpio::GpioMode::Output);
        assert_eq!(p.read_gpio_pad(32), None);
    }

    /// No `ENABLE` register: direction is not derivable, so nothing is
    /// invented.
    #[test]
    fn declarative_gpio_without_enable_reports_no_pad() {
        let p = GenericPeripheral::new(gpio_descriptor(false));
        assert_eq!(p.read_gpio_pad(3), None);
        assert_eq!(p.read_gpio_pad_drive(3), None);
        assert!(p.gpio_routing(3).is_none());
    }
}
