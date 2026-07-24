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

use labwired_config::{Encode, Endian, LabDescriptor, RegisterSpec};

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
pub(crate) fn scale_from_one(sf: &labwired_config::ScaleFrom, reg_values: &HashMap<String, u32>) -> f64 {
    let regval = reg_values.get(&sf.register).copied().unwrap_or(0);
    let field = (regval >> sf.shift as u32) & sf.mask;
    sf.map.get(&field).copied().unwrap_or(1.0)
}

/// Product of a register's `scale_from` factors, folded left-to-right from 1.0.
pub(crate) fn scale_from_product(reg: &RegisterSpec, reg_values: &HashMap<String, u32>) -> f64 {
    reg.scale_from.iter().fold(1.0, |acc, sf| acc * scale_from_one(sf, reg_values))
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

#[cfg(test)]
mod tests {
    use super::*;
    use labwired_config::{Endian, RegisterAccess, RegisterSpec};
    use std::collections::HashMap;

    fn reg(name: &str, addr: u8, width: u8, endian: Endian, source: Option<&str>) -> RegisterSpec {
        RegisterSpec {
            name: name.into(),
            addr,
            width,
            endian,
            access: RegisterAccess::R,
            reset: 0,
            source: source.map(Into::into),
            encode: None,
            scale_from: vec![],
            source_scale: None,
            resolution: None,
            signed: false,
            fields: vec![],
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
        };
        let mut slots = HashMap::new();
        slots.insert("ax".to_string(), -1.0); // -1 g × 256 = -256 = 0xFF00 two's-complement, LE
        assert_eq!(
            register_read_bytes(&r, &slots, &HashMap::new()),
            vec![0x00, 0xFF]
        );
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
        };
        let mut slots = HashMap::new();
        slots.insert("tc".to_string(), -25.0); // -25°C → -100 → 14-bit two's-comp = 0x3F9C, <<18
        let b = register_read_bytes(&r, &slots, &HashMap::new());
        let word = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        assert_eq!((word >> 18) & 0x3FFF, 0x3F9C);
    }
}
