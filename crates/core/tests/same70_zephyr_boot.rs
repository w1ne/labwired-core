// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! SAM E70 Xplained: stock Zephyr v3.7.0 hello_world runs its own SoC
//! bring-up, and the registers it leaves behind are the ones silicon would
//! show. `firmware_survival` only checks the console line; this checks the
//! path that got there:
//!
//! - the Cortex-M7 L1 caches went through Zephyr's real sequence: the SoC's
//!   `z_arm_platform_init` invalidates the D-cache by set/way and enables
//!   both caches, then the arch's `z_arm_init_arch_hw_at_boot`
//!   (CONFIG_INIT_ARCH_HW_AT_BOOT) cleans and disables them again, so v3.7
//!   on this board runs cache-off. The set/way loops are sized from the
//!   CCSIDR the SCB reports for the SAM E70 16 KiB D-cache;
//! - the PMC clock tree was walked: the crystal selected, PLLA locked with
//!   Zephyr's MULA 24 / DIVA 1, the UPLL locked, MCK on PLLA with MDIV /2;
//! - CHIPID reads the revision-B CIDR.
//!
//! Fixture: tests/fixtures/same70q21b-zephyr-hello.elf
//! (crates/firmware-sam-e70-xplained-zephyr/build.sh).

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::cortex_m::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Machine};
use std::path::PathBuf;

const SCB_CCR: u64 = 0xE000_ED14;
const SCB_CLIDR: u64 = 0xE000_ED78;
const CCR_IC: u32 = 1 << 17;
const CCR_DC: u32 = 1 << 16;

const PMC: u64 = 0x400E_0600;
const CKGR_MOR: u64 = PMC + 0x20;
const CKGR_PLLAR: u64 = PMC + 0x28;
const PMC_MCKR: u64 = PMC + 0x30;
const PMC_SR: u64 = PMC + 0x68;
const CHIPID_CIDR: u64 = 0x400E_0940;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Boots the fixture for `steps` instructions, returning the machine and the
/// OR of every CCR value seen after an instruction.
fn booted_machine(steps: usize) -> (Machine<CortexM>, u32) {
    let system_path = root().join("configs/systems/sam-e70-xplained.yaml");
    let manifest = SystemManifest::from_file(&system_path).expect("load system");
    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let image =
        labwired_loader::load_elf(&root().join("tests/fixtures/same70q21b-zephyr-hello.elf"))
            .expect("parse fixture");
    machine.load_firmware(&image).expect("load firmware");
    let mut ccr_seen = 0;
    for _ in 0..steps {
        machine.step().expect("step");
        ccr_seen |= machine.bus.read_u32(SCB_CCR).unwrap();
    }
    (machine, ccr_seen)
}

/// Maintenance operations the `l1_cache` window saw, read from its snapshot
/// (no downcast: see `tests::downcast_ratchet`).
fn cache_ops(machine: &Machine<CortexM>) -> u64 {
    let entry = machine
        .bus
        .peripherals
        .iter()
        .find(|p| p.name == "l1_cache")
        .expect("same70q21b declares a cortex_m7_cache");
    assert!(
        entry.dev.l1_cache_geometry().is_some(),
        "l1_cache is not the cache model"
    );
    entry.dev.snapshot()["ops"]
        .as_u64()
        .expect("ops in the cache snapshot")
}

#[test]
fn zephyr_bring_up_walks_the_m7_caches_and_locks_the_plls() {
    let (machine, ccr_seen) = booted_machine(60_000);

    // Caches: CLIDR advertises I + D at level 1. Both were enabled during
    // z_arm_platform_init and disabled again by z_arm_init_arch_hw_at_boot.
    assert_eq!(machine.bus.read_u32(SCB_CLIDR).unwrap(), 0x0900_0003);
    assert_eq!(
        ccr_seen & (CCR_IC | CCR_DC),
        CCR_IC | CCR_DC,
        "caches never enabled"
    );
    let ccr = machine.bus.read_u32(SCB_CCR).unwrap();
    assert_eq!(ccr & (CCR_IC | CCR_DC), 0, "CCR = {ccr:#010x}");
    // Four 128-set x 4-way loops: z_arm_platform_init's SCB_InvalidateDCache
    // and SCB_DisableDCache, the invalidate inside arch_dcache_enable, and
    // z_arm_init_arch_hw_at_boot's SCB_DisableDCache; plus the ICIALLUs.
    let ops = cache_ops(&machine);
    assert!(
        ops >= 4 * 128 * 4,
        "only {ops} cache maintenance operations"
    );

    // PMC: on the crystal (MOSCSEL), PLLA = ONE | MULA 24 | DIVA 1,
    // MCK = PLLA (CSS 2) / MDIV 2, and every source it waited on is ready.
    let mor = machine.bus.read_u32(CKGR_MOR).unwrap();
    assert_ne!(
        mor & (1 << 24),
        0,
        "MOSCSEL: MAINCK on the crystal, MOR = {mor:#010x}"
    );
    let pllar = machine.bus.read_u32(CKGR_PLLAR).unwrap();
    assert_eq!((pllar >> 16) & 0x7FF, 24, "MULA");
    assert_eq!(pllar & 0xFF, 1, "DIVA");
    let mckr = machine.bus.read_u32(PMC_MCKR).unwrap();
    assert_eq!(mckr & 0x3, 2, "CSS = PLLA");
    assert_eq!((mckr >> 8) & 0x3, 1, "MDIV = /2");
    let sr = machine.bus.read_u32(PMC_SR).unwrap();
    for (bit, name) in [
        (0, "MOSCXTS"),
        (1, "LOCKA"),
        (3, "MCKRDY"),
        (6, "LOCKU"),
        (16, "MOSCSELS"),
    ] {
        assert_ne!(sr & (1 << bit), 0, "PMC_SR.{name} (SR = {sr:#010x})");
    }

    assert_eq!(machine.bus.read_u32(CHIPID_CIDR).unwrap(), 0xA102_0E01);
}
