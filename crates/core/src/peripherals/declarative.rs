// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::{Peripheral, PeripheralTickResult, SimResult};
use labwired_config::PeripheralDescriptor;
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
#[derive(Debug)]
struct StuckBit {
    byte_offset: u64,
    bit_in_byte: u8,
    level: u8,
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
}

impl GenericPeripheral {
    /// Creates a new `GenericPeripheral` from a descriptor.
    ///
    /// The backing memory is automatically sized to accommodate the highest register offset,
    /// and all registers are initialized to their specified `reset_value`.
    pub fn new(descriptor: PeripheralDescriptor) -> Self {
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
            data: RefCell::new(data),
            reg_at_byte,
            inflight_events: RefCell::new(Vec::new()),
            stuck_bits: RefCell::new(Vec::new()),
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

    fn set_gpio_input(&mut self, pin: u8, level: bool) -> bool {
        if pin >= 32 {
            return false;
        }
        let Some(offset) = self.gpio_reg_offset("IN") else {
            return false;
        };
        let Some(mut value) = self.read_register_storage_u32(offset) else {
            return false;
        };
        if level {
            value |= 1u32 << pin;
        } else {
            value &= !(1u32 << pin);
        }
        self.write_register_storage_u32(offset, value)
    }

    fn peripheral_descriptor(&self) -> Option<PeripheralDescriptor> {
        Some(self.descriptor.clone())
    }

    /// Expose the descriptor's register layout to the universal inspect
    /// interface. This one method makes every declarative peripheral — the
    /// whole ESP32-C3/S3 register wall — decode named registers + bitfields for
    /// free (see [`crate::inspect::default_inspect`]).
    fn describe_registers(&self) -> Option<Vec<crate::inspect::RegisterSchema>> {
        Some(
            self.descriptor
                .registers
                .iter()
                .map(|reg| crate::inspect::RegisterSchema {
                    name: reg.id.clone(),
                    offset: reg.address_offset,
                    size: reg.size,
                    access: match reg.access {
                        labwired_config::Access::ReadWrite => "rw",
                        labwired_config::Access::ReadOnly => "ro",
                        labwired_config::Access::WriteOnly => "wo",
                    },
                    fields: reg
                        .fields
                        .iter()
                        .map(|f| crate::inspect::FieldSchema {
                            name: f.name.clone(),
                            bits: f.bit_range,
                        })
                        .collect(),
                })
                .collect(),
        )
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
        assert_eq!(cpc.value, 12, "live reset word read via peek");
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
        assert_eq!(reg_value(&first), 0x1234_5678);
        let second = p.inspect(0, "mock", &InspectOpts::default());
        assert_eq!(
            reg_value(&second),
            0x1234_5678,
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
            let mut h = std::collections::HashMap::new();
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
}
