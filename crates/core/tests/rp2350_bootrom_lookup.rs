// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! RP2350 `rom_func_lookup` must enter a boot ROM, not the flash image.
//!
//! pico-sdk (RP2350, Arm) reads `rom_table_lookup` as the halfword at `0x16`
//! (datasheet 5.4.1, Table 453) and branches to it with the code in r0 and an
//! `RT_FLAG_FUNC_ARM_{SEC,NONSEC}` mask in r1. With no ROM window the
//! Cortex-M flash alias serves the image's own vector table at 0: the
//! halfword at `0x16` is the top half of a `0x1000_xxxx` vector, `0x1000`,
//! so the lookup jumps to flash offset `0x1000`. On the arduino-pico
//! rp2350-zero image that is OTA-stage data, and the core takes a UsageFault
//! (UNDEFINSTR) at `0x1008` that escalates to HardFault — the boot fault
//! reported on 2026-11-01.
//!
//! These tests run the exact `rom_func_lookup` the arduino-pico image links
//! against a real `rp2350-zero` bus, then call the returned ROM functions.

mod common;
use common::root;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Cpu, Machine};
use std::sync::Mutex;

/// Serialises `LABWIRED_RP2350_BOOTROM` mutation (process-global).
static ENV_LOCK: Mutex<()> = Mutex::new(());
const ENV_BOOTROM: &str = "LABWIRED_RP2350_BOOTROM";

/// `rom_func_lookup` from the arduino-pico 6.1.0 rp2350 image (0x100054a2):
/// the halfword at 0x16 is the lookup, TT picks the Arm-S / Arm-NS mask.
const LOOKUP_ADDR: u32 = 0x2000_1000;
const LOOKUP: [u8; 20] = [
    0x00, 0x23, // movs  r3, #0
    0xdb, 0x8a, // ldrh  r3, [r3, #22]
    0x00, 0x22, // movs  r2, #0
    0x42, 0xe8, 0x00, 0xf2, // tt r2, r2
    0x52, 0x02, // lsls  r2, r2, #9
    0x54, 0xbf, // ite   pl
    0x10, 0x21, // movpl r1, #16  (RT_FLAG_FUNC_ARM_NONSEC)
    0x04, 0x21, // movmi r1, #4   (RT_FLAG_FUNC_ARM_SEC)
    0x18, 0x47, // bx    r3
];

/// `b .` — where the saved LR returns.
const SENTINEL: u32 = 0x2000_2000;
const STACK: u32 = 0x2007_F000;
const BUF: u32 = 0x2000_3000;

/// First 0x20 bytes of the arduino-pico rp2350-zero image (its OTA stage's
/// vector table): SP 0x20082000, then 0x1000_01xx handlers. The halfword at
/// 0x16 is 0x1000.
const IMAGE_HEAD: [u8; 0x20] = [
    0x00, 0x20, 0x08, 0x20, 0x47, 0x01, 0x00, 0x10, 0x1b, 0x01, 0x00, 0x10, 0x1d, 0x01, 0x00, 0x10,
    0x11, 0x01, 0x00, 0x10, 0x11, 0x01, 0x00, 0x10, 0x11, 0x01, 0x00, 0x10, 0x11, 0x01, 0x00, 0x10,
];
const FLASH_ALIAS_TARGET: u32 = 0x1000;

const fn code(a: u8, b: u8) -> u32 {
    a as u32 | ((b as u32) << 8)
}

fn load_zero() -> (ChipDescriptor, SystemManifest) {
    let chip_path = root("configs/chips/rp2350.yaml");
    let system_path = root("configs/systems/rp2350-zero.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("rp2350 chip");
    let mut manifest = SystemManifest::from_file(&system_path).expect("rp2350-zero system");
    manifest.chip = chip_path.to_str().expect("utf8 chip path").to_string();
    (chip, manifest)
}

fn machine() -> Machine<CortexM> {
    let (chip, manifest) = load_zero();
    let mut bus = labwired_core::bus::SystemBus::from_config(&chip, &manifest).expect("rp2350 bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut m = Machine::new(cpu, bus);
    for (i, b) in IMAGE_HEAD.iter().enumerate() {
        m.bus.write_u8(0x1000_0000 + i as u64, *b).unwrap();
    }
    for (i, b) in LOOKUP.iter().enumerate() {
        m.bus
            .write_u8(u64::from(LOOKUP_ADDR) + i as u64, *b)
            .unwrap();
    }
    m.bus.write_u16(u64::from(SENTINEL), 0xe7fe).unwrap();
    m
}

/// Run from `pc` with r0..r2 until the sentinel. Panics on a fault, on
/// entering the flash alias target, or on no return.
fn call(m: &mut Machine<CortexM>, pc: u32, args: [u32; 3]) -> u32 {
    m.cpu.set_pc(pc & !1);
    m.cpu.set_sp(STACK);
    m.cpu.lr = SENTINEL | 1;
    m.cpu.r0 = args[0];
    m.cpu.r1 = args[1];
    m.cpu.r2 = args[2];
    for step in 0..2000 {
        let pc = m.cpu.get_pc();
        assert!(
            !(FLASH_ALIAS_TARGET..FLASH_ALIAS_TARGET + 0x100).contains(&pc),
            "branched into the flash alias at {pc:#x} (step {step}): no boot ROM at 0x16"
        );
        if pc == SENTINEL {
            let cfsr = m.bus.read_u32(0xE000_ED28).unwrap_or(0);
            assert_eq!(cfsr, 0, "call set CFSR {cfsr:#x}");
            return m.cpu.r0;
        }
        m.step()
            .unwrap_or_else(|e| panic!("step {step} at {:#x}: {e:?}", m.cpu.get_pc()));
    }
    panic!("no return (pc {:#x})", m.cpu.get_pc());
}

fn lookup(m: &mut Machine<CortexM>, c: u32) -> u32 {
    call(m, LOOKUP_ADDR, [c, 0, 0])
}

/// Codes the arduino-pico rp2350 image (crt0 + OTA stage) looks up at boot.
const BOOT_FUNCS: &[(u32, &str)] = &[
    (code(b'S', b'R'), "bootrom_state_reset"),
    (code(b'G', b'S'), "get_sys_info"),
    (code(b'R', b'B'), "reboot"),
    (code(b'I', b'F'), "connect_internal_flash"),
    (code(b'E', b'X'), "flash_exit_xip"),
    (code(b'F', b'C'), "flash_flush_cache"),
    (code(b'C', b'X'), "flash_enter_cmd_xip"),
];

#[test]
fn well_known_words_are_the_rom_not_the_flash_alias() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var(ENV_BOOTROM);
    let m = machine();
    // Table 453: 'M','u',0x02 at 0x10; 16-bit pointers at 0x14/0x16/0x18.
    assert_eq!(m.bus.read_u8(0x10).unwrap(), b'M');
    assert_eq!(m.bus.read_u8(0x11).unwrap(), b'u');
    assert_eq!(m.bus.read_u8(0x12).unwrap(), 0x02);
    let lookup_val = m.bus.read_u16(0x16).unwrap();
    assert_ne!(
        lookup_val, 0x1000,
        "0x16 still reads the flash vector table"
    );
    assert_eq!(
        lookup_val & 1,
        1,
        "rom_table_lookup_val must be a Thumb entry"
    );
    assert!(
        u32::from(lookup_val) < 0x8000,
        "lookup outside the 32 KB ROM"
    );
}

#[test]
fn boot_lookups_return_rom_functions_and_unknown_is_null() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var(ENV_BOOTROM);
    let mut m = machine();
    for &(c, name) in BOOT_FUNCS {
        let f = lookup(&mut m, c);
        assert!(
            f & 1 == 1 && f < 0x8000 && f > 0x1a,
            "{name} ({c:#06x}) -> {f:#x}: not a Thumb ROM entry"
        );
    }
    assert_eq!(
        lookup(&mut m, code(b'Z', b'Z')),
        0,
        "unknown code must be NULL"
    );
    // flash_range_erase is not modelled: NULL, not a silent no-op.
    assert_eq!(lookup(&mut m, code(b'R', b'E')), 0);
}

#[test]
fn bootrom_state_reset_returns_and_get_sys_info_fills_chip_info() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var(ENV_BOOTROM);
    let mut m = machine();

    // runtime_init_bootrom_reset: rom_bootrom_state_reset(STATE_RESET_GLOBAL_STATE).
    let sr = lookup(&mut m, code(b'S', b'R'));
    call(&mut m, sr, [0x4, 0, 0]);

    // _retrieve_unique_id_on_boot: get_sys_info(buf, 9, SYS_INFO_CHIP_INFO).
    let gs = lookup(&mut m, code(b'G', b'S'));
    let words = call(&mut m, gs, [BUF, 9, 0x1]) as i32;
    assert_eq!(
        words, 4,
        "flags word + 3 CHIP_INFO words (datasheet 5.4.8.17)"
    );
    let rd = |m: &mut Machine<CortexM>, i: u64| m.bus.read_u32(u64::from(BUF) + 4 * i).unwrap();
    assert_eq!(rd(&mut m, 0), 0x1, "supported subset = CHIP_INFO");
    let id = u64::from(rd(&mut m, 2)) | (u64::from(rd(&mut m, 3)) << 32);
    assert_ne!(id, 0, "device id must be non-zero (USB serial / unique id)");

    // Unsupported flags are masked out of the first word, not faked.
    let words = call(&mut m, gs, [BUF, 9, 0x10]) as i32;
    assert_eq!(words, 1);
    assert_eq!(rd(&mut m, 0), 0);

    // Too small a buffer for CHIP_INFO: BOOTROM_ERROR_BUFFER_TOO_SMALL.
    assert_eq!(call(&mut m, gs, [BUF, 2, 0x1]) as i32, -13);
}

#[test]
fn flash_devinfo16_ptr_points_at_a_16mb_cs0_devinfo() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var(ENV_BOOTROM);
    let mut m = machine();
    // rom_data_lookup: same lookup with RT_FLAG_DATA (0x40).
    let lookup_val = u32::from(m.bus.read_u16(0x16).unwrap());
    let p = call(&mut m, lookup_val, [code(b'F', b'D'), 0x40, 0]);
    assert_ne!(p, 0, "FD data entry missing");
    let devinfo = m.bus.read_u32(u64::from(p)).unwrap();
    assert_eq!(m.bus.read_u16(u64::from(devinfo)).unwrap(), 0x0C00);
    // A data code is not a function: function masks miss it.
    assert_eq!(call(&mut m, lookup_val, [code(b'F', b'D'), 0x4, 0]), 0);
}

/// `LABWIRED_RP2350_BOOTROM=` (empty) drops the window: 0x16 is the flash
/// alias again, and the lookup lands at 0x1000 — the reported fault path.
#[test]
fn empty_bootrom_env_restores_the_flash_alias_fault_path() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var(ENV_BOOTROM, "");
    let mut m = machine();
    std::env::remove_var(ENV_BOOTROM);
    assert_eq!(m.bus.read_u16(0x16).unwrap(), 0x1000);
    m.cpu.set_pc(LOOKUP_ADDR);
    m.cpu.set_sp(STACK);
    m.cpu.lr = SENTINEL | 1;
    m.cpu.r0 = code(b'S', b'R');
    for _ in 0..=9 {
        if m.cpu.get_pc() == FLASH_ALIAS_TARGET {
            return;
        }
        let _ = m.step();
    }
    panic!("lookup did not reach 0x1000 (pc {:#x})", m.cpu.get_pc());
}
