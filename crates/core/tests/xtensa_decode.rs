use labwired_core::decoder::xtensa::{decode, Instruction};

#[test]
fn unknown_words_decode_as_unknown() {
    let ins = decode(0xFFFF_FFFF);
    assert!(matches!(ins, Instruction::Unknown(0x00FF_FFFF)));
}

#[test]
fn entry_point_ignores_high_byte_for_wide_ops() {
    let bits = 0xAA_12_34_56u32; // top byte must be ignored for 24-bit decode
    let ins = decode(bits);
    // Only low 24 bits may influence the decoded variant.
    let truncated = decode(bits & 0x00FF_FFFF);
    assert_eq!(ins, truncated);
}

fn rrr(op2: u32, op1: u32, r: u32, s: u32, t: u32) -> u32 {
    (op2 << 20) | (op1 << 16) | (r << 12) | (s << 8) | (t << 4) | 0x0
}

#[test]
fn decode_add() {
    // ADD ar, as_, at  →  op2=0x8, op1=0x0
    let w = rrr(0x8, 0x0, 3, 4, 5);
    assert_eq!(decode(w), Instruction::Add { ar: 3, as_: 4, at: 5 });
}

#[test]
fn decode_sub() {
    // SUB ar, as_, at  →  op2=0xC, op1=0x0
    let w = rrr(0xC, 0x0, 1, 2, 3);
    assert_eq!(decode(w), Instruction::Sub { ar: 1, as_: 2, at: 3 });
}

#[test]
fn decode_and_or_xor() {
    assert_eq!(decode(rrr(0x1, 0x0, 7, 8, 9)), Instruction::And { ar: 7, as_: 8, at: 9 });
    assert_eq!(decode(rrr(0x2, 0x0, 1, 1, 1)), Instruction::Or { ar: 1, as_: 1, at: 1 });
    assert_eq!(decode(rrr(0x3, 0x0, 1, 2, 3)), Instruction::Xor { ar: 1, as_: 2, at: 3 });
}

#[test]
fn decode_neg_abs() {
    // NEG ar, at — op2=0x6, op1=0x0, s == 0, t = at, r = ar
    assert_eq!(decode(rrr(0x6, 0x0, 5, 0, 4)), Instruction::Neg { ar: 5, at: 4 });
    // ABS — op2=0x6, op1=0x0, s == 1
    assert_eq!(decode(rrr(0x6, 0x0, 5, 1, 4)), Instruction::Abs { ar: 5, at: 4 });
}

#[test]
fn decode_addx_subx() {
    // ADDX2: op2=0x9, op1=0x0;  ADDX4: op2=0xA;  ADDX8: op2=0xB
    assert_eq!(decode(rrr(0x9, 0x0, 1, 2, 3)), Instruction::Addx2 { ar: 1, as_: 2, at: 3 });
    assert_eq!(decode(rrr(0xA, 0x0, 1, 2, 3)), Instruction::Addx4 { ar: 1, as_: 2, at: 3 });
    assert_eq!(decode(rrr(0xB, 0x0, 1, 2, 3)), Instruction::Addx8 { ar: 1, as_: 2, at: 3 });
    // SUBX2: op2=0xD; SUBX4: 0xE; SUBX8: 0xF
    assert_eq!(decode(rrr(0xD, 0x0, 1, 2, 3)), Instruction::Subx2 { ar: 1, as_: 2, at: 3 });
    assert_eq!(decode(rrr(0xE, 0x0, 1, 2, 3)), Instruction::Subx4 { ar: 1, as_: 2, at: 3 });
    assert_eq!(decode(rrr(0xF, 0x0, 1, 2, 3)), Instruction::Subx8 { ar: 1, as_: 2, at: 3 });
}

#[test]
fn decode_sll() {
    // SLL ar, as_ : op2=0xA, op1=0x1, r=ar, s=as_, t=0
    let w = rrr(0xA, 0x1, 3, 4, 0);
    assert_eq!(decode(w), Instruction::Sll { ar: 3, as_: 4 });
}

#[test]
fn decode_srl() {
    // SRL ar, at : op2=0x9, op1=0x1, r=ar, s=0, t=at
    let w = rrr(0x9, 0x1, 3, 0, 5);
    assert_eq!(decode(w), Instruction::Srl { ar: 3, at: 5 });
}

#[test]
fn decode_sra() {
    // SRA ar, at : op2=0xB, op1=0x1, r=ar, s=0, t=at
    let w = rrr(0xB, 0x1, 3, 0, 5);
    assert_eq!(decode(w), Instruction::Sra { ar: 3, at: 5 });
}

#[test]
fn decode_src() {
    // SRC ar, as_, at : op2=0x8, op1=0x1
    let w = rrr(0x8, 0x1, 1, 2, 3);
    assert_eq!(decode(w), Instruction::Src { ar: 1, as_: 2, at: 3 });
}

#[test]
fn decode_slli() {
    // SLLI ar, as_, shamt : op2=0x0, op1=0x1, r=ar, s=as_, t=encoded
    // ISA RM §8 SLLI: encodes 1_sa = 32 - sa across {op2[0], t[3:0]}.
    // raw = (op2 & 1) << 4 | t; shamt = 32 - raw.
    // Use raw=27 (op2=0x1 giving bit4=1, t=0xB=11) → shamt = 32 - 27 = 5.
    let w = rrr(0x1, 0x1, 3, 4, 11);
    match decode(w) {
        Instruction::Slli { ar, as_, shamt } => {
            assert_eq!(ar, 3);
            assert_eq!(as_, 4);
            // ISA RM: shamt = 32 - raw, raw = 27 → shamt = 5.
            assert_eq!(shamt, 5);
        }
        other => panic!("expected Slli, got {:?}", other),
    }
}

#[test]
fn decode_srli() {
    // SRLI ar, at, shamt : op2=0x4, op1=0x1, r=ar, s=0, t=at
    // ISA RM §8 SRLI: shamt = t directly (4-bit, 0..15).
    let w = rrr(0x4, 0x1, 3, 0, 7);
    match decode(w) {
        Instruction::Srli { ar, at, shamt } => {
            assert_eq!(ar, 3);
            assert_eq!(at, 7);
            // ISA RM: shamt = t directly for SRLI.
            assert_eq!(shamt, 7);
        }
        other => panic!("expected Srli, got {:?}", other),
    }
}

#[test]
fn decode_srai() {
    // SRAI ar, at, shamt : op2=0x2, op1=0x1
    // ISA RM §8 SRAI: shamt = ((op2 & 1) << 4) | t (direct, no complement).
    let w = rrr(0x2, 0x1, 1, 0, 3);
    match decode(w) {
        Instruction::Srai { ar, at, shamt } => {
            assert_eq!(ar, 1);
            assert_eq!(at, 3);
            // op2=0x2 → op2&1=0; raw = (0<<4)|3 = 3. shamt = 3.
            assert_eq!(shamt, 3);
        }
        other => panic!("expected Srai, got {:?}", other),
    }
}

#[test]
fn decode_ssl_ssr_ssai() {
    // SSR as_ : op0=0, op1=0, op2=4, r=0
    let w = rrr(0x4, 0x0, 0, 5, 0);
    assert_eq!(decode(w), Instruction::Ssr { as_: 5 });
    // SSL as_ : op0=0, op1=0, op2=4, r=1
    let w = rrr(0x4, 0x0, 1, 5, 0);
    assert_eq!(decode(w), Instruction::Ssl { as_: 5 });
    // SSAI shamt=9 : op0=0, op1=0, op2=4, r=4
    // ISA RM §8 SSAI: shamt is 5-bit; encoded as {t[0], s[3:0]}.
    // shamt=9 → low4 = 9, bit4 = 0 → s=9, t=0.
    let w = rrr(0x4, 0x0, 4, 9, 0);
    assert_eq!(decode(w), Instruction::Ssai { shamt: 9 });
}

#[test]
fn decode_l32r() {
    // at=3, imm16 = 0xFFFE => signed -2 => offset in bytes = -2*4 = -8
    // Word encoding: op0=0x1 at bits[3:0], at=3 at bits[7:4], imm16=0xFFFE at bits[23:8]
    let w = 0x0001u32 | (3u32 << 4) | (0xFFFEu32 << 8);
    match decode(w) {
        Instruction::L32r { at, pc_rel_byte_offset } => {
            assert_eq!(at, 3);
            assert_eq!(pc_rel_byte_offset, -8);
        }
        other => panic!("expected L32R, got {:?}", other),
    }
}

#[test]
fn decode_l32r_positive_imm() {
    // at=5, imm16 = 0x0001 => signed +1 => offset = +4 bytes
    let w = 0x0001u32 | (5u32 << 4) | (0x0001u32 << 8);
    match decode(w) {
        Instruction::L32r { at, pc_rel_byte_offset } => {
            assert_eq!(at, 5);
            assert_eq!(pc_rel_byte_offset, 4);
        }
        other => panic!("expected L32R, got {:?}", other),
    }
}

#[test]
fn decode_l32r_large_negative() {
    // at=1, imm16 = 0x8000 (most negative 16-bit signed) => -32768 word-offset
    // byte-offset = -32768 * 4 = -131072
    let w = 0x0001u32 | (1u32 << 4) | (0x8000u32 << 8);
    match decode(w) {
        Instruction::L32r { at, pc_rel_byte_offset } => {
            assert_eq!(at, 1);
            assert_eq!(pc_rel_byte_offset, -131072);
        }
        other => panic!("expected L32R, got {:?}", other),
    }
}
