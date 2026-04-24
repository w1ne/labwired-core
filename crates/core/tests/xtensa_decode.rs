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

fn rri8(r: u32, s: u32, t: u32, imm8: u32) -> u32 {
    0x2 | (t << 4) | (s << 8) | (r << 12) | ((imm8 & 0xFF) << 16)
}

#[test]
fn decode_l8ui() {
    let w = rri8(0x0, 4, 5, 0x10);
    assert_eq!(decode(w), Instruction::L8ui { at: 5, as_: 4, imm: 0x10 });
}

#[test]
fn decode_l16ui() {
    let w = rri8(0x1, 4, 5, 0x10);
    assert_eq!(decode(w), Instruction::L16ui { at: 5, as_: 4, imm: 0x20 }); // 0x10 << 1
}

#[test]
fn decode_l32i() {
    let w = rri8(0x2, 4, 5, 0x10);
    assert_eq!(decode(w), Instruction::L32i { at: 5, as_: 4, imm: 0x40 }); // 0x10 << 2
}

#[test]
fn decode_s8i_s16i_s32i() {
    assert_eq!(decode(rri8(0x4, 4, 5, 0x10)), Instruction::S8i  { at: 5, as_: 4, imm: 0x10 });
    assert_eq!(decode(rri8(0x5, 4, 5, 0x10)), Instruction::S16i { at: 5, as_: 4, imm: 0x20 });
    assert_eq!(decode(rri8(0x6, 4, 5, 0x10)), Instruction::S32i { at: 5, as_: 4, imm: 0x40 });
}

#[test]
fn decode_l16si() {
    let w = rri8(0x9, 4, 5, 0x10);
    assert_eq!(decode(w), Instruction::L16si { at: 5, as_: 4, imm: 0x20 });
}

#[test]
fn decode_l32ai() {
    let w = rri8(0xB, 4, 5, 0x10);
    assert_eq!(decode(w), Instruction::L32ai { at: 5, as_: 4, imm: 0x40 });
}

#[test]
fn decode_addi_positive() {
    let w = rri8(0xC, 4, 5, 0x10);
    assert_eq!(decode(w), Instruction::Addi { at: 5, as_: 4, imm8: 0x10 });
}

#[test]
fn decode_addi_negative() {
    // imm8 = 0xFF => sext => -1
    let w = rri8(0xC, 4, 5, 0xFF);
    assert_eq!(decode(w), Instruction::Addi { at: 5, as_: 4, imm8: -1 });
}

#[test]
fn decode_addmi() {
    // ADDMI: imm is sext8 << 8
    let w = rri8(0xD, 4, 5, 0x10);
    assert_eq!(decode(w), Instruction::Addmi { at: 5, as_: 4, imm: 0x1000 }); // 0x10 << 8
    // negative case: imm8 = 0xFF => sext=-1 => imm = -256
    let w = rri8(0xD, 4, 5, 0xFF);
    assert_eq!(decode(w), Instruction::Addmi { at: 5, as_: 4, imm: -256 });
}

#[test]
fn decode_s32c1i() {
    let w = rri8(0xE, 4, 5, 0x10);
    assert_eq!(decode(w), Instruction::S32c1i { at: 5, as_: 4, imm: 0x40 });
}

#[test]
fn decode_s32ri() {
    let w = rri8(0xF, 4, 5, 0x10);
    assert_eq!(decode(w), Instruction::S32ri { at: 5, as_: 4, imm: 0x40 });
}

#[test]
fn decode_lsai_unknown_subop() {
    // r=0x3 is unassigned in LSAI — should be Unknown
    let w = rri8(0x3, 4, 5, 0x10);
    match decode(w) {
        Instruction::Unknown(_) => (),
        other => panic!("expected Unknown, got {:?}", other),
    }
}

// ── Branch family (Task B7) ──────────────────────────────────────────────────

// BR format (op0=0x7): BEQ/BNE/BLT/BGE/BLTU/BGEU/BANY/BALL/BNONE/BNALL/BBC/BBS/BBCI/BBSI
//
// Xtensa BR format (ISA RM §3.2):
//   bits[3:0]  = op0 = 0x7
//   bits[7:4]  = t   (second register / at)
//   bits[11:8] = s   (first register / as_)
//   bits[15:12]= r   (bit-index for BBCI/BBSI; unused for reg-reg branches)
//   bits[23:16]= imm8 (8-bit signed branch offset, PC-relative to PC+4)
//   bits[23:20]= op2 (the branch condition sub-opcode, occupies HIGH NIBBLE of imm8)
//
// Note: op2 and imm8 share bits[23:16]. op2 is the high nibble of imm8. This means
// for a given branch type (fixed op2), valid offsets are restricted to the range where
// sext8((op2<<4)|low_nibble) is a meaningful offset. The decoder simply extracts the
// full 8-bit imm8 field and sign-extends it; the test must place a consistent byte.
//
// Helper: build a BR-format word. `imm8` is the full 8-bit offset byte (bits[23:16]);
// op2 must match the top nibble of imm8 (the decoder extracts op2 from bits[23:20]).
fn br_word(op2: u32, r: u32, s: u32, t: u32, imm8: u32) -> u32 {
    // Caller must ensure (imm8 >> 4) == op2 for a consistent word.
    debug_assert_eq!(imm8 >> 4, op2, "imm8 top nibble must equal op2 in BR format");
    0x7 | (t << 4) | (s << 8) | (r << 12) | ((imm8 & 0xFF) << 16)
}

#[test]
fn decode_beq() {
    // BEQ as_, at, offset : op0=0x7, op2=0x1.
    // imm8 must have top nibble = op2 = 0x1, e.g. imm8=0x10 → sext8(0x10)+4 = 16+4 = 20.
    let w = br_word(0x1, 0, 2, 3, 0x10);
    assert_eq!(decode(w), Instruction::Beq { as_: 2, at: 3, offset: 20 });
}

#[test]
fn decode_bne_bge_blt_bltu_bgeu() {
    // BNE op2=0x9: imm8 top nibble = 0x9 → use imm8=0x90 → sext8(0x90)+4 = -112+4 = -108.
    let w = br_word(0x9, 0, 2, 3, 0x90);
    assert_eq!(decode(w), Instruction::Bne { as_: 2, at: 3, offset: -108 });
    // BGE op2=0xA: imm8=0xA0 → sext8(0xA0)+4 = -96+4 = -92.
    let w = br_word(0xA, 0, 2, 3, 0xA0);
    assert_eq!(decode(w), Instruction::Bge { as_: 2, at: 3, offset: -92 });
    // BLT op2=0x2: imm8=0x25 → sext8(0x25)+4 = 37+4 = 41.
    let w = br_word(0x2, 0, 2, 3, 0x25);
    assert_eq!(decode(w), Instruction::Blt { as_: 2, at: 3, offset: 41 });
    // BLTU op2=0x3: imm8=0x30 → sext8(0x30)+4 = 48+4 = 52.
    let w = br_word(0x3, 0, 2, 3, 0x30);
    assert_eq!(decode(w), Instruction::Bltu { as_: 2, at: 3, offset: 52 });
    // BGEU op2=0xB: imm8=0xB0 → sext8(0xB0)+4 = -80+4 = -76.
    let w = br_word(0xB, 0, 2, 3, 0xB0);
    assert_eq!(decode(w), Instruction::Bgeu { as_: 2, at: 3, offset: -76 });
}

#[test]
fn decode_bany_ball_bnone_bnall() {
    // BANY op2=0x8: imm8=0x80 → sext8(0x80)+4 = -128+4 = -124.
    let w = br_word(0x8, 0, 2, 3, 0x80);
    assert_eq!(decode(w), Instruction::Bany { as_: 2, at: 3, offset: -124 });
    // BALL op2=0x4: imm8=0x44 → sext8(0x44)+4 = 68+4 = 72.
    let w = br_word(0x4, 0, 2, 3, 0x44);
    assert_eq!(decode(w), Instruction::Ball { as_: 2, at: 3, offset: 72 });
    // BNONE op2=0x0: imm8=0x04 → sext8(0x04)+4 = 4+4 = 8.
    let w = br_word(0x0, 0, 2, 3, 0x04);
    assert_eq!(decode(w), Instruction::Bnone { as_: 2, at: 3, offset: 8 });
    // BNALL op2=0xC: imm8=0xC4 → sext8(0xC4)+4 = -60+4 = -56.
    let w = br_word(0xC, 0, 2, 3, 0xC4);
    assert_eq!(decode(w), Instruction::Bnall { as_: 2, at: 3, offset: -56 });
}

#[test]
fn decode_bbc_bbs_bbci_bbsi() {
    // BBC op2=0x5: imm8=0x54 → sext8(0x54)+4 = 84+4 = 88.
    let w = br_word(0x5, 0, 2, 3, 0x54);
    assert_eq!(decode(w), Instruction::Bbc { as_: 2, at: 3, offset: 88 });
    // BBS op2=0xD: imm8=0xD4 → sext8(0xD4)+4 = -44+4 = -40.
    let w = br_word(0xD, 0, 2, 3, 0xD4);
    assert_eq!(decode(w), Instruction::Bbs { as_: 2, at: 3, offset: -40 });
    // BBCI op2=0x6, r=7: bit = (7&0xF) | ((0x6&0x1)<<4) = 7|0 = 7. imm8=0x64 → offset=104.
    let w = br_word(0x6, 7, 2, 3, 0x64);
    assert_eq!(decode(w), Instruction::Bbci { as_: 2, bit: 7, offset: 104 });
    // BBCI op2=0x7, r=7: bit = 7 | ((0x7&0x1)<<4) = 7|16 = 23. imm8=0x74 → offset=120.
    let w = br_word(0x7, 7, 2, 3, 0x74);
    assert_eq!(decode(w), Instruction::Bbci { as_: 2, bit: 23, offset: 120 });
    // BBSI op2=0xE, r=7: bit = 7 | ((0xE&0x1)<<4) = 7|0 = 7. imm8=0xE4 → offset=-24.
    let w = br_word(0xE, 7, 2, 3, 0xE4);
    assert_eq!(decode(w), Instruction::Bbsi { as_: 2, bit: 7, offset: -24 });
}

// J instruction (op0=0x6, m=0, n=0)

#[test]
fn decode_j() {
    // J offset: imm18 = bits[23:6]; encode imm18=0 → offset = 0+4 = 4
    let w = 0x6u32;
    assert_eq!(decode(w), Instruction::J { offset: 4 });

    // imm18 = 0x10 → offset = 16+4 = 20
    let w = 0x6u32 | (0x10u32 << 6);
    assert_eq!(decode(w), Instruction::J { offset: 20 });
}

// BI group (op0=0x6, m=1): BEQI/BNEI/BLTI/BGEI with B4CONST table

#[test]
fn decode_beqi_bnei_blti_bgei() {
    // BEQI: m=1 (bits[7:6]=01), n=0 (bits[5:4]=00), s=as_, r=b4const index, imm8=offset
    // b4const(5) = 5; offset = sext8(0x10)+4 = 16+4 = 20
    let w = 0x6u32 | (0x40u32) | (0u32 << 4) | (2u32 << 8) | (5u32 << 12) | (0x10u32 << 16);
    assert_eq!(decode(w), Instruction::Beqi { as_: 2, imm: 5, offset: 20 });

    // BGEI: n=3, r=15 → b4const(15)=256
    let w = 0x6u32 | (0x40u32) | (3u32 << 4) | (2u32 << 8) | (15u32 << 12) | (0x10u32 << 16);
    assert_eq!(decode(w), Instruction::Bgei { as_: 2, imm: 256, offset: 20 });
}

// BIU group (op0=0x6, m=2): BLTUI/BGEUI with B4CONSTU table

#[test]
fn decode_bltui_bgeui() {
    // BLTUI: m=2 (bits[7:6]=10), n=0, b4constu(5)=5
    let w = 0x6u32 | (0x80u32) | (0u32 << 4) | (2u32 << 8) | (5u32 << 12) | (0x10u32 << 16);
    assert_eq!(decode(w), Instruction::Bltui { as_: 2, imm: 5, offset: 20 });

    // BGEUI: n=1, b4constu(0)=32768
    let w = 0x6u32 | (0x80u32) | (1u32 << 4) | (2u32 << 8) | (0u32 << 12) | (0x10u32 << 16);
    assert_eq!(decode(w), Instruction::Bgeui { as_: 2, imm: 32768, offset: 20 });
}
