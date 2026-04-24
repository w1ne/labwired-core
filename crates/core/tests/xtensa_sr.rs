use labwired_core::cpu::xtensa_sr::{
    XtensaSrFile,
    EPC1, EPC2, EPC3, DEPC, EPS2, EXCSAVE1,
    CPENABLE, INTERRUPT, INTCLEAR, INTENABLE,
    PS, VECBASE, EXCCAUSE, CCOUNT, PRID, EXCVADDR,
    LITBASE, SCOMPARE1, WINDOWBASE, WINDOWSTART,
    SAR, M0, M1, M2, M3, ACCLO, ACCHI,
};

#[test]
fn sr_ids_match_xtensa_lx7_encoding() {
    // Verified against xtensa-esp-elf-as:
    assert_eq!(EPC1,        177);
    assert_eq!(EPC2,        178);
    assert_eq!(DEPC,        192);
    assert_eq!(EPS2,        194);
    assert_eq!(EXCSAVE1,    209);
    assert_eq!(CPENABLE,    224);
    assert_eq!(INTERRUPT,   226);
    assert_eq!(INTCLEAR,    227);
    assert_eq!(INTENABLE,   228);
    assert_eq!(PS,          230);
    assert_eq!(VECBASE,     231);
    assert_eq!(EXCCAUSE,    232);
    assert_eq!(CCOUNT,      234);
    assert_eq!(PRID,        235);
    assert_eq!(EXCVADDR,    238);
    assert_eq!(LITBASE,     5);
    assert_eq!(SCOMPARE1,   12);
    assert_eq!(WINDOWBASE,  72);
    assert_eq!(WINDOWSTART, 73);
}

#[test]
fn basic_rw_roundtrip() {
    let mut sr = XtensaSrFile::new();
    sr.write(EPC1, 0xDEAD_BEEF);
    assert_eq!(sr.read(EPC1), 0xDEAD_BEEF);
}

#[test]
fn xsr_swaps_atomically() {
    let mut sr = XtensaSrFile::new();
    sr.write(EPC2, 0x1111);
    let old = sr.swap(EPC2, 0x2222);
    assert_eq!(old, 0x1111);
    assert_eq!(sr.read(EPC2), 0x2222);
}

#[test]
fn intclear_clears_interrupt_bits() {
    let mut sr = XtensaSrFile::new();
    sr.set_raw(INTERRUPT, 0xFF); // latch bits directly (test helper)
    sr.write(INTCLEAR, 0x0F);   // INTCLEAR clears low 4
    assert_eq!(sr.read(INTERRUPT), 0xF0);
}

#[test]
fn litbase_write_and_readback() {
    let mut sr = XtensaSrFile::new();
    sr.write(LITBASE, 0xABCD);
    // Sim latches the written value so debugger readback reflects what firmware wrote.
    assert_eq!(sr.read(LITBASE), 0xABCD);
}

#[test]
fn prid_is_read_only() {
    let mut sr = XtensaSrFile::new();
    let initial = sr.read(PRID);
    assert_ne!(initial, 0, "PRID should have a nonzero reset value");
    sr.write(PRID, 0xDEAD);
    assert_eq!(sr.read(PRID), initial, "PRID writes should be ignored");
}

#[test]
fn mac16_stubs_roundtrip() {
    let mut sr = XtensaSrFile::new();
    for id in [ACCLO, ACCHI, M0, M1, M2, M3] {
        sr.write(id, 0xC0FFEE);
        assert_eq!(sr.read(id), 0xC0FFEE, "MAC16 SR id={id} roundtrip failed");
    }
}

#[test]
fn ccount_is_writable_by_software() {
    let mut sr = XtensaSrFile::new();
    sr.write(CCOUNT, 12345);
    assert_eq!(sr.read(CCOUNT), 12345);
}

#[test]
fn reading_unknown_sr_returns_zero_and_is_logged() {
    let sr = XtensaSrFile::new();
    assert_eq!(sr.read(9999), 0);
    // No panic.
}

#[test]
fn vecbase_reset_value() {
    let sr = XtensaSrFile::new();
    // Xtensa ISA RM: VECBASE after reset = 0x40000000 on ESP32-S3 (ROM vectors).
    assert_eq!(sr.read(VECBASE), 0x40000000);
}

#[test]
fn exc_shadow_stack_independence() {
    let mut sr = XtensaSrFile::new();
    sr.write(EPC1, 0xAA);
    sr.write(EPC2, 0xBB);
    sr.write(EPC3, 0xCC);
    assert_eq!(sr.read(EPC1), 0xAA);
    assert_eq!(sr.read(EPC2), 0xBB);
    assert_eq!(sr.read(EPC3), 0xCC);
}

#[test]
fn sar_is_masked_to_6_bits() {
    let mut sr = XtensaSrFile::new();
    sr.write(SAR, 0xFF);
    assert_eq!(sr.read(SAR), 0x3F, "SAR must be masked to 6 bits");
}
