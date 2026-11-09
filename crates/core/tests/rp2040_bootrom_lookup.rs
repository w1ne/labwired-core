// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! RP2040 `rom_func_lookup` must enter the mask ROM, not the stage-2 bootloader.
//!
//! Pico firmware loads the table-lookup pointer as the halfword at `0x18` and
//! `BLX`s it. The W25Q080 stage-2 used by Mbed and Zephyr has `movs r1, #0`
//! (`0x2100`) at that offset. If the 16 KiB bootrom window is missing, the
//! Cortex-M flash alias serves those bytes, the lookup branches to flash
//! offset `0x2100`, and the next instructions BusFault (the Hardware Lab
//! report was PC `0x2106`, called from `rom_func_lookup`).
//!
//! The product path is the in-tree B0 image compiled into the core. These
//! tests build a real `rp2040-pico` bus with no ROM file and no env var, then
//! execute the 16-byte lookup the Arduino Mbed core links.

mod common;
use common::root;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Cpu, Machine};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Serialises cwd / `LABWIRED_RP2040_BOOTROM` mutation. Cargo runs the tests
/// in this binary in parallel, and both are process-global.
static ENV_LOCK: Mutex<()> = Mutex::new(());

const ENV_BOOTROM: &str = "LABWIRED_RP2040_BOOTROM";

/// `rom_func_lookup` as linked by the Arduino Mbed core (16 bytes, Thumb).
const LOOKUP_ADDR: u32 = 0x2000_1000;
const LOOKUP: [u8; 16] = [
    0x14, 0x23, // movs r3, #0x14
    0x10, 0xb5, // push {r4, lr}
    0x01, 0x00, // movs r1, r0
    0x18, 0x88, // ldrh r0, [r3]
    0x04, 0x33, // adds r3, #4
    0x1b, 0x88, // ldrh r3, [r3]
    0x98, 0x47, // blx r3
    0x10, 0xbd, // pop {r4, pc}
];

/// `b .` — where the saved LR returns after the lookup.
const SENTINEL: u32 = 0x2000_2000;
/// Stack in the Pico's SRAM (RAM ends at `0x20042000`).
const STACK: u32 = 0x2003_F000;
/// Official `table_lookup` entry in the in-tree bootrom (thumb bit cleared).
const TABLE_LOOKUP: u32 = 0x1c;
/// Boot2 opcode the alias serves from flash offset `0x18`, and the PC the
/// Hardware Lab fault was reported at a few Thumb instructions later.
const BOOT2_LOOKUP_TARGET: u32 = 0x2100;
const BOOT2_FAULT_PC: u32 = 0x2106;

/// First 0x40 bytes of the W25Q080 stage-2 shared by the Arduino Mbed and
/// Zephyr Pico images. Halfword at `0x18` is `0x2100`.
const W25Q080_BOOT2_PREFIX: [u8; 0x40] = [
    0x00, 0xb5, 0x32, 0x4b, 0x21, 0x20, 0x58, 0x60, 0x98, 0x68, 0x02, 0x21, 0x88, 0x43, 0x98, 0x60,
    0xd8, 0x60, 0x18, 0x61, 0x58, 0x61, 0x2e, 0x4b, 0x00, 0x21, 0x99, 0x60, 0x02, 0x21, 0x59, 0x61,
    0x01, 0x21, 0xf0, 0x22, 0x99, 0x50, 0x2b, 0x49, 0x19, 0x60, 0x01, 0x21, 0x99, 0x60, 0x35, 0x20,
    0x00, 0xf0, 0x44, 0xf8, 0x02, 0x22, 0x90, 0x42, 0x14, 0xd0, 0x06, 0x21, 0x19, 0x66, 0x00, 0xf0,
];

/// `(code, pointer, name)` from the in-tree B0 table. `P3` and `ZZ` are absent
/// and must come back as NULL rather than a boot2 address.
const CODES: &[(u32, u32, &str)] = &[
    (0x534d, 0x39, "MS memset"),
    (0x3453, 0x39, "S4 memset4"),
    (0x434d, 0x49, "MC memcpy"),
    (0x3443, 0x49, "C4 memcpy44"),
    (0x4649, 0x35, "IF connect_internal_flash"),
    (0x5845, 0x35, "EX flash_exit_xip"),
    (0x4552, 0x0113, "RE flash_range_erase"),
    (0x5052, 0x012b, "RP flash_range_program"),
    (0x4346, 0x35, "FC flash_flush_cache"),
    (0x5843, 0x35, "CX flash_enter_cmd_xip"),
    (0x4255, 0x31, "UB reset_usb_boot"),
    (0x5657, 0x31, "WV wait_for_vector"),
    (0x3350, 0x00bd, "P3 popcount32"),
    (0x3352, 0x00d3, "R3 reverse32"),
    (0x334c, 0x00e7, "L3 clz32"),
    (0x3354, 0x00fb, "T3 ctz32"),
    (0x5a5a, 0x00, "ZZ unknown"),
];

fn load_pico() -> (ChipDescriptor, SystemManifest) {
    let chip_path = root("configs/chips/rp2040.yaml");
    let system_path = root("configs/systems/rp2040-pico.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("rp2040 chip");
    let mut manifest = SystemManifest::from_file(&system_path).expect("rp2040-pico system");
    manifest.chip = chip_path.to_str().expect("utf8 chip path").to_string();
    (chip, manifest)
}

fn write_bytes(bus: &mut labwired_core::bus::SystemBus, addr: u32, bytes: &[u8]) {
    for (i, byte) in bytes.iter().enumerate() {
        bus.write_u8(u64::from(addr) + i as u64, *byte)
            .unwrap_or_else(|e| panic!("write {addr:#x}+{i}: {e:?}"));
    }
}

fn plant_lookup(machine: &mut Machine<CortexM>) {
    write_bytes(&mut machine.bus, LOOKUP_ADDR, &LOOKUP);
    machine.bus.write_u16(u64::from(SENTINEL), 0xe7fe).unwrap();
}

fn plant_boot2(machine: &mut Machine<CortexM>) {
    write_bytes(&mut machine.bus, 0x1000_0000, &W25Q080_BOOT2_PREFIX);
}

/// Run one lookup. Returns `r0`. Panics if execution enters the flash alias
/// (`0x2100` / `0x2106`) or never reaches the sentinel.
fn run_lookup(machine: &mut Machine<CortexM>, code: u32) -> u32 {
    machine.cpu.set_pc(LOOKUP_ADDR);
    machine.cpu.set_sp(STACK);
    machine.cpu.lr = SENTINEL | 1;
    machine.cpu.r0 = code;
    let mut entered_table = None;
    for step in 0..400 {
        let pc = machine.cpu.get_pc();
        assert_ne!(
            pc, BOOT2_LOOKUP_TARGET,
            "rom_func_lookup BLX'd the boot2 halfword at 0x18 (code {code:#x}, step {step})"
        );
        assert_ne!(
            pc, BOOT2_FAULT_PC,
            "rom_func_lookup entered the flash image at the reported fault PC (code {code:#x})"
        );
        if pc == TABLE_LOOKUP {
            entered_table.get_or_insert(step);
        }
        if pc == SENTINEL {
            let at = entered_table.expect("returned without entering table_lookup");
            assert_eq!(
                at, 7,
                "the BLX (7th instruction) must land on the ROM table_lookup"
            );
            return machine.cpu.r0;
        }
        machine.step().unwrap_or_else(|e| {
            panic!(
                "lookup step {step} failed: {e:?} (pc {:#x}, code {code:#x})",
                machine.cpu.get_pc()
            )
        });
    }
    panic!(
        "lookup did not return (pc {:#x}, code {code:#x})",
        machine.cpu.get_pc()
    );
}

struct EnvGuard {
    cwd: PathBuf,
    bootrom: Option<String>,
    manifest_dir: Option<String>,
}

impl EnvGuard {
    /// Hide the on-disk ROM: empty cwd, and neither env var the loader searches.
    fn hide_bootrom_file() -> Self {
        let guard = Self {
            cwd: std::env::current_dir().expect("cwd"),
            bootrom: std::env::var(ENV_BOOTROM).ok(),
            manifest_dir: std::env::var("CARGO_MANIFEST_DIR").ok(),
        };
        let dir = std::env::temp_dir().join(format!("lw-rp2040-bootrom-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::env::set_current_dir(&dir).expect("chdir");
        std::env::remove_var(ENV_BOOTROM);
        std::env::remove_var("CARGO_MANIFEST_DIR");
        guard
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.cwd);
        restore(ENV_BOOTROM, self.bootrom.as_deref());
        restore("CARGO_MANIFEST_DIR", self.manifest_dir.as_deref());
        let dir = std::env::temp_dir().join(format!("lw-rp2040-bootrom-{}", std::process::id()));
        if dir != self.cwd {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

fn restore(key: &str, value: Option<&str>) {
    match value {
        Some(v) => std::env::set_var(key, v),
        None => std::env::remove_var(key),
    }
}

fn machine_from(chip: &ChipDescriptor, manifest: &SystemManifest) -> Machine<CortexM> {
    let mut bus = labwired_core::bus::SystemBus::from_config(chip, manifest).expect("rp2040 bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    Machine::new(cpu, bus)
}

/// Unset env and a cwd that cannot see `roms/rp2040/bootrom.bin` must still
/// map the compiled-in image. Stage-2 in flash must not win the halfword at
/// `0x18`, and the lookup must return the B0 table pointers.
#[test]
fn unset_env_embeds_bootrom_and_lookup_returns_table_pointers() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = EnvGuard::hide_bootrom_file();
    // The file search must actually miss. `env!` is compile-time and is not
    // what `from_config` reads.
    assert!(std::env::var("CARGO_MANIFEST_DIR").is_err());
    assert!(std::env::var(ENV_BOOTROM).is_err());
    assert!(!Path::new("roms/rp2040/bootrom.bin").is_file());
    assert!(!Path::new("crates/core/roms/rp2040/bootrom.bin").is_file());

    let (chip, manifest) = load_pico();
    let mut machine = machine_from(&chip, &manifest);
    plant_boot2(&mut machine);
    plant_lookup(&mut machine);

    assert_eq!(
        machine.bus.read_u16(0x1000_0018).unwrap(),
        0x2100,
        "stage-2 was stored at the flash XIP address"
    );
    assert_eq!(
        machine.bus.read_u16(0x14).unwrap(),
        0x005c,
        "bootrom function table must win over the boot2 halfword at 0x14"
    );
    assert_eq!(
        machine.bus.read_u16(0x18).unwrap(),
        0x001d,
        "bootrom table_lookup pointer must win over boot2's 0x2100"
    );

    for &(code, expect, name) in CODES {
        let got = run_lookup(&mut machine, code);
        assert_eq!(got, expect, "{name} (code {code:#06x})");
    }
    assert_eq!(machine.bus.read_u8(0x13).unwrap(), 3, "ROM version is V3");
    let dt = u32::from(machine.bus.read_u16(0x16).unwrap());
    let mut off = dt;
    let (mut sf, mut sd) = (0u32, 0u32);
    loop {
        let code = u32::from(machine.bus.read_u16(u64::from(off)).unwrap());
        let ptr = u32::from(machine.bus.read_u16(u64::from(off + 2)).unwrap());
        if code == 0 {
            break;
        }
        if code == 0x4653 {
            sf = ptr;
        }
        if code == 0x4453 {
            sd = ptr;
        }
        off += 4;
    }
    assert_ne!(sf, 0, "SF data table");
    assert_ne!(sd, 0, "SD data table");
    assert_eq!(machine.bus.read_u8(u64::from(sf - 2)).unwrap(), 32);
    assert_eq!(machine.bus.read_u8(u64::from(sd - 2)).unwrap(), 32);
    let cfsr = machine.bus.read_u32(0xE000_ED28).unwrap_or(0);
    assert_eq!(
        cfsr, 0,
        "a successful lookup must not set CFSR (got {cfsr:#x})"
    );
}

/// The same snippet with the bootrom region removed is the Hardware Lab
/// failure: `BLX` of the halfword at `0x18` enters flash at `0x2100`.
#[test]
fn missing_bootrom_region_branches_lookup_into_boot2() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (mut chip, manifest) = load_pico();
    chip.memory_regions.clear();
    let mut machine = machine_from(&chip, &manifest);
    plant_boot2(&mut machine);
    plant_lookup(&mut machine);

    assert_eq!(
        machine.bus.read_u16(0x18).unwrap(),
        0x2100,
        "without a ROM window the flash alias must serve boot2"
    );

    machine.cpu.set_pc(LOOKUP_ADDR);
    machine.cpu.set_sp(STACK);
    machine.cpu.lr = SENTINEL | 1;
    machine.cpu.r0 = 0x534d; // MS
    for step in 0..16 {
        if machine.cpu.get_pc() == BOOT2_LOOKUP_TARGET {
            assert!(
                step <= 10,
                "BLX into boot2 took {step} steps; the lookup itself is 7 instructions"
            );
            return;
        }
        machine.step().unwrap_or_else(|e| {
            panic!(
                "step {step} failed before reaching 0x2100: {e:?} (pc {:#x})",
                machine.cpu.get_pc()
            )
        });
    }
    panic!(
        "lookup never entered boot2 at 0x2100 (pc {:#x})",
        machine.cpu.get_pc()
    );
}

/// `LABWIRED_RP2040_BOOTROM=` (empty) is the bare-metal opt-out: the window
/// stays unmapped so a vector table at the flash alias is visible at 0.
#[test]
fn empty_bootrom_env_keeps_the_flash_alias() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let saved = std::env::var(ENV_BOOTROM).ok();
    std::env::set_var(ENV_BOOTROM, "");
    struct Restore(Option<String>);
    impl Drop for Restore {
        fn drop(&mut self) {
            restore(ENV_BOOTROM, self.0.as_deref());
        }
    }
    let _restore = Restore(saved);

    let (chip, manifest) = load_pico();
    let mut machine = machine_from(&chip, &manifest);
    plant_boot2(&mut machine);
    assert!(
        machine
            .bus
            .extra_mem
            .iter()
            .all(|region| region.base_addr != 0),
        "an empty LABWIRED_RP2040_BOOTROM must drop the base-0 window"
    );
    assert_eq!(machine.bus.read_u16(0x18).unwrap(), 0x2100);
}

fn run_rom_fn(machine: &mut Machine<CortexM>, thumb: u32) {
    machine.cpu.set_pc(thumb & !1);
    machine.cpu.set_sp(STACK);
    machine.cpu.lr = SENTINEL | 1;
    for step in 0..4_000 {
        if machine.cpu.get_pc() == SENTINEL {
            return;
        }
        machine.step().unwrap_or_else(|e| {
            panic!(
                "rom fn step {step} failed: {e:?} (pc {:#x})",
                machine.cpu.get_pc()
            )
        });
    }
    panic!("rom fn did not return (pc {:#x})", machine.cpu.get_pc());
}

/// Bit-counting helpers and the V3 SF/SD tables, invoked the way pico-sdk
/// does: `rom_func_lookup` then `blx`. HLE traps are the `bkpt` stubs.
#[test]
fn bitops_and_softfloat_tables_match_the_datasheet() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (chip, manifest) = load_pico();
    let mut machine = machine_from(&chip, &manifest);

    let lookup = |m: &mut Machine<CortexM>, code: u32| -> u32 {
        plant_lookup(m);
        run_lookup(m, code)
    };

    let p3 = lookup(&mut machine, 0x3350);
    machine.cpu.r0 = 0b1010_1100;
    run_rom_fn(&mut machine, p3);
    assert_eq!(machine.cpu.r0, 4, "popcount32");

    let r3 = lookup(&mut machine, 0x3352);
    machine.cpu.r0 = 0x8000_0001;
    run_rom_fn(&mut machine, r3);
    assert_eq!(machine.cpu.r0, 0x8000_0001u32.reverse_bits());

    let l3 = lookup(&mut machine, 0x334c);
    machine.cpu.r0 = 0;
    run_rom_fn(&mut machine, l3);
    assert_eq!(machine.cpu.r0, 32, "clz32(0)");
    machine.cpu.r0 = 1 << 15;
    run_rom_fn(&mut machine, l3);
    assert_eq!(machine.cpu.r0, 16, "clz32");

    let t3 = lookup(&mut machine, 0x3354);
    machine.cpu.r0 = 0;
    run_rom_fn(&mut machine, t3);
    assert_eq!(machine.cpu.r0, 32, "ctz32(0)");
    machine.cpu.r0 = 1 << 7;
    run_rom_fn(&mut machine, t3);
    assert_eq!(machine.cpu.r0, 7, "ctz32");

    // Data-table lookup of 'SF', then the _fadd slot at offset 0.
    plant_lookup(&mut machine);
    machine.cpu.set_pc(LOOKUP_ADDR);
    machine.cpu.set_sp(STACK);
    machine.cpu.lr = SENTINEL | 1;
    machine.cpu.r0 = 0x4653; // 'S','F'
                             // rom_data_lookup is the same helper with the data-table pointer:
                             // the snippet at LOOKUP_ADDR loads the *function* table from 0x14.
                             // Call table_lookup directly with r0 = data table, r1 = 'SF'.
    let dt = u32::from(machine.bus.read_u16(0x16).unwrap());
    machine.cpu.set_pc(0x1c);
    machine.cpu.r0 = dt;
    machine.cpu.r1 = 0x4653;
    run_rom_fn(&mut machine, 0x1d);
    let sf = machine.cpu.r0;
    assert_ne!(sf, 0);
    let fadd = machine.bus.read_u32(u64::from(sf)).unwrap();
    machine.cpu.r0 = 1.5f32.to_bits();
    machine.cpu.r1 = 2.25f32.to_bits();
    run_rom_fn(&mut machine, fadd);
    assert_eq!(f32::from_bits(machine.cpu.r0), 3.75);

    machine.cpu.set_pc(0x1c);
    machine.cpu.lr = SENTINEL | 1;
    machine.cpu.r0 = dt;
    machine.cpu.r1 = 0x4453; // 'S','D'
    run_rom_fn(&mut machine, 0x1d);
    let sd = machine.cpu.r0;
    let dadd = machine.bus.read_u32(u64::from(sd)).unwrap();
    let a = 0.1f64.to_bits();
    let b = 0.2f64.to_bits();
    machine.cpu.r0 = a as u32;
    machine.cpu.r1 = (a >> 32) as u32;
    machine.cpu.r2 = b as u32;
    machine.cpu.r3 = (b >> 32) as u32;
    run_rom_fn(&mut machine, dadd);
    let out = u64::from(machine.cpu.r0) | (u64::from(machine.cpu.r1) << 32);
    assert_eq!(f64::from_bits(out), 0.1 + 0.2);

    // Datasheet 2.8.3.2.1: float2fix(0.5, 16) == 0x8000.
    let float2fix = machine.bus.read_u32(u64::from(sf + 0x20)).unwrap();
    machine.cpu.r0 = 0.5f32.to_bits();
    machine.cpu.r1 = 16;
    run_rom_fn(&mut machine, float2fix);
    assert_eq!(machine.cpu.r0, 0x8000);
}

/// Datasheet 2.8.3: `RE` fills the mapped XIP window with 0xFF; `RP` programs
/// by bitwise-AND (NOR flash can only clear bits). The address is the flash
/// offset, not the 0x10000000 window address.
#[test]
fn flash_range_erase_and_program_the_xip_window() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (chip, manifest) = load_pico();
    let mut machine = machine_from(&chip, &manifest);
    plant_lookup(&mut machine);

    const OFF: u32 = 0x400;
    const XIP: u32 = 0x1000_0000;
    const N: usize = 16;
    const SRC: u32 = 0x2000_3000;

    write_bytes(&mut machine.bus, XIP + OFF - 1, &[0xA5]);
    write_bytes(&mut machine.bus, XIP + OFF, &[0x11; N]);
    write_bytes(&mut machine.bus, XIP + OFF + N as u32, &[0x5A]);

    let re = run_lookup(&mut machine, 0x4552);
    assert_eq!(re, 0x0113, "RE pointer");
    machine.cpu.r0 = OFF;
    machine.cpu.r1 = N as u32;
    machine.cpu.r2 = 0x1000;
    machine.cpu.r3 = 0x20;
    run_rom_fn(&mut machine, re);
    for i in 0..N {
        assert_eq!(
            machine.bus.read_u8(u64::from(XIP + OFF) + i as u64).unwrap(),
            0xFF,
            "erased {i}"
        );
    }
    assert_eq!(machine.bus.read_u8(u64::from(XIP + OFF - 1)).unwrap(), 0xA5);
    assert_eq!(
        machine.bus.read_u8(u64::from(XIP + OFF) + N as u64).unwrap(),
        0x5A
    );

    write_bytes(&mut machine.bus, SRC, &[0x0F; N]);
    let rp = run_lookup(&mut machine, 0x5052);
    assert_eq!(rp, 0x012b, "RP pointer");
    machine.cpu.r0 = OFF;
    machine.cpu.r1 = SRC;
    machine.cpu.r2 = N as u32;
    run_rom_fn(&mut machine, rp);
    for i in 0..N {
        assert_eq!(
            machine.bus.read_u8(u64::from(XIP + OFF) + i as u64).unwrap(),
            0x0F,
            "programmed {i}"
        );
    }

    write_bytes(&mut machine.bus, SRC, &[0xF0; N]);
    machine.cpu.r0 = OFF;
    machine.cpu.r1 = SRC;
    machine.cpu.r2 = N as u32;
    run_rom_fn(&mut machine, rp);
    for i in 0..N {
        assert_eq!(
            machine.bus.read_u8(u64::from(XIP + OFF) + i as u64).unwrap(),
            0x00,
            "AND {i}"
        );
    }
    assert_eq!(machine.bus.read_u8(u64::from(XIP + OFF - 1)).unwrap(), 0xA5);
}
