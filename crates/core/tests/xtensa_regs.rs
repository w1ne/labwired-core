use labwired_core::cpu::xtensa_regs::{ArFile, Ps};

#[test]
fn logical_a0_maps_to_physical_0_when_windowbase_zero() {
    let mut f = ArFile::new();
    f.set_windowbase(0);
    f.write_logical(0, 0xDEAD_BEEF);
    assert_eq!(f.read_logical(0), 0xDEAD_BEEF);
    assert_eq!(f.physical(0), 0xDEAD_BEEF);
}

#[test]
fn windowbase_rotation_shifts_by_four() {
    let mut f = ArFile::new();
    f.set_windowbase(1); // logical a0 → physical 4
    f.write_logical(0, 0x1111_2222);
    assert_eq!(f.physical(4), 0x1111_2222);
    f.set_windowbase(0);
    assert_eq!(f.read_logical(4), 0x1111_2222);
}

#[test]
fn logical_index_15_is_valid() {
    let mut f = ArFile::new();
    f.set_windowbase(5);
    f.write_logical(15, 0xAAAA);
    assert_eq!(f.physical((5 * 4 + 15) % 64), 0xAAAA);
}

#[test]
fn windowstart_bit_tracks_allocated_frames() {
    let mut f = ArFile::new();
    f.set_windowstart(0);
    f.set_windowstart_bit(3, true);
    assert!(f.windowstart_bit(3));
    f.set_windowstart_bit(3, false);
    assert!(!f.windowstart_bit(3));
}

#[test]
fn ps_fielded_readback() {
    let mut ps = Ps::from_raw(0);
    ps.set_intlevel(5);
    ps.set_excm(true);
    ps.set_woe(true);
    assert_eq!(ps.intlevel(), 5);
    assert!(ps.excm());
    assert!(ps.woe());
    let raw = ps.as_raw();
    let ps2 = Ps::from_raw(raw);
    assert_eq!(ps2.intlevel(), 5);
    assert!(ps2.excm());
    assert!(ps2.woe());
}

#[test]
fn ar_file_new_has_windowstart_bit_0() {
    // Per Xtensa reset spec: WindowStart initial = 0x1 (a0..a3 frame active)
    let f = ArFile::new();
    assert!(f.windowstart_bit(0));
    for i in 1..16u8 {
        assert!(!f.windowstart_bit(i), "bit {} should be clear at reset", i);
    }
}
