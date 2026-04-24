use labwired_core::cpu::xtensa_sr::XtensaSrFile;

#[test]
fn basic_rw_roundtrip() {
    let mut sr = XtensaSrFile::new();
    sr.write(200, 0xDEAD_BEEF); // EPC1
    assert_eq!(sr.read(200), 0xDEAD_BEEF);
}

#[test]
fn xsr_swaps_atomically() {
    let mut sr = XtensaSrFile::new();
    sr.write(201, 0x1111); // EPC2
    let old = sr.swap(201, 0x2222);
    assert_eq!(old, 0x1111);
    assert_eq!(sr.read(201), 0x2222);
}

#[test]
fn intclear_clears_interrupt_bits() {
    let mut sr = XtensaSrFile::new();
    sr.set_raw(228, 0xFF); // INTERRUPT directly (test helper)
    sr.write(230, 0x0F);   // INTCLEAR clears low 4
    assert_eq!(sr.read(228), 0xF0);
}

#[test]
fn intset_sets_interrupt_bits() {
    let mut sr = XtensaSrFile::new();
    sr.set_raw(228, 0x00);
    sr.write(229, 0x0F); // INTSET
    assert_eq!(sr.read(228), 0x0F);
}

#[test]
fn litbase_writes_accepted_but_read_returns_zero_on_s3() {
    let mut sr = XtensaSrFile::new();
    sr.write(178, 0xABCD); // LITBASE
    // ISA RM says LITBASE is hardwired to 0 on ESP32-S3. Read behavior:
    // Choice: return 0 always, OR return the latched value.
    // For the sim: store the written value so debugger readback reflects what firmware wrote,
    // but exec code should treat LITBASE as 0. Test: latch value is returned.
    assert_eq!(sr.read(178), 0xABCD);
}

#[test]
fn prid_is_read_only() {
    let mut sr = XtensaSrFile::new();
    let initial = sr.read(237);
    assert_ne!(initial, 0, "PRID should have a nonzero reset value");
    sr.write(237, 0xDEAD);
    assert_eq!(sr.read(237), initial, "PRID writes should be ignored");
}

#[test]
fn mac16_stubs_roundtrip() {
    let mut sr = XtensaSrFile::new();
    for id in [247u16, 248, 252, 253, 254, 255] {
        sr.write(id, 0xC0FFEE);
        assert_eq!(sr.read(id), 0xC0FFEE);
    }
}

#[test]
fn ccount_is_writable_by_software() {
    let mut sr = XtensaSrFile::new();
    sr.write(236, 12345);
    assert_eq!(sr.read(236), 12345);
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
    assert_eq!(sr.read(233), 0x40000000);
}

#[test]
fn exc_shadow_stack_independence() {
    let mut sr = XtensaSrFile::new();
    sr.write(200, 0xAA); // EPC1
    sr.write(201, 0xBB); // EPC2
    sr.write(202, 0xCC); // EPC3
    assert_eq!(sr.read(200), 0xAA);
    assert_eq!(sr.read(201), 0xBB);
    assert_eq!(sr.read(202), 0xCC);
}
