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
