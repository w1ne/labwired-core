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

use labwired_config::{
    DeviceTimer, Encode, Endian, LabDescriptor, ObservableSpec, ReadAction, RegisterSpec,
    TimerStart, TimingAction, WriteAction,
};

use crate::peripherals::kit::LabRef;

/// `Box::leak`s a slice of config-layer [`LabDescriptor`]s into the `'static`
/// [`LabRef`]s a `KitMetadata` requires. Shared by both declarative engines
/// (SPI and I²C) so a descriptor's `metadata.labs` becomes the kit's
/// advertised demo labs identically either way.
pub(crate) fn leak_labs(labs: &[LabDescriptor]) -> &'static [LabRef] {
    let leaked: Vec<LabRef> = labs
        .iter()
        .map(|l| LabRef {
            board_id: Box::leak(l.board_id.clone().into_boxed_str()),
            chip: Box::leak(l.chip.clone().into_boxed_str()),
            example_dir: Box::leak(l.example_dir.clone().into_boxed_str()),
            demo_elf: Box::leak(l.demo_elf.clone().into_boxed_str()),
        })
        .collect();
    Box::leak(leaked.into_boxed_slice())
}

/// Largest value representable in `width` bytes, as f64 (width ≤ 4).
pub(crate) fn width_max(width: u8) -> f64 {
    ((1u64 << (8 * width as u64)) - 1) as f64
}

/// Apply a linear encode (scale/offset/clamp) plus an extra scale factor,
/// yielding the raw integer packed into a `width`-byte word.
pub(crate) fn encode_raw(
    value: f64,
    enc: Option<&Encode>,
    extra_scale: f64,
    width: u8,
    signed: bool,
) -> u32 {
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
    let bits = 8 * width as u32;
    let mask = if bits >= 32 {
        u32::MAX
    } else {
        (1u32 << bits) - 1
    };
    if signed {
        let lo = -(2f64.powi((bits - 1) as i32));
        let hi = 2f64.powi((bits - 1) as i32) - 1.0;
        let v = raw.round().clamp(lo, hi) as i64;
        (v as u32) & mask
    } else {
        raw.round().clamp(0.0, width_max(width)) as u32
    }
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

/// One `scale_from` factor: the value another register's bit-field selects (1.0 if unmapped).
pub(crate) fn scale_from_one(
    sf: &labwired_config::ScaleFrom,
    reg_values: &HashMap<String, u32>,
) -> f64 {
    let regval = reg_values.get(&sf.register).copied().unwrap_or(0);
    let field = (regval >> sf.shift as u32) & sf.mask;
    sf.map.get(&field).copied().unwrap_or(1.0)
}

/// Product of a register's `scale_from` factors, folded left-to-right from 1.0.
pub(crate) fn scale_from_product(reg: &RegisterSpec, reg_values: &HashMap<String, u32>) -> f64 {
    reg.scale_from
        .iter()
        .fold(1.0, |acc, sf| acc * scale_from_one(sf, reg_values))
}

/// Divide dual of `encode_raw`: count = round(value / resolution), clamped. A
/// zero/negative resolution clamps to max (defensive).
pub(crate) fn divide_raw(value: f64, resolution: f64, width: u8) -> u32 {
    if resolution <= 0.0 {
        return width_max(width) as u32;
    }
    (value / resolution).round().clamp(0.0, width_max(width)) as u32
}

/// The bytes a read of `reg` returns: a sourced+encoded measurement, or the
/// plain stored value (seeded to reset) for a storage register.
pub(crate) fn register_read_bytes(
    reg: &RegisterSpec,
    slots: &HashMap<String, f64>,
    reg_values: &HashMap<String, u32>,
) -> Vec<u8> {
    // Power-gate (`zero_when`): a shut-down sensor is not converting, so it has
    // no measurement to report. Checked FIRST, ahead of every value path, so it
    // holds for composite `fields`, `popcount` and plain sourced registers
    // alike. See `labwired_config::ZeroWhen` for the modelled scope.
    if let Some(z) = &reg.zero_when {
        if reg_values.get(&z.register).copied().unwrap_or(0) & z.mask != 0 {
            return pack(0, reg.width, reg.endian);
        }
    }
    if !reg.fields.is_empty() {
        let mut word = reg.reset;
        for f in &reg.fields {
            let value = slots.get(&f.source).copied().unwrap_or(0.0);
            // Encode into `width_bits` bits (byte-width ceil for the helper), then mask.
            let byte_w = f.width_bits.div_ceil(8);
            let raw = encode_raw(value, f.encode.as_ref(), 1.0, byte_w, f.signed);
            let mask = if f.width_bits >= 32 {
                u32::MAX
            } else {
                (1u32 << f.width_bits) - 1
            };
            word |= (raw & mask) << f.shift;
        }
        return pack(word, reg.width, reg.endian);
    }
    // A rate proportional to how many elements are enabled, not to any external
    // stimulus (see `PopcountSource`): count the set bits of the named registers
    // and scale. Saturates at the register width rather than wrapping, because
    // the quantity is a physical rate, not a bit pattern.
    if let Some(pc) = &reg.popcount {
        let bits: u32 = pc
            .registers
            .iter()
            .map(|n| reg_values.get(n).copied().unwrap_or(0).count_ones())
            .sum();
        let raw = bits
            .saturating_mul(pc.per_bit)
            .min(width_max(reg.width) as u32);
        return pack(raw, reg.width, reg.endian);
    }
    let raw = if let Some(src) = &reg.source {
        let value = slots.get(src).copied().unwrap_or(0.0) * reg.source_scale.unwrap_or(1.0);
        match reg.resolution {
            Some(base) => {
                let resolution = reg
                    .scale_from
                    .iter()
                    .fold(base, |acc, sf| acc * scale_from_one(sf, reg_values));
                divide_raw(value, resolution, reg.width)
            }
            None => encode_raw(
                value,
                reg.encode.as_ref(),
                scale_from_product(reg, reg_values),
                reg.width,
                reg.signed,
            ),
        }
    } else {
        reg_values.get(&reg.name).copied().unwrap_or(reg.reset)
    };
    pack(raw, reg.width, reg.endian)
}

/// Read a named observable channel from a byte-addressable register file:
/// compose the 12-bit raw value (`((regs[base+hi_rel] & hi_mask) << 8) |
/// regs[base+lo_rel]`) and apply the optional linear map (`raw × scale + offset`,
/// clamped). Returns `None` when the channel is out of range, a composing byte
/// is out of bounds, or (with `none_when_raw_zero`) the raw value is 0. This is
/// the shared home for the register→engineering-units math; the I²C
/// `register_file` engine is its only caller (declarative_spi is unaffected).
pub(crate) fn observe(regs: &[u8], obs: &ObservableSpec, channel: u8) -> Option<f64> {
    if channel >= obs.channels {
        return None;
    }
    let base = obs.base as usize + obs.stride as usize * channel as usize;
    let lo = *regs.get(base + obs.value.u12_compose.lo_rel as usize)?;
    let hi =
        *regs.get(base + obs.value.u12_compose.hi_rel as usize)? & obs.value.u12_compose.hi_mask;
    let raw = ((hi as u16) << 8) | lo as u16;
    match &obs.map {
        Some(map) => {
            if map.none_when_raw_zero && raw == 0 {
                return None;
            }
            let mut eng = raw as f64 * map.linear.scale + map.linear.offset;
            if let Some((lo_c, hi_c)) = map.linear.clamp {
                eng = eng.clamp(lo_c, hi_c);
            }
            Some(eng)
        }
        None => Some(raw as f64),
    }
}

// ─── Tier 1: register side effects ─────────────────────────────────────────
//
// ONE vocabulary for what a datasheet says a read or a write DOES, shared by
// the I²C and SPI engines so a part expresses the same silicon the same way on
// either bus. The enums themselves are the MCU register machine's
// (`labwired_config::ReadAction` / `WriteAction`).

/// The word a register stores after the master writes `written` over `prev`.
///
/// [`RegisterSpec::write_mask`] decides WHICH bits the master may touch;
/// [`RegisterSpec::on_write`] decides what touching them does. The two compose:
/// a write-1-to-clear register with a mask clears only masked bits, and bits
/// outside the mask are never disturbed by any action.
pub(crate) fn apply_write(reg: &RegisterSpec, prev: u32, written: u32) -> u32 {
    apply_write_masked(
        reg.on_write.unwrap_or(WriteAction::None),
        prev,
        written,
        reg.write_mask.unwrap_or(u32::MAX),
    )
}

/// [`apply_write`] with the writable mask supplied by the caller.
///
/// The byte-wise auto-increment path needs this: a burst delivers ONE byte
/// lane at a time, and the action has to apply to that lane alone — a
/// write-1-to-clear byte must not clear bits in the bytes of the word the
/// master has not written. Narrowing the mask to the lane does exactly that,
/// and for a [`WriteAction::None`] register it reduces to the plain
/// merge-the-masked-bits store the engine always did.
pub(crate) fn apply_write_masked(action: WriteAction, prev: u32, written: u32, mask: u32) -> u32 {
    let ones = written & mask;
    match action {
        // Plain store: the masked bits take the written value.
        WriteAction::None => (prev & !mask) | ones,
        // `reg &= !data` — a 1 clears.
        WriteAction::WriteOneToClear => prev & !ones,
        // `reg &= data` — a 0 clears. Only bits the mask exposes can drop.
        WriteAction::WriteZeroToClear => prev & !(!written & mask),
        // `reg |= data` — a 1 sets, a 0 is inert.
        WriteAction::OneToSet => prev | ones,
    }
}

/// Whether a completed READ of this register zeroes it
/// ([`RegisterSpec::on_read`]). See that field for what "completes" means on
/// each wire shape.
pub(crate) fn read_clears(reg: &RegisterSpec) -> bool {
    reg.on_read.unwrap_or(ReadAction::None) == ReadAction::Clear
}

// ─── Tier 1: device timers ─────────────────────────────────────────────────

/// The declared [`DeviceTimer`]s of one device plus their running deadlines.
///
/// Shared by both declarative engines: a timer is a property of the PART, not
/// of the bus it hangs off, so an I²C and an SPI descriptor get bit-identical
/// firing sequences from the same YAML. Empty ⇒ every method returns without
/// touching anything, so a device that declares no timer is unchanged.
pub(crate) struct TimerBank {
    timers: Vec<DeviceTimer>,
    /// Absolute µs at which timer `i` next fires; `None` ⇒ not running.
    deadlines: Vec<Option<u64>>,
}

impl TimerBank {
    /// Arm the `on_reset` timers at power-on; leave `manual` ones idle.
    pub(crate) fn new(timers: &[DeviceTimer]) -> Self {
        let deadlines = timers
            .iter()
            .map(|t| match t.start {
                TimerStart::OnReset => Self::interval(t),
                TimerStart::Manual => None,
            })
            .collect();
        Self {
            timers: timers.to_vec(),
            deadlines,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.timers.is_empty()
    }

    /// The declared delay of a timer: its period, or its one-shot delay.
    /// Validation guarantees exactly one is present and non-zero, so a
    /// descriptor that slipped through with neither simply never runs.
    fn interval(t: &DeviceTimer) -> Option<u64> {
        t.period_us.or(t.after_us).filter(|us| *us > 0)
    }

    /// (Re)start every timer whose `start_on_write` names `register` and whose
    /// mask the written value satisfies. `stored` is the register's value AFTER
    /// the write — level-triggered, exactly like [`labwired_config::DataReady`].
    pub(crate) fn start_on_write(&mut self, register: &str, stored: u32, now: u64) {
        for (i, t) in self.timers.iter().enumerate() {
            let Some(trigger) = &t.start_on_write else {
                continue;
            };
            if trigger.register != register {
                continue;
            }
            // No mask ⇒ "write anything to trigger"; a mask ⇒ the bits must be
            // left set by the write.
            if trigger.mask.is_some_and(|m| stored & m == 0) {
                continue;
            }
            self.deadlines[i] = Self::interval(t).map(|us| now.saturating_add(us));
        }
    }

    /// Every action due at or before `now`, in firing order: ascending
    /// deadline, ties broken by declaration order. A periodic timer that is due
    /// several times over one advance fires once per elapsed period, in order,
    /// so a late service pass sees exactly the samples that accrued while the
    /// CPU was elsewhere.
    pub(crate) fn due(&mut self, now: u64) -> Vec<TimingAction> {
        let mut out = Vec::new();
        if self.timers.is_empty() {
            return out;
        }
        loop {
            let next = self
                .deadlines
                .iter()
                .enumerate()
                .filter_map(|(i, d)| d.filter(|deadline| *deadline <= now).map(|d| (d, i)))
                .min();
            let Some((deadline, i)) = next else { break };
            out.extend(self.timers[i].on_fire.iter().cloned());
            // Reschedule a periodic timer from its DEADLINE, not from `now`, so
            // it does not drift with the service cadence; a one-shot goes idle
            // until something starts it again.
            self.deadlines[i] = self.timers[i]
                .period_us
                .filter(|p| *p > 0)
                .map(|period| deadline.saturating_add(period));
        }
        out
    }
}

/// Apply one timer action to a name-keyed register file. Unknown register
/// names cannot occur — validation rejects them at load — so a miss is a
/// no-op rather than a panic.
pub(crate) fn apply_timing_action(
    action: &TimingAction,
    reg_values: &mut std::collections::HashMap<String, u32>,
) {
    match action {
        TimingAction::SetBits { register, bits } => {
            let v = reg_values.get(register).copied().unwrap_or(0);
            reg_values.insert(register.clone(), v | bits);
        }
        TimingAction::ClearBits { register, bits } => {
            let v = reg_values.get(register).copied().unwrap_or(0);
            reg_values.insert(register.clone(), v & !bits);
        }
        TimingAction::WriteValue { register, value } => {
            reg_values.insert(register.clone(), *value);
        }
    }
}

/// Validate the [`DeviceTimer`] list of a descriptor against its register map.
/// Shared by both engines' `validate_descriptor`.
pub(crate) fn validate_timers(
    timers: &[DeviceTimer],
    register_names: &[String],
) -> anyhow::Result<()> {
    let known = |name: &String| register_names.iter().any(|r| r == name);
    for t in timers {
        match (t.period_us, t.after_us) {
            (Some(_), Some(_)) => anyhow::bail!(
                "timer '{}' declares both period_us and after_us — a timer is one or the other",
                t.name
            ),
            (None, None) => anyhow::bail!(
                "timer '{}' declares neither period_us nor after_us, so it could never fire",
                t.name
            ),
            (Some(0), _) | (_, Some(0)) => anyhow::bail!(
                "timer '{}' has a zero interval, which would fire without bound",
                t.name
            ),
            _ => {}
        }
        if t.on_fire.is_empty() {
            anyhow::bail!(
                "timer '{}' has no on_fire actions, so it is dead weight",
                t.name
            );
        }
        for action in &t.on_fire {
            let register = match action {
                TimingAction::SetBits { register, .. }
                | TimingAction::ClearBits { register, .. }
                | TimingAction::WriteValue { register, .. } => register,
            };
            if !known(register) {
                anyhow::bail!(
                    "timer '{}' fires at '{register}', which is not a declared register",
                    t.name
                );
            }
        }
        if let Some(trigger) = &t.start_on_write {
            if !known(&trigger.register) {
                anyhow::bail!(
                    "timer '{}' starts on a write to '{}', which is not a declared register",
                    t.name,
                    trigger.register
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use labwired_config::{Endian, RegisterAccess, RegisterSpec};
    use std::collections::HashMap;

    fn reg(name: &str, addr: u16, width: u8, endian: Endian, source: Option<&str>) -> RegisterSpec {
        RegisterSpec {
            name: name.into(),
            addr,
            width,
            endian,
            access: RegisterAccess::R,
            write_mask: None,
            reset: 0,
            source: source.map(Into::into),
            encode: None,
            scale_from: vec![],
            source_scale: None,
            resolution: None,
            signed: false,
            fields: vec![],
            page: None,
            self_clearing: None,
            popcount: None,
            zero_when: None,
            on_read: None,
            on_write: None,
        }
    }

    #[test]
    fn signed_negative_value_packs_twos_complement_le() {
        use labwired_config::{Endian, RegisterAccess, RegisterSpec};
        use std::collections::HashMap;
        let r = RegisterSpec {
            name: "DATAX".into(),
            addr: 0x32,
            width: 2,
            endian: Endian::Le,
            access: RegisterAccess::R,
            write_mask: None,
            reset: 0,
            source: Some("ax".into()),
            encode: Some(labwired_config::Encode {
                scale: 256.0,
                offset: 0.0,
                clamp_min: None,
                clamp_max: None,
            }),
            scale_from: vec![],
            source_scale: None,
            resolution: None,
            signed: true,
            fields: vec![],
            page: None,
            self_clearing: None,
            popcount: None,
            zero_when: None,
            on_read: None,
            on_write: None,
        };
        let mut slots = HashMap::new();
        slots.insert("ax".to_string(), -1.0); // -1 g × 256 = -256 = 0xFF00 two's-complement, LE
        assert_eq!(
            register_read_bytes(&r, &slots, &HashMap::new()),
            vec![0x00, 0xFF]
        );
    }

    #[test]
    fn write_actions_follow_the_systemrdl_definitions() {
        use labwired_config::WriteAction::*;
        // prev, written, mask, action → stored
        let cases = [
            (0xF0u32, 0x30u32, u32::MAX, None, 0x30u32),
            (0xF0, 0x30, u32::MAX, WriteOneToClear, 0xC0),
            (0xFF, 0x0F, u32::MAX, WriteZeroToClear, 0x0F),
            (0x01, 0x80, u32::MAX, OneToSet, 0x81),
            // The mask narrows every action to the bits firmware owns.
            (0xFF, 0xFF, 0x0F, WriteOneToClear, 0xF0),
            (0xFF, 0x00, 0x0F, WriteZeroToClear, 0xF0),
            (0x00, 0xFF, 0x0F, OneToSet, 0x0F),
            (0xF0, 0x0F, 0x0F, None, 0xFF),
        ];
        for (prev, written, mask, action, want) in cases {
            assert_eq!(
                apply_write_masked(action, prev, written, mask),
                want,
                "{action:?} prev={prev:#x} written={written:#x} mask={mask:#x}"
            );
        }
    }

    #[test]
    fn a_write_action_with_no_key_is_a_plain_store() {
        // The default every descriptor written before `on_write` existed means.
        let r = reg("CTRL", 0x00, 1, Endian::Le, None);
        assert_eq!(apply_write(&r, 0xF0, 0x0F), 0x0F);
    }

    #[test]
    fn timers_fire_in_deadline_order_with_declaration_order_as_the_tiebreak() {
        use labwired_config::{DeviceTimer, TimerStart, TimingAction};
        let timer = |name: &str, period: u64, bits: u32| DeviceTimer {
            name: name.into(),
            period_us: Some(period),
            after_us: Option::None,
            start: TimerStart::OnReset,
            start_on_write: Option::None,
            on_fire: vec![TimingAction::SetBits {
                register: name.into(),
                bits,
            }],
        };
        // `slow` is declared FIRST but is due later; `fast` fires twice inside
        // the same advance. Deadline order decides, declaration order breaks
        // the tie at 20 µs.
        let mut bank = TimerBank::new(&[timer("slow", 20, 1), timer("fast", 10, 2)]);
        let fired: Vec<String> = bank
            .due(25)
            .into_iter()
            .map(|a| match a {
                TimingAction::SetBits { register, .. } => register,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(fired, vec!["fast", "slow", "fast"]);
    }

    #[test]
    fn a_periodic_timer_does_not_drift_with_the_service_cadence() {
        use labwired_config::{DeviceTimer, TimerStart, TimingAction};
        let mut bank = TimerBank::new(&[DeviceTimer {
            name: "s".into(),
            period_us: Some(10),
            after_us: Option::None,
            start: TimerStart::OnReset,
            start_on_write: Option::None,
            on_fire: vec![TimingAction::SetBits {
                register: "S".into(),
                bits: 1,
            }],
        }]);
        // Serviced late at 15 µs, then again at 21: the second period is due at
        // 20, not at 25 (which is what rescheduling from `now` would give).
        assert_eq!(bank.due(15).len(), 1);
        assert_eq!(bank.due(21).len(), 1);
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

    #[test]
    fn composite_fields_assemble_into_word() {
        use labwired_config::{Encode, Endian, FieldSpec, RegisterAccess, RegisterSpec};
        use std::collections::HashMap;
        // 32-bit BE frame: thermocouple °C at bits[31:18] signed 14-bit, 0.25°C/LSB
        // (scale 4.0); internal °C at bits[15:4] signed 12-bit, 0.0625°C/LSB (16.0).
        let r = RegisterSpec {
            name: "OUT".into(),
            addr: 0,
            width: 4,
            endian: Endian::Be,
            access: RegisterAccess::R,
            write_mask: None,
            reset: 0,
            source: None,
            encode: None,
            scale_from: vec![],
            source_scale: None,
            resolution: None,
            signed: false,
            fields: vec![
                FieldSpec {
                    source: "tc".into(),
                    shift: 18,
                    width_bits: 14,
                    signed: true,
                    encode: Some(Encode {
                        scale: 4.0,
                        offset: 0.0,
                        clamp_min: None,
                        clamp_max: None,
                    }),
                },
                FieldSpec {
                    source: "internal".into(),
                    shift: 4,
                    width_bits: 12,
                    signed: true,
                    encode: Some(Encode {
                        scale: 16.0,
                        offset: 0.0,
                        clamp_min: None,
                        clamp_max: None,
                    }),
                },
            ],
            page: None,
            self_clearing: None,
            popcount: None,
            zero_when: None,
            on_read: None,
            on_write: None,
        };
        let mut slots = HashMap::new();
        slots.insert("tc".to_string(), 100.0); // 100°C → 400 = 0x190 in bits[31:18]
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
            name: "OUT".into(),
            addr: 0,
            width: 4,
            endian: Endian::Be,
            access: RegisterAccess::R,
            write_mask: None,
            reset: 0,
            source: None,
            encode: None,
            scale_from: vec![],
            source_scale: None,
            resolution: None,
            signed: false,
            fields: vec![FieldSpec {
                source: "tc".into(),
                shift: 18,
                width_bits: 14,
                signed: true,
                encode: Some(Encode {
                    scale: 4.0,
                    offset: 0.0,
                    clamp_min: None,
                    clamp_max: None,
                }),
            }],
            page: None,
            self_clearing: None,
            popcount: None,
            zero_when: None,
            on_read: None,
            on_write: None,
        };
        let mut slots = HashMap::new();
        slots.insert("tc".to_string(), -25.0); // -25°C → -100 → 14-bit two's-comp = 0x3F9C, <<18
        let b = register_read_bytes(&r, &slots, &HashMap::new());
        let word = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        assert_eq!((word >> 18) & 0x3FFF, 0x3F9C);
    }
}
