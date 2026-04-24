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
