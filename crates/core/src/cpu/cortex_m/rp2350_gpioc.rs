// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RP2350 GPIO coprocessor (GPIOC), coprocessor p0 (RP2350 datasheet 3.6.1).
//!
//! GPIOC is a single-cycle path from each Cortex-M33 to its SIO GPIO
//! registers. pico-sdk uses it for every `gpio_put`/`gpio_set_dir` on the
//! RP2350 Arm build (`hardware/gpio_coproc.h`), so arduino-pico's `pinMode`
//! and `digitalWrite` issue `mcrr p0, #4, ...` rather than SIO stores.
//!
//! Each instruction is executed as the equivalent SIO register access at
//! `0xD000_0000`, so the bus's SIO model stays the only GPIO state. The
//! encodings below are the complete set in `gpio_coproc.h`:
//!
//! | instruction | CRm | effect |
//! |---|---|---|
//! | `mcr p0, #0..3, Rt, c0, c{0,1,4,5}` | OUT lo/hi, OE lo/hi | put / xor / set / clr |
//! | `mcrr p0, #0..3, lo, hi, c{0,4}` | OUT, OE (64-bit) | put / xor / set / clr |
//! | `mcrr p0, #4, pin, val, c{0,4}` | OUT, OE | single-bit put |
//! | `mcr p0, #5..7, pin, c0, c{0,4}` | OUT, OE | single-bit xor / set / clr |
//! | `mcrr p0, #5..7, pin, val, c{0,4}` | OUT, OE | `(val & 1) << pin` xor / set / clr |
//! | `mcrr p0, #8..11, val, index, c{0,4}` | OUT, OE word `index` | put / xor / set / clr |
//! | `mrc p0, #0, Rt, c0, c{0,1,4,5,8,9}` | OUT, OE, IN lo/hi | read |
//! | `mrrc p0, #0, lo, hi, c{0,4,8}` | OUT, OE, IN (64-bit) | read |
//!
//! Anything else on p0 returns `None` and stays UNDEFINED.

/// SIO base on the RP2350.
pub const SIO_BASE: u32 = 0xD000_0000;

// RP2350 SIO GPIO offsets (pico-sdk rp2350 hardware_regs/sio.h).
const GPIO_IN: u32 = 0x04;
const GPIO_OUT: u32 = 0x10;
const GPIO_OE: u32 = 0x30;
// Within a register group: +4 selects the HI word; SET/CLR/XOR follow at +8.
const HI: u32 = 0x04;

/// Write operation, in the GPIOC opc1 order (0 put, 1 xor, 2 set, 3 clr).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Put,
    Xor,
    Set,
    Clr,
}

impl Op {
    fn from_index(i: u16) -> Self {
        match i & 3 {
            0 => Op::Put,
            1 => Op::Xor,
            2 => Op::Set,
            _ => Op::Clr,
        }
    }

    /// Offset of the SIO alias register for this op (RP2350 map:
    /// OUT 0x10, OUT_SET 0x18, OUT_CLR 0x20, OUT_XOR 0x28; OE the same +0x20).
    fn delta(self) -> u32 {
        match self {
            Op::Put => 0x00,
            Op::Set => 0x08,
            Op::Clr => 0x10,
            Op::Xor => 0x18,
        }
    }
}

/// One GPIOC instruction, decoded to SIO accesses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpiocOp {
    /// Word writes: `(sio offset, value)` pairs, applied in order.
    Write(Vec<(u32, u32)>),
    /// `mrc`: read one SIO word into Rt.
    Read1 { off: u32, rt: u8 },
    /// `mrrc`: read the lo/hi SIO words into Rt / Rt2.
    Read2 { off: u32, rt: u8, rt2: u8 },
}

/// SIO group base selected by CRm for a write: c0 = OUT, c4 = OE.
fn group(crm: u16) -> Option<u32> {
    match crm {
        0 => Some(GPIO_OUT),
        4 => Some(GPIO_OE),
        _ => None,
    }
}

/// A 64-bit mask op as up to two word writes (a zero half is skipped for
/// set/clr/xor, where it would be a no-op anyway; put always writes both).
fn pair(base: u32, op: Op, lo: u32, hi: u32) -> Vec<(u32, u32)> {
    let mut v = Vec::with_capacity(2);
    if op == Op::Put || lo != 0 {
        v.push((base + op.delta(), lo));
    }
    if op == Op::Put || hi != 0 {
        v.push((base + HI + op.delta(), hi));
    }
    v
}

/// One-bit op on `pin` (0..63): a single word write to the lo or hi word.
fn bit(base: u32, op: Op, pin: u32) -> Vec<(u32, u32)> {
    let pin = pin & 0x3F;
    let word = if pin >= 32 { HI } else { 0 };
    vec![(base + word + op.delta(), 1u32 << (pin & 31))]
}

/// Decode a 32-bit Thumb word as a GPIOC instruction. `read_reg` supplies
/// core register values for write forms.
pub fn decode(h1: u16, h2: u16, read_reg: impl Fn(u8) -> u32) -> Option<GpiocOp> {
    // p0 only, and only the T1 (non-"2") forms.
    if (h2 >> 8) & 0xF != 0 || h1 >> 12 != 0xE {
        return None;
    }
    let rt = ((h2 >> 12) & 0xF) as u8;
    let crm = h2 & 0xF;
    match (h1 >> 4) & 0xFF {
        // MCRR (0xEC4x) / MRRC (0xEC5x).
        0xC4 | 0xC5 => {
            let rt2 = (h1 & 0xF) as u8;
            let opc1 = (h2 >> 4) & 0xF;
            if (h1 >> 4) & 1 == 1 {
                // MRRC p0, #0, lo, hi, c0/c4/c8.
                let off = match (opc1, crm) {
                    (0, 0) => GPIO_OUT,
                    (0, 4) => GPIO_OE,
                    (0, 8) => GPIO_IN,
                    _ => return None,
                };
                return Some(GpiocOp::Read2 { off, rt, rt2 });
            }
            let base = group(crm)?;
            let (a, b) = (read_reg(rt), read_reg(rt2));
            let writes = match opc1 {
                0..=3 => pair(base, Op::from_index(opc1), a, b),
                // Single-bit put: Rt = pin, Rt2 = value.
                4 => bit(base, if b & 1 != 0 { Op::Set } else { Op::Clr }, a),
                // Conditional xor/set/clr of `(val & 1) << pin`.
                5..=7 => {
                    if b & 1 == 0 {
                        Vec::new()
                    } else {
                        bit(base, Op::from_index(opc1 - 4), a)
                    }
                }
                // Indexed word op: Rt = value, Rt2 = word index (0 lo, 1 hi).
                8..=11 => match b {
                    0 => vec![(base + Op::from_index(opc1 - 8).delta(), a)],
                    1 => vec![(base + HI + Op::from_index(opc1 - 8).delta(), a)],
                    _ => Vec::new(), // undefined index: no effect
                },
                _ => return None,
            };
            Some(GpiocOp::Write(writes))
        }
        // MCR / MRC: 0xEExx with h2[4] set.
        b if b >> 4 == 0xE && h2 & 0x10 != 0 => {
            let opc1 = (h1 >> 5) & 0x7;
            let l = (h1 >> 4) & 1;
            let crn = h1 & 0xF;
            let opc2 = (h2 >> 5) & 0x7;
            if crn != 0 || opc2 != 0 {
                return None;
            }
            if l == 1 {
                if opc1 != 0 {
                    return None;
                }
                let off = match crm {
                    0 => GPIO_OUT,
                    1 => GPIO_OUT + HI,
                    4 => GPIO_OE,
                    5 => GPIO_OE + HI,
                    8 => GPIO_IN,
                    9 => GPIO_IN + HI,
                    _ => return None,
                };
                return Some(GpiocOp::Read1 { off, rt });
            }
            let v = read_reg(rt);
            let writes = match (opc1, crm) {
                // Word ops on OUT/OE lo (c0/c4) or hi (c1/c5).
                (0..=3, 0 | 1 | 4 | 5) => {
                    let base = if crm >= 4 { GPIO_OE } else { GPIO_OUT };
                    let word = if crm & 1 == 1 { HI } else { 0 };
                    vec![(base + word + Op::from_index(opc1).delta(), v)]
                }
                // Unconditional single-bit xor/set/clr: Rt = pin.
                (5..=7, 0 | 4) => bit(group(crm)?, Op::from_index(opc1 - 4), v),
                _ => return None,
            };
            Some(GpiocOp::Write(writes))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Encoders for the forms gpio_coproc.h emits (ARMv8-M ARM A7.7.74/75/79/80).
    fn mcr(opc1: u16, rt: u16, crm: u16) -> (u16, u16) {
        (0xEE00 | (opc1 << 5), (rt << 12) | 0x0010 | crm)
    }
    fn mrc(rt: u16, crm: u16) -> (u16, u16) {
        (0xEE10, (rt << 12) | 0x0010 | crm)
    }
    fn mcrr(opc1: u16, rt: u16, rt2: u16, crm: u16) -> (u16, u16) {
        (0xEC40 | rt2, (rt << 12) | (opc1 << 4) | crm)
    }
    fn mrrc(rt: u16, rt2: u16, crm: u16) -> (u16, u16) {
        (0xEC50 | rt2, (rt << 12) | crm)
    }

    fn regs(r: u8) -> u32 {
        // r3 = pin 25, r4 = 1, r5 = 0, r6 = 0xF0, r7 = 1 (index hi).
        match r {
            3 => 25,
            4 => 1,
            5 => 0,
            6 => 0xF0,
            7 => 1,
            _ => 0xDEAD,
        }
    }

    fn dec(w: (u16, u16)) -> Option<GpiocOp> {
        decode(w.0, w.1, regs)
    }

    #[test]
    fn arduino_pico_pinmode_word_is_oe_bit_put() {
        // The exact word the Pico 2 L2 sketch faulted on: 0xEC43 0x4044 =
        // mcrr p0, #4, r4, r3, c4 (gpioc_bit_oe_put(pin=r4, val=r3)).
        // With r4 = 1 and r3 = 25 (odd): set OE bit 1 via OE_SET (0x38).
        assert_eq!(
            decode(0xEC43, 0x4044, regs),
            Some(GpiocOp::Write(vec![(0x38, 1 << 1)]))
        );
    }

    #[test]
    fn bit_put_out_set_and_clear() {
        // gpioc_bit_out_put(25, 1) -> OUT_SET; (25, 0) -> OUT_CLR.
        assert_eq!(
            dec(mcrr(4, 3, 4, 0)),
            Some(GpiocOp::Write(vec![(0x18, 1 << 25)]))
        );
        assert_eq!(
            dec(mcrr(4, 3, 5, 0)),
            Some(GpiocOp::Write(vec![(0x20, 1 << 25)]))
        );
    }

    #[test]
    fn single_bit_xor_set_clr_and_conditional_forms() {
        assert_eq!(
            dec(mcr(5, 3, 0)),
            Some(GpiocOp::Write(vec![(0x28, 1 << 25)]))
        );
        assert_eq!(
            dec(mcr(6, 3, 4)),
            Some(GpiocOp::Write(vec![(0x38, 1 << 25)]))
        );
        assert_eq!(
            dec(mcr(7, 3, 4)),
            Some(GpiocOp::Write(vec![(0x40, 1 << 25)]))
        );
        // Conditional with val = 0 does nothing; val = 1 acts.
        assert_eq!(dec(mcrr(6, 3, 5, 0)), Some(GpiocOp::Write(vec![])));
        assert_eq!(
            dec(mcrr(7, 3, 4, 0)),
            Some(GpiocOp::Write(vec![(0x20, 1 << 25)]))
        );
    }

    #[test]
    fn word_and_pair_ops_hit_the_rp2350_sio_map() {
        // gpioc_lo_out_put / hi_oe_xor.
        assert_eq!(dec(mcr(0, 6, 0)), Some(GpiocOp::Write(vec![(0x10, 0xF0)])));
        assert_eq!(dec(mcr(1, 6, 5)), Some(GpiocOp::Write(vec![(0x4C, 0xF0)])));
        // gpioc_hilo_oe_set(lo = 0xF0, hi = 0): only the lo word moves.
        assert_eq!(
            dec(mcrr(2, 6, 5, 4)),
            Some(GpiocOp::Write(vec![(0x38, 0xF0)]))
        );
        // gpioc_hilo_out_put writes both words.
        assert_eq!(
            dec(mcrr(0, 6, 5, 0)),
            Some(GpiocOp::Write(vec![(0x10, 0xF0), (0x14, 0)]))
        );
        // gpioc_index_out_clr(index = r7 = 1, mask = r6): Rt = mask, Rt2 = index.
        assert_eq!(
            dec(mcrr(11, 6, 7, 0)),
            Some(GpiocOp::Write(vec![(0x24, 0xF0)]))
        );
    }

    #[test]
    fn reads_map_to_out_oe_in() {
        assert_eq!(dec(mrc(2, 8)), Some(GpiocOp::Read1 { off: 0x04, rt: 2 }));
        assert_eq!(dec(mrc(2, 5)), Some(GpiocOp::Read1 { off: 0x34, rt: 2 }));
        assert_eq!(
            dec(mrrc(0, 1, 0)),
            Some(GpiocOp::Read2 {
                off: 0x10,
                rt: 0,
                rt2: 1
            })
        );
    }

    #[test]
    fn other_coprocessors_and_unlisted_forms_stay_undefined() {
        // p4 (DCP) is not GPIOC.
        assert_eq!(decode(0xEC43, 0x4444, regs), None);
        // mcr2 (T2) is not GPIOC.
        assert_eq!(decode(0xFE00, 0x3010, regs), None);
        // Unlisted CRm.
        assert_eq!(dec(mcr(0, 3, 2)), None);
        assert_eq!(dec(mrc(2, 3)), None);
    }
}
