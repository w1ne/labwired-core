// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! KW41Z (MKW41Z4) clock-bring-up conformance: the simulator must satisfy the
//! status-bit poll loops in the NXP MCUXpresso boot path, so unmodified vendor
//! firmware reaches `main` instead of spinning forever.
//!
//! This replays the exact register sequence of `BOARD_RfOscInit` +
//! `CLOCK_SetFeeMode` (from the public NXP `fsl_clock.c`) against an assembled
//! `mkw41z4` bus and asserts every `while (...)` the SDK spins on terminates.
//! It is the deterministic, ELF-independent twin of `test_kw41z_nxp_survival`
//! in `firmware_survival.rs`, which boots a real NXP-vendor-code ELF end to end.
//!
//! Register facts: public CMSIS-SVD `data/NXP/MKW41Z4.svd` and the NXP
//! `fsl_clock.c` / FRDM-KW41Z `clock_config.c` boot sequence.

use labwired_config::ChipDescriptor;
use labwired_core::bus::SystemBus;
use labwired_core::Bus;

// Absolute MMIO addresses on the boot path.
const RSIM_CONTROL: u64 = 0x4005_9000;
const SIM_SDID: u64 = 0x4004_8024;
const MCG_C1: u64 = 0x4006_4000; // 8-bit
const MCG_C4: u64 = 0x4006_4003; // 8-bit
const MCG_S: u64 = 0x4006_4006; // 8-bit
const SIM_COPC: u64 = 0x4004_8100;

// Bit positions used by the SDK poll loops.
const RF_OSC_EN_SHIFT: u32 = 8; // RSIM_CONTROL[11:8]
const RF_OSC_READY: u32 = 1 << 24; // RSIM_CONTROL[24]
const S_OSCINIT0: u8 = 1 << 1;
const S_CLKST_MASK: u8 = 0b11 << 2;
const S_IREFST: u8 = 1 << 4;

fn kw41z_bus() -> SystemBus {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../configs/chips/mkw41z4.yaml");
    let chip = ChipDescriptor::from_file(&path).expect("load mkw41z4 chip");
    let manifest = labwired_config::SystemManifest {
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "kw41z-clock-boot".to_string(),
        chip: path.to_string_lossy().to_string(),
        external_devices: vec![],
        board_io: vec![],
        debug_uart: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
    };
    SystemBus::from_config(&chip, &manifest).expect("assemble mkw41z4 bus")
}

/// `read8` helper: the MCG registers are byte-wide; read the low byte of the
/// containing word.
fn read8(bus: &SystemBus, addr: u64) -> u8 {
    (bus.read_u32(addr & !3).unwrap() >> ((addr & 3) * 8)) as u8
}

#[test]
fn sdid_revid_is_nonzero_to_skip_xcvr_workaround() {
    // BOARD_RfOscInit reads SIM_SDID[REVID]; a 0 revision triggers the rev-1.0
    // XCVR_TSM/ANA_TRIM path this profile does not model. Must be non-zero.
    let bus = kw41z_bus();
    let revid = (bus.read_u32(SIM_SDID).unwrap() >> 12) & 0xF;
    assert_ne!(
        revid, 0,
        "SIM_SDID REVID must be non-zero (Rev 2.0 silicon)"
    );
}

#[test]
fn rf_osc_ready_poll_terminates() {
    // BOARD_RfOscInit:
    //   RSIM->CONTROL = (CONTROL & ~RF_OSC_EN_MASK) | RF_OSC_EN(1);
    //   while ((RSIM->CONTROL & RF_OSC_READY_MASK) == 0) {}
    let mut bus = kw41z_bus();
    assert_eq!(
        bus.read_u32(RSIM_CONTROL).unwrap() & RF_OSC_READY,
        0,
        "RF_OSC_READY must be clear at reset"
    );

    let ctrl = bus.read_u32(RSIM_CONTROL).unwrap();
    let ctrl = (ctrl & !(0xF << RF_OSC_EN_SHIFT)) | (1 << RF_OSC_EN_SHIFT);
    bus.write_u32(RSIM_CONTROL, ctrl).unwrap();

    assert_ne!(
        bus.read_u32(RSIM_CONTROL).unwrap() & RF_OSC_READY,
        0,
        "RF_OSC_READY must set after RF_OSC_EN — the SDK spin loop would hang otherwise"
    );
}

#[test]
fn fee_mode_status_polls_terminate() {
    // CLOCK_SetFeeMode (FLL Engaged External): write C1 selecting the external
    // reference + FLL, then spin until IREFST→0 and CLKST→00.
    let mut bus = kw41z_bus();

    // Reset: FEI — internal ref selected, FLL output.
    assert_eq!(
        read8(&bus, MCG_S) & S_IREFST,
        S_IREFST,
        "reset IREFST=1 (FEI)"
    );
    assert_eq!(read8(&bus, MCG_S) & S_CLKST_MASK, 0, "reset CLKST=00 (FLL)");

    // C1 = CLKS(0=FLL) | FRDIV(5) | IREFS(0=external) = 0x28.
    bus.write_u32(MCG_C1 & !3, {
        let word = bus.read_u32(MCG_C1 & !3).unwrap();
        (word & !0xFF) | 0x28
    })
    .unwrap();

    // while (kMCG_FllSrcExternal != IREFST) — waits IREFST==0.
    assert_eq!(
        read8(&bus, MCG_S) & S_IREFST,
        0,
        "IREFST must clear when IREFS=0 — SetFeeMode would hang otherwise"
    );

    // MCG_C4 readback poll: write DRST_DRS(1)=0x20, then while (C4 != written).
    bus.write_u32(MCG_C4 & !3, {
        let word = bus.read_u32(MCG_C4 & !3).unwrap();
        let shift = (MCG_C4 & 3) * 8;
        (word & !(0xFF << shift)) | (0x20u32 << shift)
    })
    .unwrap();
    assert_eq!(read8(&bus, MCG_C4), 0x20, "MCG_C4 must read back its write");

    // while (kMCG_ClkOutStatFll != CLKST) — waits CLKST==00 (FLL).
    assert_eq!(
        read8(&bus, MCG_S) & S_CLKST_MASK,
        0,
        "CLKST must be 00 (FLL output) in FEE"
    );
}

#[test]
fn oscinit0_sets_when_crystal_selected() {
    // The OSCINIT0 poll inside SetFeeMode is gated on C2[EREFS]; when a firmware
    // does select the crystal oscillator, the bit must come up.
    let mut bus = kw41z_bus();
    let c2 = 0x4006_4001u64;
    assert_eq!(
        read8(&bus, MCG_S) & S_OSCINIT0,
        0,
        "OSCINIT0 clear at reset"
    );
    // C2 = reset(0xC0) | EREFS(bit2).
    bus.write_u32(c2 & !3, {
        let shift = (c2 & 3) * 8;
        let word = bus.read_u32(c2 & !3).unwrap();
        (word & !(0xFF << shift)) | ((0xC0u32 | (1 << 2)) << shift)
    })
    .unwrap();
    assert_eq!(
        read8(&bus, MCG_S) & S_OSCINIT0,
        S_OSCINIT0,
        "OSCINIT0 must set once the crystal oscillator is selected"
    );
}

#[test]
fn sim_high_block_is_mapped() {
    // SystemInit writes SIM_COPC (0x40048100) to disable the COP watchdog; the
    // register lives past the first 4KB page, so the SIM window must cover it.
    // The write must land (read back) rather than fault into open bus.
    let mut bus = kw41z_bus();
    assert_eq!(
        bus.read_u32(SIM_COPC).unwrap(),
        0x0000_000C,
        "SIM_COPC reset value (COP enabled) must be readable in the high block"
    );
    bus.write_u32(SIM_COPC, 0).unwrap();
    assert_eq!(
        bus.read_u32(SIM_COPC).unwrap(),
        0,
        "SIM_COPC=0 (COP disable) must persist — SystemInit relies on it"
    );
}
