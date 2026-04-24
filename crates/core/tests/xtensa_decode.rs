use labwired_core::decoder::xtensa::{decode, Instruction};
use labwired_core::decoder::xtensa_narrow::decode_narrow;

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
// Xtensa BR format (ISA RM §3.2, RRI8):
//   bits[3:0]  = op0 = 0x7
//   bits[7:4]  = t   (second register / at, or low 4 bits of bit-index for BBCI/BBSI)
//   bits[11:8] = s   (first register / as_)
//   bits[15:12]= r   (sub-op selector; high bit of bit-index for BBCI/BBSI)
//   bits[23:16]= imm8 (8-bit signed branch byte offset, PC-relative to PC+4)
//
// Dispatch is on r (bits[15:12]).
// For BBCI/BBSI the 5-bit bit-index = ((r & 1) << 4) | (t & 0xF).

/// Pack an RRI8-format instruction.
/// op0 at bits[3:0], t at bits[7:4], s at bits[11:8], r at bits[15:12], imm8 at bits[23:16].
fn rri8_pack(op0: u32, t: u32, s: u32, r: u32, imm8: u32) -> u32 {
    (op0 & 0xF) | ((t & 0xF) << 4) | ((s & 0xF) << 8) | ((r & 0xF) << 12) | ((imm8 & 0xFF) << 16)
}

/// Pack a BRI12-format instruction. op0=6, n at bits[5:4], m at bits[7:6], s at bits[11:8], imm12 at bits[23:12].
fn bri12_pack(m: u32, n: u32, s: u32, imm12: u32) -> u32 {
    0x6 | ((n & 0x3) << 4) | ((m & 0x3) << 6) | ((s & 0xF) << 8) | ((imm12 & 0xFFF) << 12)
}

/// Pack J (CALL format with op0=6, n=0): imm18 at bits[23:6].
fn j_pack(imm18: u32) -> u32 {
    0x6 | ((imm18 & 0x3_FFFF) << 6)
}

#[test]
fn decode_beq() {
    // BEQ as_=2, at=3, imm8=0x10 → decoded offset = sext8(0x10)+4 = 16+4 = 20
    let w = rri8_pack(0x7, 3, 2, 0x1, 0x10);
    assert_eq!(decode(w), Instruction::Beq { as_: 2, at: 3, offset: 20 });
}

#[test]
fn decode_bne_bge_blt_bltu_bgeu() {
    // BNE r=0x9, imm8=0x05 → sext8(5)+4 = 9
    assert_eq!(decode(rri8_pack(0x7, 3, 2, 0x9, 0x05)), Instruction::Bne   { as_: 2, at: 3, offset: 9 });
    // BGE r=0xA
    assert_eq!(decode(rri8_pack(0x7, 3, 2, 0xA, 0x05)), Instruction::Bge   { as_: 2, at: 3, offset: 9 });
    // BLT r=0x2
    assert_eq!(decode(rri8_pack(0x7, 3, 2, 0x2, 0x05)), Instruction::Blt   { as_: 2, at: 3, offset: 9 });
    // BLTU r=0x3
    assert_eq!(decode(rri8_pack(0x7, 3, 2, 0x3, 0x05)), Instruction::Bltu  { as_: 2, at: 3, offset: 9 });
    // BGEU r=0xB
    assert_eq!(decode(rri8_pack(0x7, 3, 2, 0xB, 0x05)), Instruction::Bgeu  { as_: 2, at: 3, offset: 9 });
}

#[test]
fn decode_bany_ball_bnone_bnall() {
    // BANY r=0x8, imm8=0x04 → offset = 4+4 = 8
    assert_eq!(decode(rri8_pack(0x7, 3, 2, 0x8, 0x04)), Instruction::Bany  { as_: 2, at: 3, offset: 8 });
    // BALL r=0x4
    assert_eq!(decode(rri8_pack(0x7, 3, 2, 0x4, 0x04)), Instruction::Ball  { as_: 2, at: 3, offset: 8 });
    // BNONE r=0x0
    assert_eq!(decode(rri8_pack(0x7, 3, 2, 0x0, 0x04)), Instruction::Bnone { as_: 2, at: 3, offset: 8 });
    // BNALL r=0xC
    assert_eq!(decode(rri8_pack(0x7, 3, 2, 0xC, 0x04)), Instruction::Bnall { as_: 2, at: 3, offset: 8 });
}

#[test]
fn decode_bbc_bbs_bbci_bbsi() {
    // BBC r=0x5, BBS r=0xD — use t=3 as at, imm8=0x04 → offset=8
    assert_eq!(decode(rri8_pack(0x7, 3, 2, 0x5, 0x04)), Instruction::Bbc { as_: 2, at: 3, offset: 8 });
    assert_eq!(decode(rri8_pack(0x7, 3, 2, 0xD, 0x04)), Instruction::Bbs { as_: 2, at: 3, offset: 8 });
    // BBCI r=0x6 (high bit of bit-index=0), t=7 → bit = ((0&1)<<4)|7 = 7
    assert_eq!(decode(rri8_pack(0x7, 7, 2, 0x6, 0x04)), Instruction::Bbci { as_: 2, bit: 7,  offset: 8 });
    // BBCI r=0x7 (high bit=1), t=7 → bit = ((1&1)<<4)|7 = 7|16 = 23
    assert_eq!(decode(rri8_pack(0x7, 7, 2, 0x7, 0x04)), Instruction::Bbci { as_: 2, bit: 23, offset: 8 });
    // BBSI r=0xE (high bit=0), t=7 → bit=7
    assert_eq!(decode(rri8_pack(0x7, 7, 2, 0xE, 0x04)), Instruction::Bbsi { as_: 2, bit: 7,  offset: 8 });
    // BBSI r=0xF (high bit=1), t=7 → bit=23
    assert_eq!(decode(rri8_pack(0x7, 7, 2, 0xF, 0x04)), Instruction::Bbsi { as_: 2, bit: 23, offset: 8 });
}

// BZ family (op0=6, n=1): BEQZ/BNEZ/BLTZ/BGEZ, m selects sub-op.

#[test]
fn decode_beqz_bnez_bltz_bgez() {
    // imm12=0x010 → sext12(0x010)=16 → offset = 16+4 = 20
    assert_eq!(decode(bri12_pack(0, 1, 2, 0x010)), Instruction::Beqz { as_: 2, offset: 20 });
    assert_eq!(decode(bri12_pack(1, 1, 2, 0x010)), Instruction::Bnez { as_: 2, offset: 20 });
    assert_eq!(decode(bri12_pack(2, 1, 2, 0x010)), Instruction::Bltz { as_: 2, offset: 20 });
    assert_eq!(decode(bri12_pack(3, 1, 2, 0x010)), Instruction::Bgez { as_: 2, offset: 20 });
    // Negative imm12: 0xFFE = -2 → offset = -2+4 = 2
    assert_eq!(decode(bri12_pack(0, 1, 2, 0xFFE)), Instruction::Beqz { as_: 2, offset: 2 });
}

// BI group (op0=6, n=2): BEQI/BNEI/BLTI/BGEI with B4CONST table, m selects sub-op.

#[test]
fn decode_beqi_bnei_blti_bgei() {
    // BEQI: n=2, m=0, s=2, r=5 → b4const[5]=5, imm8=0x10 → offset=20
    assert_eq!(decode(0x6u32 | (2u32 << 4) | (0u32 << 6) | (2u32 << 8) | (5u32 << 12) | (0x10u32 << 16)),
        Instruction::Beqi { as_: 2, imm: 5, offset: 20 });
    // BGEI: n=2, m=3, r=15 → b4const[15]=256
    assert_eq!(decode(0x6u32 | (2u32 << 4) | (3u32 << 6) | (2u32 << 8) | (15u32 << 12) | (0x10u32 << 16)),
        Instruction::Bgei { as_: 2, imm: 256, offset: 20 });
}

// BIU group (op0=6, n=3): BLTUI/BGEUI with B4CONSTU table; m=0,1 reserved.

#[test]
fn decode_bltui_bgeui() {
    // BLTUI: n=3, m=2, s=2, r=5 → b4constu[5]=5, imm8=0x10 → offset=20
    assert_eq!(decode(0x6u32 | (3u32 << 4) | (2u32 << 6) | (2u32 << 8) | (5u32 << 12) | (0x10u32 << 16)),
        Instruction::Bltui { as_: 2, imm: 5, offset: 20 });
    // BGEUI: n=3, m=3, r=0 → b4constu[0]=32768
    assert_eq!(decode(0x6u32 | (3u32 << 4) | (3u32 << 6) | (2u32 << 8) | (0u32 << 12) | (0x10u32 << 16)),
        Instruction::Bgeui { as_: 2, imm: 32768, offset: 20 });
    // BIU m=0 is reserved → Unknown
    match decode(0x6u32 | (3u32 << 4) | (0u32 << 6) | (2u32 << 8) | (5u32 << 12) | (0x10u32 << 16)) {
        Instruction::Unknown(_) => (),
        other => panic!("expected Unknown for reserved BIU m=0, got {:?}", other),
    }
}

// J instruction (op0=0x6, n=0)

#[test]
fn decode_j() {
    // imm18=0 → offset = 0+4 = 4
    assert_eq!(decode(j_pack(0)), Instruction::J { offset: 4 });
    // imm18=0x10 → offset = 16+4 = 20
    assert_eq!(decode(j_pack(0x10)), Instruction::J { offset: 20 });
    // imm18=0x3_FFFE (sign-extended = -2) → offset = -2+4 = 2
    assert_eq!(decode(j_pack(0x3_FFFE)), Instruction::J { offset: 2 });
}

// ── Task B8: CALL / CALLX / RET / RETW / JX / RFE+family / SYSCALL ──────────
//
// CALLN format (op0=5): bits[5:4]=n, bits[23:6]=imm18 (signed word offset from PC+4).
// ST0 format (op0=0, op1=0, op2=0): dispatches on r field:
//   r=0, t=8      → RET      (s ignored)
//   r=0, t=9      → RETW     (s ignored)
//   r=0, s=<as>, t=0xA → JX
//   r=0, s=<as>, t=0xC..F → CALLX0/4/8/12
//   r=2, s=0, t=0..3,0xC,0xD,0xF → ISYNC/RSYNC/ESYNC/DSYNC/MEMW/EXTW/NOP
//   r=3, s=0,t=0 → RFE; s=2,t=0 → RFDE; s=4,t=0 → RFWO; s=5,t=0 → RFWU
//   r=3, s=<level>,t=1 → RFI
//   r=4, s=<imm_s>,t=<imm_t> → BREAK
//   r=5, s=0,t=0 → SYSCALL
//
// All encoding values HW-oracle verified against xtensa-esp-elf-objdump.

/// Pack a CALLN instruction: op0=5, n at bits[5:4], imm18 at bits[23:6].
fn calln_pack(n: u32, imm18: u32) -> u32 {
    0x5 | ((n & 0x3) << 4) | ((imm18 & 0x3_FFFF) << 6)
}

/// Pack a ST0 instruction (op0=0, op1=0, op2=0): r at bits[15:12], s at bits[11:8], t at bits[7:4].
fn st0_pack(r: u32, s: u32, t: u32) -> u32 {
    (r << 12) | (s << 8) | (t << 4) | 0x0
}

#[test]
fn decode_call0_4_8_12() {
    // CALL0 imm18=0 → offset = 0 * 4 = 0 bytes
    // HW: "call0 0x4" when PC=0 (target = PC+4+0 = 4); offset stored = 0
    assert_eq!(decode(calln_pack(0, 0)), Instruction::Call0 { offset: 0 });

    // CALL4 imm18=1 → offset = 1 * 4 = 4 bytes
    // HW: "call4 0x8" when PC=0 (target = PC+4+4 = 8)
    assert_eq!(decode(calln_pack(1, 1)), Instruction::Call4 { offset: 4 });

    // CALL8 imm18=0x10 → offset = 0x10 * 4 = 0x40 = 64 bytes
    // HW: "call8 0x44" when PC=0 (target = PC+4+64 = 68 = 0x44)
    assert_eq!(decode(calln_pack(2, 0x10)), Instruction::Call8 { offset: 0x40 });

    // CALL12 imm18=0x3FFFF (all ones = signed -1) → offset = -1 * 4 = -4 bytes
    // HW: "call12 0x0" when PC=0 (target = PC+4-4 = 0)
    assert_eq!(decode(calln_pack(3, 0x3FFFF)), Instruction::Call12 { offset: -4 });
}

#[test]
fn decode_callx0_4_8_12() {
    // CALLX0 as_=5: r=0, s=5, t=0xC
    // HW: 0x0C0500 → "callx0 a5"
    assert_eq!(decode(st0_pack(0, 5, 0xC)), Instruction::Callx0 { as_: 5 });
    // CALLX4 as_=5: r=0, s=5, t=0xD
    // HW: 0x0D0500 → "callx4 a5"
    assert_eq!(decode(st0_pack(0, 5, 0xD)), Instruction::Callx4 { as_: 5 });
    // CALLX8 as_=5: r=0, s=5, t=0xE
    // HW: 0x0E0500 → "callx8 a5"
    assert_eq!(decode(st0_pack(0, 5, 0xE)), Instruction::Callx8 { as_: 5 });
    // CALLX12 as_=5: r=0, s=5, t=0xF
    // HW: 0x0F0500 → "callx12 a5"
    assert_eq!(decode(st0_pack(0, 5, 0xF)), Instruction::Callx12 { as_: 5 });
}

#[test]
fn decode_ret_retw() {
    // RET: r=0, s=0, t=8
    // HW: 0x000080 → "ret"
    assert_eq!(decode(st0_pack(0, 0, 8)), Instruction::Ret);
    // RETW: r=0, s=0, t=9
    // HW: 0x000090 → "retw"
    assert_eq!(decode(st0_pack(0, 0, 9)), Instruction::Retw);
    // RET with non-zero s (s field ignored per ISA RM): r=0, s=3, t=8
    // HW: confirmed "ret" regardless of s
    assert_eq!(decode(st0_pack(0, 3, 8)), Instruction::Ret);
    // RETW with non-zero s: r=0, s=3, t=9
    assert_eq!(decode(st0_pack(0, 3, 9)), Instruction::Retw);
}

#[test]
fn decode_jx() {
    // JX as_=4: r=0, s=4, t=0xA
    // HW: 0x0A0400 → "jx a4"
    assert_eq!(decode(st0_pack(0, 4, 0xA)), Instruction::Jx { as_: 4 });
    // JX as_=1: r=0, s=1, t=0xA
    // HW: 0x0A0100 → "jx a1"
    assert_eq!(decode(st0_pack(0, 1, 0xA)), Instruction::Jx { as_: 1 });
}

#[test]
fn decode_isync_rsync_esync_dsync() {
    // ISYNC: r=2, s=0, t=0
    // HW: 0x002000 → "isync"
    assert_eq!(decode(st0_pack(2, 0, 0)), Instruction::Isync);
    // RSYNC: r=2, s=0, t=1
    // HW: 0x002010 → "rsync"
    assert_eq!(decode(st0_pack(2, 0, 1)), Instruction::Rsync);
    // ESYNC: r=2, s=0, t=2
    // HW: 0x002020 → "esync"
    assert_eq!(decode(st0_pack(2, 0, 2)), Instruction::Esync);
    // DSYNC: r=2, s=0, t=3
    // HW: 0x002030 → "dsync"
    assert_eq!(decode(st0_pack(2, 0, 3)), Instruction::Dsync);
    // MEMW: r=2, s=0, t=0xC
    // HW: 0x0020C0 → "memw"
    assert_eq!(decode(st0_pack(2, 0, 0xC)), Instruction::Memw);
    // EXTW: r=2, s=0, t=0xD
    // HW: 0x0020D0 → "extw"
    assert_eq!(decode(st0_pack(2, 0, 0xD)), Instruction::Extw);
    // NOP: r=2, s=0, t=0xF
    // HW: 0x0020F0 → "nop"
    assert_eq!(decode(st0_pack(2, 0, 0xF)), Instruction::Nop);
}

#[test]
fn decode_rfe_rfde_rfwo_rfwu() {
    // RFE: r=3, s=0, t=0
    // HW: 0x003000 → "rfe"
    assert_eq!(decode(st0_pack(3, 0, 0)), Instruction::Rfe);
    // RFDE: r=3, s=2, t=0
    // HW: 0x003200 → "rfde"
    assert_eq!(decode(st0_pack(3, 2, 0)), Instruction::Rfde);
    // RFWO: r=3, s=4, t=0
    // HW: 0x003400 → "rfwo"
    assert_eq!(decode(st0_pack(3, 4, 0)), Instruction::Rfwo);
    // RFWU: r=3, s=5, t=0
    // HW: 0x003500 → "rfwu"
    assert_eq!(decode(st0_pack(3, 5, 0)), Instruction::Rfwu);
}

#[test]
fn decode_rfi_level() {
    // RFI takes interrupt level in s field; t=1.
    // RFI level=0: r=3, s=0, t=1
    // HW: 0x003010 → "rfi 0"
    assert_eq!(decode(st0_pack(3, 0, 1)), Instruction::Rfi { level: 0 });
    // RFI level=2: r=3, s=2, t=1
    // HW: 0x003210 → "rfi 2"
    assert_eq!(decode(st0_pack(3, 2, 1)), Instruction::Rfi { level: 2 });
    // RFI level=7: r=3, s=7, t=1
    // HW: 0x003710 → "rfi 7"
    assert_eq!(decode(st0_pack(3, 7, 1)), Instruction::Rfi { level: 7 });
    // RFI level=15: r=3, s=15, t=1
    // HW: 0x003F10 → "rfi 15"
    assert_eq!(decode(st0_pack(3, 15, 1)), Instruction::Rfi { level: 15 });
}

#[test]
fn decode_syscall() {
    // SYSCALL: r=5, s=0, t=0
    // HW: 0x005000 → "syscall"
    assert_eq!(decode(st0_pack(5, 0, 0)), Instruction::Syscall);
}

#[test]
fn decode_break_st0() {
    // BREAK imm_s=0, imm_t=0: r=4, s=0, t=0
    // HW: 0x004000 → "break 0, 0"
    assert_eq!(decode(st0_pack(4, 0, 0)), Instruction::Break { imm_s: 0, imm_t: 0 });
    // BREAK imm_s=3, imm_t=5: r=4, s=3, t=5
    // HW: 0x004350 → "break 3, 5"
    assert_eq!(decode(st0_pack(4, 3, 5)), Instruction::Break { imm_s: 3, imm_t: 5 });
}

// ── Task D8: Narrow (Code Density) decoder tests ──────────────────────────────
//
// All byte values are HW-oracle verified via xtensa-esp32s3-elf-as + objdump.
// Field layout: bits[3:0]=op0, bits[7:4]=s, bits[11:8]=t, bits[15:12]=r.
// Note: for each instruction, which role s/t/r plays differs — see comments.

#[test]
fn test_decode_narrow_l32i_n() {
    // l32i.n a3, a4, 4  →  0x1438
    // at=s=3, as_=t=4, imm=r<<2=1<<2=4
    // HW-oracle: addr 0x0e: 38 14
    assert_eq!(decode_narrow(0x1438), Instruction::L32i { at: 3, as_: 4, imm: 4 });
    // l32i.n a3, a4, 0  →  imm=0: r=0
    assert_eq!(decode_narrow(0x0438), Instruction::L32i { at: 3, as_: 4, imm: 0 });
    // l32i.n a3, a4, 60  →  imm=60: r=15
    assert_eq!(decode_narrow(0xF438), Instruction::L32i { at: 3, as_: 4, imm: 60 });
}

#[test]
fn test_decode_narrow_s32i_n() {
    // s32i.n a3, a4, 8  →  0x2439
    // at=s=3, as_=t=4, imm=r<<2=2<<2=8
    // HW-oracle: addr 0x10: 39 24
    assert_eq!(decode_narrow(0x2439), Instruction::S32i { at: 3, as_: 4, imm: 8 });
    // s32i.n a3, a4, 0  →  imm=0: r=0
    assert_eq!(decode_narrow(0x0439), Instruction::S32i { at: 3, as_: 4, imm: 0 });
    // s32i.n a3, a4, 60  →  imm=60: r=15
    assert_eq!(decode_narrow(0xF439), Instruction::S32i { at: 3, as_: 4, imm: 60 });
}

#[test]
fn test_decode_narrow_add_n() {
    // add.n a3, a4, a5  →  0x345a
    // ar=r=3, as_=t=4, at=s=5
    // HW-oracle: addr 0x00: 5a 34
    assert_eq!(decode_narrow(0x345a), Instruction::Add { ar: 3, as_: 4, at: 5 });
    // add.n a1, a2, a3  →  r=1, t=2, s=3  →  hw = (1<<12)|(2<<8)|(3<<4)|0xA = 0x123a
    assert_eq!(decode_narrow(0x123a), Instruction::Add { ar: 1, as_: 2, at: 3 });
}

#[test]
fn test_decode_narrow_addi_n() {
    // addi.n a3, a4, 5  →  0x345b
    // at=r=3, as_=t=4, imm=sext4_nonzero(s=5)=5
    // HW-oracle: addr 0x02: 5b 34
    assert_eq!(decode_narrow(0x345b), Instruction::Addi { at: 3, as_: 4, imm8: 5 });
    // addi.n a3, a4, -1  →  0x340b: s=0 encodes imm=-1
    // HW-oracle: addr 0x04: 0b 34
    assert_eq!(decode_narrow(0x340b), Instruction::Addi { at: 3, as_: 4, imm8: -1 });
    // addi.n a3, a4, 15  →  maximum positive imm (s=0xF)
    assert_eq!(decode_narrow(0x34fb), Instruction::Addi { at: 3, as_: 4, imm8: 15 });
    // addi.n a3, a4, 1  →  minimum positive imm (s=1)
    assert_eq!(decode_narrow(0x341b), Instruction::Addi { at: 3, as_: 4, imm8: 1 });
}

#[test]
fn test_decode_narrow_mov_n() {
    // mov.n a3, a4  →  0x043d
    // MOV.N = OR ar=s=3, as_=t=4, at=t=4  (OR with same src twice)
    // HW-oracle: addr 0x06: 3d 04
    assert_eq!(decode_narrow(0x043d), Instruction::Or { ar: 3, as_: 4, at: 4 });
    // mov.n a0, a1  →  r=0, t=1, s=0  →  hw = (0<<12)|(1<<8)|(0<<4)|0xD = 0x010d
    assert_eq!(decode_narrow(0x010d), Instruction::Or { ar: 0, as_: 1, at: 1 });
}

#[test]
fn test_decode_narrow_movi_n_positive() {
    // movi.n a3, 5  →  0x530c: t=3=at, s=0, r=5; raw7=(0<<4)|5=5; imm=5
    // HW-oracle: addr 0x08: 0c 53
    assert_eq!(decode_narrow(0x530c), Instruction::Movi { at: 3, imm: 5 });
    // movi.n a3, 0  →  0x030c: s=0, r=0; raw7=0; imm=0
    assert_eq!(decode_narrow(0x030c), Instruction::Movi { at: 3, imm: 0 });
    // movi.n a3, 63  →  0xf33c: s=3, r=15; raw7=(3<<4)|15=63; imm=63
    assert_eq!(decode_narrow(0xf33c), Instruction::Movi { at: 3, imm: 63 });
    // movi.n a3, 95  →  0xf35c: s=5, r=15; raw7=(5<<4)|15=95; imm=95 (POSITIVE despite bit6=1)
    // HW-oracle: addr 0x0c: 5c f3
    assert_eq!(decode_narrow(0xf35c), Instruction::Movi { at: 3, imm: 95 });
    // movi.n a3, 64  →  0x034c: s=4, r=0; raw7=64; imm=64 (not -64!)
    assert_eq!(decode_narrow(0x034c), Instruction::Movi { at: 3, imm: 64 });
}

#[test]
fn test_decode_narrow_movi_n_negative() {
    // movi.n a3, -32  →  0x036c: s=6, r=0; raw7=(6<<4)|0=96; 96>=96 → imm=96-128=-32
    // HW-oracle: addr 0x0a: 6c 03
    assert_eq!(decode_narrow(0x036c), Instruction::Movi { at: 3, imm: -32 });
    // movi.n a3, -1  →  0xf37c: s=7, r=15; raw7=(7<<4)|15=127; 127>=96 → imm=127-128=-1
    assert_eq!(decode_narrow(0xf37c), Instruction::Movi { at: 3, imm: -1 });
    // movi.n a3, -16  →  0x037c: s=7, r=0; raw7=112; 112>=96 → imm=112-128=-16
    assert_eq!(decode_narrow(0x037c), Instruction::Movi { at: 3, imm: -16 });
    // boundary: raw7=95 → positive (95 < 96)
    assert_eq!(decode_narrow(0xf35c), Instruction::Movi { at: 3, imm: 95 });
    // boundary: raw7=96 → negative
    assert_eq!(decode_narrow(0x036c), Instruction::Movi { at: 3, imm: -32 });
}

#[test]
fn test_decode_narrow_beqz_n() {
    // HW-oracle bytes from xtensa-esp32s3-elf-as (correct formula: ((b4<<4)|r)+4).
    // beqz.n a3, +4  →  hw=0x038c [bytes 8c 03]: r=0, b4=0 → offset=4
    assert_eq!(decode_narrow(0x038c), Instruction::Beqz { as_: 3, offset: 4 });
    // beqz.n a2, +4  →  hw=0x028c [bytes 8c 02]: r=0, b4=0, as_=2 → offset=4
    assert_eq!(decode_narrow(0x028c), Instruction::Beqz { as_: 2, offset: 4 });
    // beqz.n a3, +6  →  hw=0x238c [bytes 8c 23]: r=2, b4=0 → offset=6
    assert_eq!(decode_narrow(0x238c), Instruction::Beqz { as_: 3, offset: 6 });
    // beqz.n a3, +18 →  hw=0xe38c [bytes 8c e3]: r=14, b4=0 → offset=18
    assert_eq!(decode_narrow(0xe38c), Instruction::Beqz { as_: 3, offset: 18 });
    // beqz.n a3, +19 →  hw=0xf38c [bytes 8c f3]: r=15, b4=0 → offset=19 (b4=0 boundary)
    assert_eq!(decode_narrow(0xf38c), Instruction::Beqz { as_: 3, offset: 19 });
    // beqz.n a3, +21 →  hw=0x139c [bytes 9c 13]: r=1,  b4=1 → offset=21 (b4=1 minimum)
    assert_eq!(decode_narrow(0x139c), Instruction::Beqz { as_: 3, offset: 21 });
    // beqz.n a3, +35 →  hw=0xf39c [bytes 9c f3]: r=15, b4=1 → offset=35 (maximum)
    assert_eq!(decode_narrow(0xf39c), Instruction::Beqz { as_: 3, offset: 35 });
}

#[test]
fn test_decode_narrow_bnez_n() {
    // HW-oracle bytes from xtensa-esp32s3-elf-as (same formula as BEQZ.N; s[2]=1 distinguishes BNEZ.N).
    // bnez.n a3, +4  →  hw=0x03cc [bytes cc 03]: r=0, b4=0 → offset=4
    assert_eq!(decode_narrow(0x03cc), Instruction::Bnez { as_: 3, offset: 4 });
    // bnez.n a4, +4  →  hw=0x04cc [bytes cc 04]: r=0, b4=0, as_=4 → offset=4
    assert_eq!(decode_narrow(0x04cc), Instruction::Bnez { as_: 4, offset: 4 });
    // bnez.n a3, +6  →  hw=0x23cc [bytes cc 23]: r=2, b4=0 → offset=6
    assert_eq!(decode_narrow(0x23cc), Instruction::Bnez { as_: 3, offset: 6 });
    // bnez.n a3, +18 →  hw=0xe3cc [bytes cc e3]: r=14, b4=0 → offset=18
    assert_eq!(decode_narrow(0xe3cc), Instruction::Bnez { as_: 3, offset: 18 });
    // bnez.n a3, +35 →  hw=0xf3dc [bytes dc f3]: r=15, b4=1 → offset=35 (maximum)
    // HW-oracle: xtensa-esp32s3-elf-as assembles bnez.n a3, +35 → 0xf3dc
    assert_eq!(decode_narrow(0xf3dc), Instruction::Bnez { as_: 3, offset: 35 });
}

#[test]
fn test_decode_narrow_ret_n() {
    // ret.n  →  0xf00d: r=0xF, s=0, t=0
    // HW-oracle: addr 0x14: 0d f0
    assert_eq!(decode_narrow(0xf00d), Instruction::Ret);
}

#[test]
fn test_decode_narrow_retw_n() {
    // retw.n  →  0xf01d: r=0xF, s=1, t=0
    // HW-oracle: addr 0x16: 1d f0
    assert_eq!(decode_narrow(0xf01d), Instruction::Retw);
}

#[test]
fn test_decode_narrow_break_n() {
    // break.n 0  →  0xf02d: r=0xF, s=2, t=0  → Break{imm_s=0,imm_t=0}
    // HW-oracle: addr 0x18: 2d f0
    assert_eq!(decode_narrow(0xf02d), Instruction::Break { imm_s: 0, imm_t: 0 });
}

#[test]
fn test_decode_narrow_nop_n() {
    // nop.n  →  0xf03d: r=0xF, s=3, t=0
    // HW-oracle: addr 0x12: 3d f0
    assert_eq!(decode_narrow(0xf03d), Instruction::Nop);
}

#[test]
fn test_decode_narrow_ill_n() {
    // ill.n  →  0xf06d: r=0xF, s=6, t=0
    // HW-oracle: addr 0x1a: 6d f0
    assert_eq!(decode_narrow(0xf06d), Instruction::Ill);
}
