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
    CalendarField, DeviceTimer, Encode, Endian, LabDescriptor, ObservableSpec, ReadAction,
    RegisterSpec, Rounding, TimerStart, TimingAction, WriteAction,
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

/// Pack an integer into binary-coded decimal: two decimal digits per byte,
/// tens in the high nibble. A value with more digits than `width` bytes hold
/// saturates at all-nines rather than spilling into a neighbouring field — a
/// counter chain runs out of digits, it does not carry into the next register.
/// Negative values are not representable and clamp to 0.
pub(crate) fn to_bcd(value: i64, width: u8) -> u32 {
    let digits = 2 * u32::from(width).min(4);
    let max = 10i64.pow(digits) - 1;
    let mut v = value.clamp(0, max);
    let mut out: u32 = 0;
    for d in 0..digits {
        out |= ((v % 10) as u32) << (4 * d);
        v /= 10;
    }
    out
}

/// Unpack binary-coded decimal into an integer. A nibble above 9 is not a
/// decimal digit; it is decoded the way the silicon's counter chain reads it
/// (`digit * 10^k` with the raw nibble as the digit), so `0x1A` is 20 rather
/// than a load error — the master put it on the wire and the part has to
/// answer.
pub(crate) fn from_bcd(word: u32, width: u8) -> i64 {
    let digits = 2 * u32::from(width).min(4);
    let mut out: i64 = 0;
    for d in (0..digits).rev() {
        out = out * 10 + i64::from((word >> (4 * d)) & 0xF);
    }
    out
}

/// The saturation window in raw counts: the constant `clamp_min`/`clamp_max`
/// pair, INTERSECTED with every `clamp_from` entry whose register field is
/// present in its map. An unmapped field value is neutral, exactly as an
/// unmapped `scale_from` factor is 1.0.
pub(crate) fn resolve_clamp(
    enc: Option<&Encode>,
    reg_values: &HashMap<String, u32>,
) -> (Option<f64>, Option<f64>) {
    let Some(e) = enc else {
        return (None, None);
    };
    let (mut lo, mut hi) = (e.clamp_min, e.clamp_max);
    for cf in &e.clamp_from {
        let regval = reg_values.get(&cf.register).copied().unwrap_or(0);
        let field = (regval >> cf.shift as u32) & cf.mask;
        let Some(w) = cf.map.get(&field) else {
            continue;
        };
        lo = Some(lo.map_or(w.min, |c: f64| c.max(w.min)));
        hi = Some(hi.map_or(w.max, |c: f64| c.min(w.max)));
    }
    (lo, hi)
}

/// Apply a linear encode (scale/offset/clamp) plus an extra scale factor,
/// yielding the raw integer packed into a `width`-byte word. The clamp window
/// is the register's constant one; see [`encode_raw_clamped`] for the
/// field-driven form.
pub(crate) fn encode_raw(
    value: f64,
    enc: Option<&Encode>,
    extra_scale: f64,
    width: u8,
    signed: bool,
) -> u32 {
    let clamp = (enc.and_then(|e| e.clamp_min), enc.and_then(|e| e.clamp_max));
    encode_raw_bits(value, enc, extra_scale, 8 * u32::from(width), signed, clamp)
}

/// [`encode_raw`] with the destination width given in BITS and the saturation
/// window supplied by the caller.
///
/// A bit width rather than a byte width because a [`labwired_config::FieldSpec`]
/// is not byte-sized: the MMA8451Q's 14-bit left-justified output saturates at
/// ±8192 counts (its own width), and rounding it into the 16 bits its
/// byte-ceiling would give lets a 3 g reading at the ±2 g full scale land as
/// 12288 counts, which the field mask then truncates into a NEGATIVE
/// acceleration. Saturating at the field's real width is what a converter does
/// at the end of its range.
///
/// The window is a parameter rather than being read off `enc` because
/// `encode.clamp_from` resolves it against the LIVE register file, which this
/// function cannot see. [`resolve_clamp`] is what computes it; [`encode_raw`]
/// passes the constant pair for a caller that has no register file to hand.
pub(crate) fn encode_raw_bits(
    value: f64,
    enc: Option<&Encode>,
    extra_scale: f64,
    bits: u32,
    signed: bool,
    clamp: (Option<f64>, Option<f64>),
) -> u32 {
    let scale = enc.map(|e| e.scale).unwrap_or(1.0) * extra_scale;
    let offset = enc.map(|e| e.offset).unwrap_or(0.0);
    let mut raw = value * scale + offset;
    if let Some(lo) = clamp.0 {
        raw = raw.max(lo);
    }
    if let Some(hi) = clamp.1 {
        raw = raw.min(hi);
    }
    // `round:` picks how the value becomes a count. Nearest is the default and
    // is what every descriptor written before the key existed means; `floor` is
    // the one a counter-field decomposition needs.
    let round = |v: f64| match enc.and_then(|e| e.round).unwrap_or_default() {
        Rounding::Nearest => v.round(),
        Rounding::Floor => v.floor(),
        Rounding::Ceil => v.ceil(),
        Rounding::Trunc => v.trunc(),
    };
    // BCD is the LAST step: the count is computed in decimal exactly as it is
    // for any other register and only then packed into nibbles, so `wrap`,
    // `clamp` and the signedness rules below all mean what they say. Packed in
    // whole BYTES, because a nibble pair is what BCD IS — a sub-byte FIELD is
    // never BCD, so the bit width rounds up here and nowhere else.
    let bcd = enc.map(|e| e.bcd).unwrap_or(false);
    let bcd_width = (bits / 8).max(1) as u8;
    let mask = if bits >= 32 {
        u32::MAX
    } else {
        (1u32 << bits) - 1
    };
    // `wrap`: the register is a modular counter of N raw counts, so the count
    // rolls over instead of saturating. Rounded to an integer FIRST — wrapping
    // 4095.99 as a float and rounding afterwards would produce 4096, a count a
    // 12-bit counter cannot hold. `rem_euclid` so a negative measurement lands
    // on the count the counter would really be showing rather than on the
    // clamp. See `labwired_config::Encode::wrap` for why this is in counts.
    if let Some(w) = enc.and_then(|e| e.wrap) {
        let v = (round(raw) as i64).rem_euclid(i64::from(w.get()));
        return if bcd {
            to_bcd(v, bcd_width) & mask
        } else {
            (v as u32) & mask
        };
    }
    if bcd {
        return to_bcd(round(raw) as i64, bcd_width) & mask;
    }
    if signed {
        let lo = -(2f64.powi((bits - 1) as i32));
        let hi = 2f64.powi((bits - 1) as i32) - 1.0;
        let v = round(raw).clamp(lo, hi) as i64;
        (v as u32) & mask
    } else {
        round(raw).clamp(0.0, f64::from(mask)) as u32
    }
}

/// The inverse of [`encode_raw`]'s LINEAR half: a raw count back to the
/// engineering value that would encode to it.
///
/// This is what makes `set_input:` the exact inverse of `input()` on a
/// register device — a rule that writes back what it read changes nothing.
///
/// ⚠️ Only the linear half inverts. `clamp`, `wrap` and `round` are one-way by
/// construction (they throw information away on purpose), and they are applied
/// again on the very next read, so the round trip a rule can observe is
/// exactly the one this function provides.
pub(crate) fn decode_raw(count: i64, enc: Option<&Encode>, extra_scale: f64, width: u8) -> f64 {
    let decoded = if enc.map(|e| e.bcd).unwrap_or(false) {
        from_bcd(count as u32, width)
    } else {
        count
    };
    let scale = enc.map(|e| e.scale).unwrap_or(1.0) * extra_scale;
    let offset = enc.map(|e| e.offset).unwrap_or(0.0);
    // A zero scale is a degenerate descriptor; 0.0 keeps this total, the same
    // rule the expression evaluator's divide-by-zero has.
    if scale == 0.0 {
        return 0.0;
    }
    (decoded as f64 - offset) / scale
}

/// The write dual of the `bcd` encode: the word the master put on the wire,
/// decoded to the integer the model stores. A register that is not BCD stores
/// what was written, unchanged.
pub(crate) fn decode_write(reg: &RegisterSpec, written: u32) -> u32 {
    match reg.encode.as_ref() {
        Some(e) if e.bcd => {
            // `value_mask:` — the masked bits are the number and the rest are
            // flags the master wrote verbatim. The clamp applies to the NUMBER
            // only; a flag is not in range of anything.
            let mask = e.value_mask.unwrap_or(u32::MAX);
            let mut v = from_bcd(written & mask, reg.width);
            if let Some(lo) = e.clamp_min {
                v = v.max(lo as i64);
            }
            if let Some(hi) = e.clamp_max {
                v = v.min(hi as i64);
            }
            (v as u32 & mask) | (written & !mask)
        }
        _ => written,
    }
}

/// True when the word this register STORES is not the word the master put on
/// the wire: a `bcd:` register stores decimal, and a `calendar:` register's
/// write lands on the sourced clock channel. Both need the translated write
/// path rather than the plain mask-and-store one.
pub(crate) fn write_is_translated(reg: &RegisterSpec) -> bool {
    reg.calendar.is_some() || reg.encode.as_ref().is_some_and(|e| e.bcd)
}

/// A civil instant, UTC, with no leap seconds: the seven fields a DS3231-class
/// RTC holds. `weekday` is 1..=7 with Sunday = 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Civil {
    pub year: i64,
    pub month: i64,
    pub day: i64,
    pub hour: i64,
    pub minute: i64,
    pub second: i64,
    pub weekday: i64,
}

/// Unix seconds → civil fields. Howard Hinnant's `civil_from_days`, which is
/// the algorithm the hand-written DS3231 model carried, so the port reproduces
/// its transcript byte for byte.
pub(crate) fn civil_from_unix(unix: i64) -> Civil {
    let days = unix.div_euclid(86_400);
    let mut rem = unix.rem_euclid(86_400);
    let hour = rem / 3600;
    rem %= 3600;
    let minute = rem / 60;
    let second = rem % 60;
    // 1970-01-01 was a Thursday; Sunday = 1 makes Thursday 5.
    let weekday = (days + 4).rem_euclid(7) + 1;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as i64;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as i64;
    let year = if month <= 2 { y + 1 } else { y };
    Civil {
        year,
        month,
        day,
        hour,
        minute,
        second,
        weekday,
    }
}

/// Civil fields → Unix seconds. Hinnant's `days_from_civil`, the exact inverse
/// of [`civil_from_unix`]; `weekday` is ignored because the date already
/// determines it.
pub(crate) fn unix_from_civil(c: Civil) -> i64 {
    let y = if c.month <= 2 { c.year - 1 } else { c.year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let m = c.month as u64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + c.day as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe as i64 - 719_468;
    days * 86_400 + c.hour * 3600 + c.minute * 60 + c.second
}

/// Read one calendar field out of a civil instant.
pub(crate) fn calendar_get(c: Civil, f: CalendarField) -> i64 {
    match f {
        CalendarField::Second => c.second,
        CalendarField::Minute => c.minute,
        CalendarField::Hour => c.hour,
        CalendarField::Weekday => c.weekday,
        CalendarField::Day => c.day,
        CalendarField::Month => c.month,
        // The two-digit year an RTC register holds.
        CalendarField::Year => c.year.rem_euclid(100),
    }
}

/// Replace one calendar field of a civil instant, clamped to the range the
/// field can hold so a nonsense write cannot roll the whole clock somewhere
/// else. `Year` sets the two-digit year within the century the instant is
/// already in, which is what an RTC that holds two digits can express.
pub(crate) fn calendar_set(c: &mut Civil, f: CalendarField, v: i64) {
    match f {
        CalendarField::Second => c.second = v.clamp(0, 59),
        CalendarField::Minute => c.minute = v.clamp(0, 59),
        CalendarField::Hour => c.hour = v.clamp(0, 23),
        // The weekday counter is independent silicon on a DS3231 — it is a
        // 1..=7 counter the master sets, not a function of the date — but the
        // model derives it from the date, so a write to it is accepted and
        // does not move the instant. Stated in `docs/part-packs.md`.
        CalendarField::Weekday => c.weekday = v.clamp(1, 7),
        CalendarField::Day => c.day = v.clamp(1, 31),
        CalendarField::Month => c.month = v.clamp(1, 12),
        CalendarField::Year => {
            let century = c.year.div_euclid(100) * 100;
            c.year = century + v.clamp(0, 99);
        }
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

/// Product of a `scale_from` list, folded left-to-right from 1.0. Shared by a
/// register's own list and by a [`labwired_config::FieldSpec`]'s.
pub(crate) fn scale_from_product_of(
    list: &[labwired_config::ScaleFrom],
    reg_values: &HashMap<String, u32>,
) -> f64 {
    list.iter()
        .fold(1.0, |acc, sf| acc * scale_from_one(sf, reg_values))
}

/// Product of a register's `scale_from` factors, folded left-to-right from 1.0.
pub(crate) fn scale_from_product(reg: &RegisterSpec, reg_values: &HashMap<String, u32>) -> f64 {
    scale_from_product_of(&reg.scale_from, reg_values)
}

/// The measurement channel a register reports: its [`RegisterSpec::source_from`]
/// multiplexer's current selection, or its plain `source`. `None` ⇒ the register
/// is storage (or a `popcount` / `fields` composite).
///
/// The mux falls back to the declared `source` for a field value the table does
/// not cover, so a partial table says what it does not model instead of silently
/// reading 0.
pub(crate) fn selected_source<'a>(
    reg: &'a RegisterSpec,
    reg_values: &HashMap<String, u32>,
) -> Option<&'a str> {
    if let Some(sf) = &reg.source_from {
        let regval = reg_values.get(&sf.register).copied().unwrap_or(0);
        let field = (regval >> sf.shift as u32) & sf.mask;
        if let Some(key) = sf.table.get(&field) {
            return Some(key.as_str());
        }
    }
    reg.source.as_deref()
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
    // The same gate, the other polarity (`zero_unless`): the part is asleep
    // until firmware SETS the enable bit. One branch, one struct, the polarity
    // in the key name — see `labwired_config::RegisterSpec::zero_unless`.
    if let Some(z) = &reg.zero_unless {
        if reg_values.get(&z.register).copied().unwrap_or(0) & z.mask == 0 {
            return pack(0, reg.width, reg.endian);
        }
    }
    if !reg.fields.is_empty() {
        let mut word = reg.reset;
        for f in &reg.fields {
            let value = slots.get(&f.source).copied().unwrap_or(0.0);
            // Encoded at the field's OWN bit width — rounded and saturated to
            // `width_bits` BEFORE `shift` places it, which is what makes a
            // left-justified output register's low bits always zero on the wire
            // and what stops an over-range measurement wrapping sign. The
            // per-field `scale_from` compounds in exactly as a register's does,
            // so a full-scale select bit-field reaches a packed field.
            let extra = scale_from_product_of(&f.scale_from, reg_values);
            let raw = encode_raw_bits(
                value,
                f.encode.as_ref(),
                extra,
                u32::from(f.width_bits),
                f.signed,
                // A composite field resolves its own `clamp_from` against the
                // same register file the whole word does.
                resolve_clamp(f.encode.as_ref(), reg_values),
            );
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
    let raw = if let Some(src) = selected_source(reg, reg_values) {
        let mut value = slots.get(src).copied().unwrap_or(0.0);
        // `calendar:` — the sourced channel carries Unix seconds and this
        // register reports ONE civil field of that instant. Decomposed before
        // any scaling, so `encode:` on such a register (in practice `bcd:`)
        // still means what it means everywhere else.
        if let Some(f) = reg.calendar {
            value = calendar_get(civil_from_unix(value as i64), f) as f64;
        }
        let value = value * reg.source_scale.unwrap_or(1.0);
        match reg.resolution {
            Some(base) => {
                let resolution = reg
                    .scale_from
                    .iter()
                    .fold(base, |acc, sf| acc * scale_from_one(sf, reg_values));
                divide_raw(value, resolution, reg.width)
            }
            None => encode_raw_bits(
                value,
                reg.encode.as_ref(),
                scale_from_product(reg, reg_values),
                8 * u32::from(reg.width),
                reg.signed,
                resolve_clamp(reg.encode.as_ref(), reg_values),
            ),
        }
    } else {
        let stored = reg_values.get(&reg.name).copied().unwrap_or(reg.reset);
        // A BCD storage register holds its value in DECIMAL (so `reg()` and
        // every guard reading it are in decimal) and puts nibbles on the wire.
        //
        // `value_mask:` splits the word: the masked bits are the number and
        // everything else is a plain flag, served back exactly as written. That
        // is the DS3231 alarm-byte shape — a BCD number and its A1Mx mask bit
        // in one byte — and without it one of the two has to be thrown away.
        match reg.encode.as_ref() {
            Some(e) if e.bcd => {
                let mask = e.value_mask.unwrap_or(u32::MAX);
                let number = to_bcd(i64::from(stored & mask), reg.width);
                (number & mask) | (stored & !mask)
            }
            _ => stored,
        }
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

/// Resolve `(register, field)` to that field's `(shift, mask)` — the one
/// lookup [`TimerBank::apply_period_from`] needs from the owning device, named
/// so the signature reads as what it is.
pub(crate) type FieldBitsFn<'a> = dyn Fn(&str, &str) -> Option<(u8, u32)> + 'a;

/// The declared [`DeviceTimer`]s of one device plus their running deadlines.
///
/// Shared by both declarative engines: a timer is a property of the PART, not
/// of the bus it hangs off, so an I²C and an SPI descriptor get bit-identical
/// firing sequences from the same YAML. Empty ⇒ every method returns without
/// touching anything, so a device that declares no timer is unchanged.
#[derive(Debug)]
pub(crate) struct TimerBank {
    timers: Vec<DeviceTimer>,
    /// Absolute µs at which timer `i` next fires; `None` ⇒ not running.
    deadlines: Vec<Option<u64>>,
    /// Timer `i`'s EFFECTIVE interval in µs — the declared `period_us` /
    /// `after_us`, or whatever [`labwired_config::TimerPeriodFrom`] last
    /// resolved to. Cached rather than recomputed per firing so the hot path
    /// (`due_by_timer`, called on every time advance of every declarative
    /// device) never touches the register file; the owning device refreshes it
    /// through [`apply_period_from`](Self::apply_period_from) after a write.
    periods: Vec<Option<u64>>,
}

impl TimerBank {
    /// Arm the `on_reset` timers at power-on; leave `manual` ones idle.
    pub(crate) fn new(timers: &[DeviceTimer]) -> Self {
        let periods: Vec<Option<u64>> = timers.iter().map(Self::declared_interval).collect();
        let deadlines = timers
            .iter()
            .zip(periods.iter())
            .map(|(t, p)| match t.start {
                TimerStart::OnReset => *p,
                TimerStart::Manual => None,
            })
            .collect();
        Self {
            timers: timers.to_vec(),
            deadlines,
            periods,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.timers.is_empty()
    }

    /// The DECLARED delay of a timer: its period, or its one-shot delay.
    /// Validation guarantees exactly one is present and non-zero, so a
    /// descriptor that slipped through with neither simply never runs.
    ///
    /// This is the reset-value interval. A timer with a
    /// [`TimerPeriodFrom`](labwired_config::TimerPeriodFrom) overrides it from
    /// the register file; see [`apply_period_from`](Self::apply_period_from).
    fn declared_interval(t: &DeviceTimer) -> Option<u64> {
        t.period_us.or(t.after_us).filter(|us| *us > 0)
    }

    /// Timer `i`'s effective interval — the cached one, which is the declared
    /// one until a `period_from` resolves to something else.
    fn interval(&self, i: usize) -> Option<u64> {
        self.periods[i]
    }

    /// Re-resolve every `period_from` timer against the register file.
    ///
    /// The owning device calls this wherever a register write lands (and once
    /// after attach), because a rate register is exactly the thing firmware
    /// writes. `reg` reads a register's stored word by name and `field_bits`
    /// resolves a named `bits:` field to `(shift, mask)` — the same two lookups
    /// [`RuleCtx`](super::rule_machine::RuleCtx) offers, so the field a rule
    /// reads and the field a timer reads cannot disagree.
    ///
    /// ⚠️ A RUNNING periodic timer whose period changed is re-anchored to
    /// `now + the new period`, not rescheduled from its old deadline: firmware
    /// that rewrote the rate register restarted the divider, and keeping the
    /// old anchor would make the first interval after the write a length
    /// neither setting has.
    ///
    /// An unmapped field value is NEUTRAL — the declared `period_us` stays in
    /// force — the same rule `scale_from` and `clamp_from` have, so a reserved
    /// encoding cannot silently stop the part's clock.
    pub(crate) fn apply_period_from(
        &mut self,
        now: u64,
        reg: &dyn Fn(&str) -> Option<u32>,
        field_bits: &FieldBitsFn<'_>,
    ) {
        for i in 0..self.timers.len() {
            let Some(spec) = self.timers[i].period_from.clone() else {
                continue;
            };
            let Some(word) = reg(&spec.register) else {
                continue;
            };
            let (shift, mask) = match (&spec.field, spec.mask) {
                (Some(f), _) => match field_bits(&spec.register, f) {
                    Some(sm) => sm,
                    None => continue,
                },
                (None, Some(m)) => (spec.shift, m << spec.shift),
                (None, None) => continue,
            };
            let value = (word & mask) >> shift;
            let resolved = match spec.table.get(&value) {
                Some(us) if *us > 0 => *us,
                // Unmapped (or a zero the validator let through): neutral.
                _ => match Self::declared_interval(&self.timers[i]) {
                    Some(us) => us,
                    None => continue,
                },
            };
            if self.periods[i] == Some(resolved) {
                continue;
            }
            self.periods[i] = Some(resolved);
            if self.deadlines[i].is_some() {
                self.deadlines[i] = Some(now.saturating_add(resolved));
            }
        }
    }

    /// Whether any timer's period is field-driven, so a device can skip the
    /// refresh entirely — which is every descriptor written before the key.
    pub(crate) fn has_field_driven_period(&self) -> bool {
        self.timers.iter().any(|t| t.period_from.is_some())
    }

    /// Timer `name`'s effective period in µs — diagnostics and tests, so a
    /// test can assert the RESOLVED rate rather than inferring it from firing
    /// counts.
    pub(crate) fn period_us_of(&self, name: &str) -> Option<u64> {
        self.timers
            .iter()
            .position(|t| t.name == name)
            .and_then(|i| self.periods[i])
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
            self.deadlines[i] = self.periods[i].map(|us| now.saturating_add(us));
        }
    }

    /// Every action due at or before `now`, in firing order: ascending
    /// deadline, ties broken by declaration order. A periodic timer that is due
    /// several times over one advance fires once per elapsed period, in order,
    /// so a late service pass sees exactly the samples that accrued while the
    /// CPU was elsewhere.
    /// Start (or restart) a timer by NAME, from `now`. The Tier-2 `timer:`
    /// action goes through here, so a rule and a `start_on_write` arm the same
    /// deadline list rather than two.
    pub(crate) fn start_named(&mut self, name: &str, now: u64) {
        for i in 0..self.timers.len() {
            if self.timers[i].name == name {
                self.deadlines[i] = self.interval(i).map(|us| now.saturating_add(us));
            }
        }
    }

    /// Stop a timer by NAME. A one-shot that has already fired is idle anyway;
    /// this is what lets a rule silence a periodic one — a part going to sleep.
    pub(crate) fn stop_named(&mut self, name: &str) {
        for (i, t) in self.timers.iter().enumerate() {
            if t.name == name {
                self.deadlines[i] = None;
            }
        }
    }

    /// Every firing due at or before `now`, kept per TIMER and tagged with its
    /// name: ascending deadline, ties broken by declaration order, one entry
    /// per elapsed period. A periodic timer that is due several times over one
    /// advance fires once per elapsed period, in order, so a late service pass
    /// sees exactly the samples that accrued while the CPU was elsewhere.
    ///
    /// The name and the per-firing boundary are what a Tier-2 rule needs
    /// (`on: { timer: NAME }`), and it must come from the SAME traversal that
    /// runs `on_fire` — not a second clock the rule machine keeps alongside
    /// this one. Two clocks is how a rule and an `on_fire` come to disagree
    /// about when a part ticked; there is exactly one here.
    ///
    /// ⚠️ A periodic timer whose period is much shorter than the advance is due
    /// many times, and a very long jump could otherwise spin here; the walk is
    /// capped at [`MAX_TIMER_CATCHUP`] firings and then re-anchors every still-
    /// due timer past `now`. The samples beyond the cap are lost, which is what
    /// a real FIFO reports after the CPU was away too long — and it is a bound,
    /// not a hang inside a bus tick.
    pub(crate) fn due_by_timer(&mut self, now: u64) -> Vec<(String, Vec<TimingAction>)> {
        let mut out = Vec::new();
        if self.timers.is_empty() {
            return out;
        }
        let mut fired = 0u32;
        loop {
            let next = self
                .deadlines
                .iter()
                .enumerate()
                .filter_map(|(i, d)| d.filter(|deadline| *deadline <= now).map(|d| (d, i)))
                .min();
            let Some((deadline, i)) = next else { break };
            out.push((self.timers[i].name.clone(), self.timers[i].on_fire.clone()));
            // Reschedule a periodic timer from its DEADLINE, not from `now`, so
            // it does not drift with the service cadence; a one-shot goes idle
            // until something starts it again.
            // Periodic timers reschedule from their own EFFECTIVE period,
            // which `period_from` may have changed since the last firing; a
            // one-shot (`after_us`, no `period_us`) goes idle.
            self.deadlines[i] = self.timers[i]
                .period_us
                .and(self.periods[i])
                .filter(|p| *p > 0)
                .map(|period| deadline.saturating_add(period));
            fired += 1;
            if fired >= MAX_TIMER_CATCHUP {
                for d in self.deadlines.iter_mut() {
                    if d.is_some_and(|deadline| deadline <= now) {
                        *d = Some(now.saturating_add(1));
                    }
                }
                break;
            }
        }
        out
    }
}

/// How many timer firings one time advance may replay before the bank gives up
/// and re-anchors.
///
/// A device that was not serviced for a long simulated stretch genuinely owes
/// many periods — that is the CPU-starvation case a FIFO overflow exists to
/// show. But an unbounded walk turns a 1 µs period plus a 10 s jump into ten
/// million iterations inside one bus tick, which is a hang, not fidelity.
const MAX_TIMER_CATCHUP: u32 = 4096;

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
    rules: &[labwired_config::Rule],
) -> anyhow::Result<()> {
    // A timer earns its place either by writing registers (`on_fire:`) or by
    // being something a Tier-2 rule listens for. Before Tier 2 there was only
    // the first, so "no on_fire" meant dead weight; now a sample clock whose
    // whole job is to raise `on: { timer: sample }` is a legitimate — and the
    // most common — shape, and refusing it would make the MPU6050's INT line
    // unexpressible.
    let listened_for = |name: &str| {
        rules
            .iter()
            .any(|r| matches!(&r.on, labwired_config::Event::Timer { name: n } if n == name))
    };
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
        if t.on_fire.is_empty() && !listened_for(&t.name) {
            anyhow::bail!(
                "timer '{}' has no on_fire actions and no rule listens for \
                 `on: {{ timer: {} }}`, so it is dead weight",
                t.name,
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
        if let Some(spec) = &t.period_from {
            anyhow::ensure!(
                known(&spec.register),
                "timer '{}' takes its period from '{}', which is not a declared register",
                t.name,
                spec.register
            );
            anyhow::ensure!(
                spec.field.is_some() != spec.mask.is_some(),
                "timer '{}' period_from must name exactly one of `field:` and `mask:`",
                t.name
            );
            anyhow::ensure!(
                !spec.table.is_empty(),
                "timer '{}' period_from has an empty `table:`, so every field value would be \
                 unmapped and the key would change nothing",
                t.name
            );
            for (value, us) in &spec.table {
                anyhow::ensure!(
                    *us > 0,
                    "timer '{}' period_from maps field value {value} to a zero period, which \
                     would fire without bound",
                    t.name
                );
            }
            // A field-driven PERIOD on a one-shot is a contradiction: `after_us`
            // is a delay measured once, and a table of repeating rates has
            // nothing to say about it.
            anyhow::ensure!(
                t.period_us.is_some(),
                "timer '{}' declares period_from but no period_us — period_us is the period the \
                 source register's RESET value gives, and without it the part has no rate before \
                 firmware writes anything",
                t.name
            );
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
            fifo: None,
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
            bits: vec![],
            on_read: None,
            on_write: None,
            calendar: None,
            zero_unless: None,
            source_from: None,
        }
    }

    #[test]
    fn signed_negative_value_packs_twos_complement_le() {
        use labwired_config::{Endian, RegisterAccess, RegisterSpec};
        use std::collections::HashMap;
        let r = RegisterSpec {
            name: "DATAX".into(),
            fifo: None,
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
                wrap: None,
                bcd: false,
                value_mask: None,
                round: None,
                clamp_from: vec![],
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
            bits: vec![],
            on_read: None,
            on_write: None,
            calendar: None,
            zero_unless: None,
            source_from: None,
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
            period_from: None,
            on_fire: vec![TimingAction::SetBits {
                register: name.into(),
                bits,
            }],
        };
        // `slow` is declared FIRST but is due later; `fast` fires twice inside
        // the same advance. Deadline order decides, declaration order breaks
        // the tie at 20 µs.
        let mut bank = TimerBank::new(&[timer("slow", 20, 1), timer("fast", 10, 2)]);
        let firings = bank.due_by_timer(25);
        let by_name: Vec<String> = firings.iter().map(|(name, _)| name.clone()).collect();
        assert_eq!(by_name, vec!["fast", "slow", "fast"]);
        // The register actions come out in the same order, which is what the
        // Tier-1 engine applies — the per-timer grouping is a view of ONE walk,
        // not a second one that could order differently.
        let fired: Vec<String> = firings
            .into_iter()
            .flat_map(|(_, actions)| actions)
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
            period_from: None,
            on_fire: vec![TimingAction::SetBits {
                register: "S".into(),
                bits: 1,
            }],
        }]);
        // Serviced late at 15 µs, then again at 21: the second period is due at
        // 20, not at 25 (which is what rescheduling from `now` would give).
        assert_eq!(bank.due_by_timer(15).len(), 1);
        assert_eq!(bank.due_by_timer(21).len(), 1);
    }

    /// A very long jump past a very short period is BOUNDED. Without the cap a
    /// 1 µs timer plus a 10 s advance is ten million iterations inside one bus
    /// tick — a hang, not fidelity. Past the cap the still-due timers re-anchor
    /// past `now`, so the next advance starts clean instead of owing the same
    /// backlog again.
    #[test]
    fn a_long_advance_is_capped_and_re_anchors() {
        use labwired_config::{DeviceTimer, TimerStart, TimingAction};
        let mut bank = TimerBank::new(&[DeviceTimer {
            name: "fast".into(),
            period_us: Some(1),
            after_us: Option::None,
            start: TimerStart::OnReset,
            start_on_write: Option::None,
            period_from: None,
            on_fire: vec![TimingAction::SetBits {
                register: "S".into(),
                bits: 1,
            }],
        }]);
        let firings = bank.due_by_timer(10_000_000);
        assert_eq!(firings.len(), MAX_TIMER_CATCHUP as usize);
        // Re-anchored: the next advance at the same instant owes nothing.
        assert!(bank.due_by_timer(10_000_000).is_empty());
        // And it is still running — a cap is not a stop. One more microsecond
        // is one more period.
        assert_eq!(bank.due_by_timer(10_000_001).len(), 1);
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
            fifo: None,
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
                        wrap: None,
                        bcd: false,
                        value_mask: None,
                        round: None,
                        clamp_from: vec![],
                    }),
                    scale_from: vec![],
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
                        wrap: None,
                        bcd: false,
                        value_mask: None,
                        round: None,
                        clamp_from: vec![],
                    }),
                    scale_from: vec![],
                },
            ],
            page: None,
            self_clearing: None,
            popcount: None,
            zero_when: None,
            bits: vec![],
            on_read: None,
            on_write: None,
            calendar: None,
            zero_unless: None,
            source_from: None,
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
            fifo: None,
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
                    wrap: None,
                    bcd: false,
                    value_mask: None,
                    round: None,
                    clamp_from: vec![],
                }),
                scale_from: vec![],
            }],
            page: None,
            self_clearing: None,
            popcount: None,
            zero_when: None,
            bits: vec![],
            on_read: None,
            on_write: None,
            calendar: None,
            zero_unless: None,
            source_from: None,
        };
        let mut slots = HashMap::new();
        slots.insert("tc".to_string(), -25.0); // -25°C → -100 → 14-bit two's-comp = 0x3F9C, <<18
        let b = register_read_bytes(&r, &slots, &HashMap::new());
        let word = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        assert_eq!((word >> 18) & 0x3FFF, 0x3F9C);
    }

    /// `encode.wrap` — the modular-counter primitive the AS5600 port found
    /// missing. Exercised on `encode_raw` directly so the rounding ORDER is
    /// pinned: a count is produced, THEN reduced.
    mod wrap {
        use super::super::encode_raw;
        use labwired_config::Encode;
        use std::num::NonZeroU32;

        /// The AS5600 encode: 4096 counts per 360°, wrapped at 4096 counts.
        fn as5600() -> Encode {
            Encode {
                scale: 4096.0 / 360.0,
                offset: 0.0,
                clamp_min: None,
                clamp_max: None,
                wrap: NonZeroU32::new(4096),
                bcd: false,
                value_mask: None,
                round: None,
                clamp_from: vec![],
            }
        }

        #[test]
        fn a_full_turn_reads_the_same_count_as_zero() {
            // THE behaviour: 4096 counts is the same shaft position as 0, so a
            // full turn must read 0 and not the impossible 4096 nor a clamped
            // 4095 that is 0.088° short of where the shaft is.
            assert_eq!(encode_raw(0.0, Some(&as5600()), 1.0, 2, false), 0);
            assert_eq!(encode_raw(360.0, Some(&as5600()), 1.0, 2, false), 0);
            assert_eq!(encode_raw(720.0, Some(&as5600()), 1.0, 2, false), 0);
        }

        #[test]
        fn every_count_below_a_full_turn_is_unchanged_by_the_wrap() {
            // The wrap must be invisible everywhere except at the roll-over,
            // or it would be a silent re-scaling of the whole channel. Swept
            // over every one of the 4096 counts rather than spot-checked.
            let plain = Encode {
                wrap: None,
                bcd: false,
                value_mask: None,
                round: None,
                clamp_from: vec![],
                ..as5600()
            };
            for count in 0..4096u32 {
                let deg = f64::from(count) * 360.0 / 4096.0;
                let wrapped = encode_raw(deg, Some(&as5600()), 1.0, 2, false);
                assert_eq!(
                    wrapped,
                    encode_raw(deg, Some(&plain), 1.0, 2, false),
                    "count {count} ({deg}°) moved when `wrap` was added"
                );
                assert_eq!(wrapped, count, "count {count} does not round-trip");
            }
        }

        #[test]
        fn the_count_is_rounded_before_it_is_reduced() {
            // 359.99° is 4095.886 counts. Reducing the FLOAT and rounding
            // afterwards yields 4096 — a count a 12-bit counter cannot hold,
            // which would then be packed as bit 12 set. Rounding first gives
            // 4096 → 0, the position the shaft is actually at.
            let raw = encode_raw(359.99, Some(&as5600()), 1.0, 2, false);
            assert_eq!(raw, 0, "359.99° rounds to a full turn, which reads 0");
            assert!(raw <= 4095, "a 12-bit counter cannot answer {raw}");
        }

        #[test]
        fn a_negative_angle_lands_on_the_count_the_counter_would_show() {
            // `rem_euclid`, not `%`: one degree below zero is one degree below
            // a full turn, which is where the magnet is. A truncating remainder
            // would answer a negative count and pack it as ~full scale by
            // accident rather than by meaning it.
            let expect = 4096 - (4096f64 / 360.0).round() as u32; // -1° → 4085
            assert_eq!(encode_raw(-1.0, Some(&as5600()), 1.0, 2, false), expect);
            assert_eq!(encode_raw(-360.0, Some(&as5600()), 1.0, 2, false), 0);
        }

        #[test]
        fn a_wrap_that_is_not_a_power_of_two_still_rolls_over() {
            // The modulus is a COUNT, not a mask: a 360-count-per-turn part
            // (1°/LSB) rolls at 360, which no bit-width could express.
            let e = Encode {
                scale: 1.0,
                offset: 0.0,
                clamp_min: None,
                clamp_max: None,
                wrap: NonZeroU32::new(360),
                bcd: false,
                value_mask: None,
                round: None,
                clamp_from: vec![],
            };
            assert_eq!(encode_raw(359.0, Some(&e), 1.0, 2, false), 359);
            assert_eq!(encode_raw(360.0, Some(&e), 1.0, 2, false), 0);
            assert_eq!(encode_raw(361.0, Some(&e), 1.0, 2, false), 1);
        }

        #[test]
        fn wrap_zero_is_refused_at_load_rather_than_ignored() {
            // A modulus of zero has no meaning. `NonZeroU32` makes it a load
            // error naming the field instead of a key that parses and does
            // nothing — the silent-no-op failure this schema refuses.
            let err = serde_yaml::from_str::<Encode>("scale: 1.0\nwrap: 0\n")
                .expect_err("wrap: 0 must not parse");
            assert!(
                err.to_string().contains("nonzero"),
                "the error must name the problem, got: {err}"
            );
            assert!(serde_yaml::from_str::<Encode>("scale: 1.0\nwrap: 4096\n").is_ok());
        }
    }

    /// `encode.bcd`, `encode.round` and `encode.clamp_from` — the three keys
    /// the DS3231 / ADXL345 ports added, tested where they live rather than
    /// only through the parts that needed them. A key exercised by exactly one
    /// descriptor is a key whose contract is whatever that descriptor happens
    /// to do.
    mod encode_keys {
        use super::*;
        use labwired_config::{ClampFrom, ClampWindow};

        fn enc() -> Encode {
            Encode {
                scale: 1.0,
                offset: 0.0,
                clamp_min: None,
                clamp_max: None,
                wrap: None,
                bcd: false,
                value_mask: None,
                round: None,
                clamp_from: vec![],
            }
        }

        #[test]
        fn bcd_round_trips_every_two_digit_value() {
            for v in 0..=99i64 {
                let packed = to_bcd(v, 1);
                assert_eq!(
                    packed,
                    u32::from(((v / 10) as u8) << 4 | (v % 10) as u8),
                    "{v} packed wrong"
                );
                assert_eq!(from_bcd(packed, 1), v, "{v} did not round-trip");
            }
        }

        #[test]
        fn bcd_saturates_at_all_nines_rather_than_carrying_into_a_neighbour() {
            // A counter chain runs out of digits; it does not spill into the
            // register next door. 100 in one byte is 0x99, not 0x00 with a carry.
            assert_eq!(to_bcd(100, 1), 0x99);
            assert_eq!(to_bcd(12_345, 2), 0x9999);
            assert_eq!(to_bcd(1234, 2), 0x1234);
            // Negative is not representable in packed BCD.
            assert_eq!(to_bcd(-5, 1), 0x00);
        }

        #[test]
        fn a_nibble_above_nine_decodes_the_way_the_counter_reads_it() {
            // The master put it on the wire and the part has to answer. 0x1A is
            // 1*10 + 10 = 20, which is what the chain's adders produce.
            assert_eq!(from_bcd(0x1A, 1), 20);
            assert_eq!(from_bcd(0xFF, 1), 165);
        }

        #[test]
        fn bcd_is_the_last_step_of_the_encode() {
            // scale and offset happen in DECIMAL, then the count is packed.
            let e = Encode {
                scale: 2.0,
                offset: 1.0,
                ..enc()
            };
            let bcd = Encode {
                bcd: true,
                ..e.clone()
            };
            // 12 * 2 + 1 = 25 ⇒ 0x25, and the same encode without `bcd` is 25.
            assert_eq!(encode_raw(12.0, Some(&e), 1.0, 1, false), 25);
            assert_eq!(encode_raw(12.0, Some(&bcd), 1.0, 1, false), 0x25);
        }

        #[test]
        fn bcd_composes_with_wrap_in_decimal_counts() {
            // A modular counter that reads out as BCD: 62 seconds is :02.
            let e = Encode {
                bcd: true,
                wrap: std::num::NonZeroU32::new(60),
                ..enc()
            };
            assert_eq!(encode_raw(62.0, Some(&e), 1.0, 1, false), 0x02);
            assert_eq!(encode_raw(59.0, Some(&e), 1.0, 1, false), 0x59);
        }

        #[test]
        fn decode_write_is_the_inverse_and_only_for_a_bcd_register() {
            let mut reg = reg("R", 0, 1, Endian::Le, None);
            assert_eq!(
                decode_write(&reg, 0x45),
                0x45,
                "a plain register stores what was written"
            );
            reg.encode = Some(Encode { bcd: true, ..enc() });
            assert_eq!(decode_write(&reg, 0x45), 45);
            // The clamp window applies to the DECODED value.
            reg.encode = Some(Encode {
                bcd: true,
                clamp_min: Some(1.0),
                clamp_max: Some(12.0),
                ..enc()
            });
            assert_eq!(decode_write(&reg, 0x99), 12);
            assert_eq!(decode_write(&reg, 0x00), 1);
        }

        #[test]
        fn rounding_modes_pick_the_count() {
            for (mode, expected) in [
                (Rounding::Nearest, 3i64),
                (Rounding::Floor, 2),
                (Rounding::Ceil, 3),
                (Rounding::Trunc, 2),
            ] {
                let e = Encode {
                    round: Some(mode),
                    ..enc()
                };
                assert_eq!(
                    encode_raw(2.6, Some(&e), 1.0, 1, false),
                    expected as u32,
                    "{mode:?} of 2.6"
                );
            }
            // Negative, where floor and trunc part company.
            for (mode, expected) in [
                (Rounding::Floor, -3i32),
                (Rounding::Trunc, -2),
                (Rounding::Ceil, -2),
            ] {
                let e = Encode {
                    round: Some(mode),
                    ..enc()
                };
                assert_eq!(
                    encode_raw(-2.6, Some(&e), 1.0, 1, true) as u8 as i8,
                    expected as i8,
                    "{mode:?} of -2.6"
                );
            }
            // Absent ⇒ nearest, which is what every descriptor written before
            // the key existed means.
            assert_eq!(encode_raw(2.6, Some(&enc()), 1.0, 1, false), 3);
        }

        fn clamp_from(map: &[(u32, f64, f64)]) -> ClampFrom {
            ClampFrom {
                register: "CFG".into(),
                mask: 0x0B,
                shift: 0,
                map: map
                    .iter()
                    .map(|&(k, min, max)| (k, ClampWindow { min, max }))
                    .collect(),
            }
        }

        #[test]
        fn clamp_from_reads_the_window_out_of_a_register_field() {
            let e = Encode {
                clamp_from: vec![clamp_from(&[
                    (0x00, -512.0, 512.0),
                    (0x0B, -4096.0, 4096.0),
                ])],
                ..enc()
            };
            let mut regs = HashMap::new();
            regs.insert("CFG".to_string(), 0x00u32);
            assert_eq!(resolve_clamp(Some(&e), &regs), (Some(-512.0), Some(512.0)));
            regs.insert("CFG".to_string(), 0x0B);
            assert_eq!(
                resolve_clamp(Some(&e), &regs),
                (Some(-4096.0), Some(4096.0))
            );
        }

        #[test]
        fn an_unmapped_field_value_leaves_the_constant_window_in_force() {
            // Same rule `scale_from` has: unmapped is NEUTRAL, not zero.
            let e = Encode {
                clamp_min: Some(-10.0),
                clamp_max: Some(10.0),
                clamp_from: vec![clamp_from(&[(0x0B, -4096.0, 4096.0)])],
                ..enc()
            };
            let mut regs = HashMap::new();
            regs.insert("CFG".to_string(), 0x02u32); // not in the map
            assert_eq!(resolve_clamp(Some(&e), &regs), (Some(-10.0), Some(10.0)));
        }

        #[test]
        fn several_clamp_from_entries_intersect() {
            // Each narrows the window, so a part whose resolution bit and range
            // bits both bound the count states each once.
            let a = ClampFrom {
                register: "A".into(),
                mask: 0x01,
                shift: 0,
                map: [(
                    1u32,
                    ClampWindow {
                        min: -100.0,
                        max: 100.0,
                    },
                )]
                .into_iter()
                .collect(),
            };
            let b = ClampFrom {
                register: "B".into(),
                mask: 0x01,
                shift: 0,
                map: [(
                    1u32,
                    ClampWindow {
                        min: -50.0,
                        max: 400.0,
                    },
                )]
                .into_iter()
                .collect(),
            };
            let e = Encode {
                clamp_from: vec![a, b],
                ..enc()
            };
            let mut regs = HashMap::new();
            regs.insert("A".to_string(), 1u32);
            regs.insert("B".to_string(), 1u32);
            assert_eq!(resolve_clamp(Some(&e), &regs), (Some(-50.0), Some(100.0)));
        }

        #[test]
        fn a_register_with_no_encode_has_no_window() {
            assert_eq!(resolve_clamp(None, &HashMap::new()), (None, None));
        }
    }

    /// The civil-calendar pair behind `RegisterSpec::calendar`.
    mod calendar {
        use super::*;

        #[test]
        fn unix_and_civil_are_inverses_across_sixty_years() {
            // Every 9 h 13 min 7 s from 1970 to 2030 — a stride that is coprime
            // with the day, so it walks every hour, minute and weekday rather
            // than sampling midnight sixty times.
            let mut t = 0i64;
            while t < 1_900_000_000 {
                let c = civil_from_unix(t);
                assert_eq!(unix_from_civil(c), t, "round trip failed at {t}");
                assert!((1..=12).contains(&c.month), "{t}: month {}", c.month);
                assert!((1..=31).contains(&c.day), "{t}: day {}", c.day);
                assert!((1..=7).contains(&c.weekday), "{t}: weekday {}", c.weekday);
                t += 33_187;
            }
        }

        #[test]
        fn the_epoch_was_a_thursday() {
            // Sunday = 1 makes Thursday 5 — the DS3231/DS1307 convention, and
            // the anchor the whole weekday derivation hangs on.
            let c = civil_from_unix(0);
            assert_eq!((c.year, c.month, c.day), (1970, 1, 1));
            assert_eq!(c.weekday, 5);
        }

        #[test]
        fn a_leap_day_is_a_day() {
            let c = civil_from_unix(1_709_164_800); // 2024-02-29 00:00:00 UTC
            assert_eq!((c.year, c.month, c.day), (2024, 2, 29));
        }

        #[test]
        fn setting_one_field_moves_only_that_field() {
            let mut c = civil_from_unix(1_784_721_600); // 2026-07-22 12:00:00
            calendar_set(&mut c, CalendarField::Hour, 7);
            assert_eq!((c.year, c.month, c.day, c.hour), (2026, 7, 22, 7));
            assert_eq!((c.minute, c.second), (0, 0));
        }

        #[test]
        fn a_field_is_clamped_to_what_it_can_hold() {
            // A nonsense write must not roll the whole clock somewhere else.
            let mut c = civil_from_unix(0);
            calendar_set(&mut c, CalendarField::Hour, 99);
            assert_eq!(c.hour, 23);
            calendar_set(&mut c, CalendarField::Month, 0);
            assert_eq!(c.month, 1);
            calendar_set(&mut c, CalendarField::Second, -4);
            assert_eq!(c.second, 0);
        }

        #[test]
        fn the_year_field_is_two_digits_within_the_current_century() {
            let mut c = civil_from_unix(1_784_721_600); // 2026
            assert_eq!(calendar_get(c, CalendarField::Year), 26);
            calendar_set(&mut c, CalendarField::Year, 31);
            assert_eq!(c.year, 2031);
        }

        #[test]
        fn calendar_get_reads_each_field() {
            let c = civil_from_unix(1_784_725_261); // 2026-07-22 13:01:01, Wed
            assert_eq!(calendar_get(c, CalendarField::Second), 1);
            assert_eq!(calendar_get(c, CalendarField::Minute), 1);
            assert_eq!(calendar_get(c, CalendarField::Hour), 13);
            assert_eq!(calendar_get(c, CalendarField::Day), 22);
            assert_eq!(calendar_get(c, CalendarField::Month), 7);
            assert_eq!(calendar_get(c, CalendarField::Weekday), 4);
        }
    }
}

#[cfg(test)]
mod period_from_tests {
    use super::*;
    use labwired_config::{DeviceTimer, TimerPeriodFrom, TimerStart};
    use std::collections::BTreeMap;

    fn rate_timer(start: TimerStart) -> DeviceTimer {
        DeviceTimer {
            name: "sample".into(),
            period_us: Some(10_000),
            after_us: None,
            start,
            start_on_write: None,
            on_fire: Vec::new(),
            period_from: Some(TimerPeriodFrom {
                register: "BW_RATE".into(),
                field: Some("RATE".into()),
                mask: None,
                shift: 0,
                table: BTreeMap::from([(0x9u32, 20_000u64), (0xAu32, 10_000), (0xDu32, 1_250)]),
            }),
        }
    }

    /// Resolve `period_from` against a register file holding exactly
    /// `BW_RATE = word`, in the shape `apply_period_from` takes.
    fn resolve(bank: &mut TimerBank, now: u64, word: u32) {
        let reg = move |name: &str| (name == "BW_RATE").then_some(word);
        let bits = |register: &str, field: &str| {
            (register == "BW_RATE" && field == "RATE").then_some((0u8, 0x0Fu32))
        };
        bank.apply_period_from(now, &reg, &bits);
    }

    #[test]
    fn the_field_value_selects_the_period() {
        let mut bank = TimerBank::new(&[rate_timer(TimerStart::OnReset)]);
        assert_eq!(
            bank.period_us_of("sample"),
            Some(10_000),
            "the declared one"
        );
        resolve(&mut bank, 0, 0x0D);
        assert_eq!(bank.period_us_of("sample"), Some(1_250), "800 Hz");
        resolve(&mut bank, 0, 0x09);
        assert_eq!(bank.period_us_of("sample"), Some(20_000), "50 Hz");
    }

    /// ⚠️ An unmapped value is NEUTRAL — the declared `period_us` stays in
    /// force. A reserved encoding must not silently stop the part's clock.
    #[test]
    fn an_unmapped_field_value_leaves_the_declared_period() {
        let mut bank = TimerBank::new(&[rate_timer(TimerStart::OnReset)]);
        resolve(&mut bank, 0, 0x0D);
        assert_eq!(bank.period_us_of("sample"), Some(1_250));
        resolve(&mut bank, 0, 0x03); // not in the table
        assert_eq!(bank.period_us_of("sample"), Some(10_000));
    }

    /// ⚠️ A RUNNING timer is re-anchored to `now + the new period`, not
    /// rescheduled from its old deadline. Firmware that rewrote the rate
    /// register restarted the divider.
    #[test]
    fn a_running_timer_re_anchors_on_a_rate_change() {
        let mut bank = TimerBank::new(&[rate_timer(TimerStart::OnReset)]);
        // 10 ms period, armed at 0. Walk to 6 ms, then ask for 800 Hz.
        assert!(bank.due_by_timer(6_000).is_empty());
        resolve(&mut bank, 6_000, 0x0D);
        // The old deadline was 10 000; the new one is 6 000 + 1 250.
        assert!(bank.due_by_timer(7_249).is_empty(), "not yet");
        assert_eq!(
            bank.due_by_timer(7_250).len(),
            1,
            "one period after the write"
        );
    }

    /// A timer that is NOT running does not get armed by a rate change — the
    /// period is resolved, the deadline stays `None`.
    #[test]
    fn a_stopped_timer_resolves_its_period_without_starting() {
        let mut bank = TimerBank::new(&[rate_timer(TimerStart::Manual)]);
        resolve(&mut bank, 0, 0x0D);
        assert_eq!(bank.period_us_of("sample"), Some(1_250));
        assert!(
            bank.due_by_timer(1_000_000).is_empty(),
            "resolving a period must not arm a manual timer"
        );
        bank.start_named("sample", 0);
        assert_eq!(
            bank.due_by_timer(1_250).len(),
            1,
            "and starting it uses the RESOLVED period, not the declared one"
        );
    }

    #[test]
    fn a_part_with_no_field_driven_period_says_so() {
        let plain = DeviceTimer {
            name: "t".into(),
            period_us: Some(5),
            after_us: None,
            start: TimerStart::OnReset,
            start_on_write: None,
            on_fire: Vec::new(),
            period_from: None,
        };
        assert!(!TimerBank::new(&[plain]).has_field_driven_period());
        assert!(TimerBank::new(&[rate_timer(TimerStart::OnReset)]).has_field_driven_period());
    }
}
