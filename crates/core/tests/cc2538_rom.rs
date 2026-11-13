// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! The in-tree clean-room CC2538 ROM (`crates/core/roms/cc2538/cc2538_rom.S`),
//! executed on the Cortex-M3 model through the chip descriptor's `rom` region.
//!
//! Every utility API entry is called the way firmware calls it: through the
//! function-pointer table at `0x48` (SWRU333A §3.2; Contiki-NG `rom-util.h`,
//! TI driverlib `rom.h`). The boot path is checked as SWRU333A §2.2 describes
//! it: a valid CCA hands over to the flash vector table with VTOR set, an
//! invalid one stays in the ROM.

mod common;
use common::root;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::cpu::CortexM;
use labwired_core::memory::{ProgramImage, Segment};
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Arch, Bus, Cpu, Machine};

const API_TABLE: u64 = 0x48;
const ROM_END: u32 = 0x0002_0000;
/// `b .` in SRAM: where LR returns.
const SENTINEL: u32 = 0x2000_6000;
const STACK: u32 = 0x2000_7F00;
const BUF: u32 = 0x2000_1000;

// SWRU333A §3.2 / rom.h order.
const CRC32: u64 = 0;
const GET_FLASH_SIZE: u64 = 1;
const GET_CHIP_ID: u64 = 2;
const PAGE_ERASE: u64 = 3;
const PROGRAM_FLASH: u64 = 4;
const MEMSET: u64 = 6;
const MEMCPY: u64 = 7;
const MEMCMP: u64 = 8;
const MEMMOVE: u64 = 9;

fn machine() -> Machine<CortexM> {
    let chip_path = root("configs/chips/cc2538sf53.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("cc2538sf53 chip");
    let mut manifest =
        SystemManifest::from_file(root("configs/systems/cc2538dk.yaml")).expect("cc2538dk");
    manifest.chip = chip_path.to_str().expect("utf8").to_string();
    let mut bus = labwired_core::bus::SystemBus::from_config(&chip, &manifest).expect("bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut m = Machine::new(cpu, bus);
    m.bus.write_u16(u64::from(SENTINEL), 0xE7FE).unwrap();
    m
}

fn api(m: &mut Machine<CortexM>, index: u64) -> u32 {
    m.bus.read_u32(API_TABLE + 4 * index).unwrap()
}

/// Call API entry `index` with `args` in r0-r2; return r0.
fn call(m: &mut Machine<CortexM>, index: u64, args: &[u32]) -> u32 {
    let entry = api(m, index);
    m.cpu.set_pc(entry & !1);
    m.cpu.set_sp(STACK);
    m.cpu.lr = SENTINEL | 1;
    let regs = [&mut m.cpu.r0, &mut m.cpu.r1, &mut m.cpu.r2];
    for (reg, &arg) in regs.into_iter().zip(args) {
        *reg = arg;
    }
    // A page erase polls FCTL.BUSY for 20 ms (320 000 cycles at 16 MHz).
    for _ in 0..2_000_000 {
        if m.cpu.get_pc() == SENTINEL {
            assert_eq!(m.cpu.sp, STACK, "API entry {index} unbalanced the stack");
            return m.cpu.r0;
        }
        m.step()
            .unwrap_or_else(|e| panic!("API entry {index}: {e:?}"));
    }
    panic!(
        "API entry {index} did not return (pc {:#x})",
        m.cpu.get_pc()
    );
}

fn put(m: &mut Machine<CortexM>, addr: u32, bytes: &[u8]) {
    for (i, b) in bytes.iter().enumerate() {
        m.bus.write_u8(u64::from(addr) + i as u64, *b).unwrap();
    }
}

fn get(m: &mut Machine<CortexM>, addr: u32, len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| m.bus.read_u8(u64::from(addr) + i as u64).unwrap())
        .collect()
}

#[test]
fn api_table_follows_the_vector_table_and_points_into_rom() {
    let mut m = machine();
    for index in 0..10 {
        let entry = api(&mut m, index);
        assert_eq!(entry & 1, 1, "entry {index} is not a Thumb address");
        assert!(
            entry < ROM_END,
            "entry {index} = {entry:#x} is outside the ROM"
        );
    }
}

#[test]
fn crc32_is_ieee_802_3() {
    let mut m = machine();
    put(&mut m, BUF, b"123456789");
    // The CRC-32 check value for "123456789".
    assert_eq!(call(&mut m, CRC32, &[BUF, 9]), 0xCBF4_3926);
    assert_eq!(call(&mut m, CRC32, &[BUF, 0]), 0);
}

#[test]
fn device_queries_report_the_cc2538sf53() {
    let mut m = machine();
    // FLASH_CTRL_DIECFG0.CHIPID (SWRU319C) and the 512 KiB flash option.
    assert_eq!(call(&mut m, GET_CHIP_ID, &[]), 0xB964);
    assert_eq!(call(&mut m, GET_FLASH_SIZE, &[]), 512 * 1024);
}

/// Page 8 of the main array (0x0020_4000), well clear of the CCA page.
const PAGE8: u32 = 0x0020_4000;
const PAGE: u32 = 2048;
/// The lock-bit page (CCA): the last page of the 512 KiB array.
const CCA_PAGE: u32 = 0x0027_F800;

fn words(m: &mut Machine<CortexM>, addr: u32, n: u32) -> Vec<u32> {
    (0..n)
        .map(|i| m.bus.read_u32(u64::from(addr + 4 * i)).unwrap())
        .collect()
}

fn put_words(m: &mut Machine<CortexM>, addr: u32, ws: &[u32]) {
    for (i, w) in ws.iter().enumerate() {
        m.bus.write_u32(u64::from(addr) + 4 * i as u64, *w).unwrap();
    }
}

#[test]
fn page_erase_and_program_flash_go_through_the_flash_controller() {
    let mut m = machine();
    // Dirty two pages so the erase has something to undo.
    put_words(&mut m, PAGE8, &[0x1234_5678; 4]);
    put_words(&mut m, PAGE8 + PAGE, &[0; 4]);
    // An erase that ends one byte into page 9 erases both pages.
    assert_eq!(call(&mut m, PAGE_ERASE, &[PAGE8, PAGE + 1]), 0);
    assert!(words(&mut m, PAGE8, 2 * PAGE / 4)
        .iter()
        .all(|&w| w == 0xFFFF_FFFF));

    let data = [0xDEAD_BEEF, 0x0102_0304, 0xFFFF_0000, 0x5555_AAAA];
    put_words(&mut m, BUF, &data);
    assert_eq!(call(&mut m, PROGRAM_FLASH, &[BUF, PAGE8 + 8, 16]), 0);
    assert_eq!(words(&mut m, PAGE8, 7)[..2], [0xFFFF_FFFF; 2]);
    assert_eq!(words(&mut m, PAGE8 + 8, 4), data);
    assert_eq!(words(&mut m, PAGE8 + 24, 1), [0xFFFF_FFFF]);

    // SWRU319C §8.3 / SWRU333A §3.2.2.9: programming ANDs, it never sets a 0
    // bit back to 1.
    put_words(&mut m, BUF, &[0x0F0F_F0F0]);
    assert_eq!(call(&mut m, PROGRAM_FLASH, &[BUF, PAGE8 + 8, 4]), 0);
    assert_eq!(words(&mut m, PAGE8 + 8, 1), [0xDEAD_BEEF & 0x0F0F_F0F0]);
}

#[test]
fn page_erase_takes_the_datasheet_20_ms() {
    let mut m = machine();
    let start = m.total_cycles;
    assert_eq!(call(&mut m, PAGE_ERASE, &[PAGE8, PAGE]), 0);
    // SWRU319C §8.1: page erase, minimum 20 ms; the controller runs at the
    // 16-MHz reset SYS clock.
    let elapsed = m.total_cycles - start;
    assert!(
        (320_000..330_000).contains(&elapsed),
        "erase took {elapsed} cycles"
    );
}

#[test]
fn flash_calls_reject_bad_parameters_with_minus_two() {
    let mut m = machine();
    for args in [
        [PAGE8 + 2, PAGE, 0],   // not word-aligned
        [0x0010_0000, PAGE, 0], // below the array
        [0x0028_0000, PAGE, 0], // past the array
        [0x0027_FFF0, 0x20, 0], // runs off the end
    ] {
        assert_eq!(call(&mut m, PAGE_ERASE, &args[..2]) as i32, -2, "{args:x?}");
    }
    assert_eq!(call(&mut m, PROGRAM_FLASH, &[BUF, PAGE8, 6]) as i32, -2);
    assert_eq!(call(&mut m, PROGRAM_FLASH, &[BUF, PAGE8 + 1, 4]) as i32, -2);
    assert_eq!(
        call(&mut m, PROGRAM_FLASH, &[BUF, 0x0028_0000, 4]) as i32,
        -2
    );
    // Zero-length calls succeed without touching the controller.
    assert_eq!(call(&mut m, PAGE_ERASE, &[PAGE8, 0]), 0);
    assert_eq!(call(&mut m, PROGRAM_FLASH, &[BUF, PAGE8, 0]), 0);
}

#[test]
fn a_cca_locked_page_aborts_erase_and_program() {
    let mut m = machine();
    // Unlock everything (all ones), then clear LOCK_PAGE_N[8] (Table 8-3:
    // byte 2016 + 8 / 8 = 2017, bit 0).
    put(&mut m, CCA_PAGE + 2016, &[0xFF; 32]);
    put(&mut m, CCA_PAGE + 2017, &[0xFE]);
    put_words(&mut m, PAGE8, &[0x1234_5678]);
    assert_eq!(call(&mut m, PAGE_ERASE, &[PAGE8, PAGE]) as i32, -1);
    assert_eq!(
        words(&mut m, PAGE8, 1),
        [0x1234_5678],
        "a locked erase changed the page"
    );
    put_words(&mut m, BUF, &[0]);
    assert_eq!(call(&mut m, PROGRAM_FLASH, &[BUF, PAGE8, 4]) as i32, -1);
    assert_eq!(
        words(&mut m, PAGE8, 1),
        [0x1234_5678],
        "a locked write landed"
    );
    // Page 9's lock bit is still 1.
    assert_eq!(call(&mut m, PAGE_ERASE, &[PAGE8 + PAGE, PAGE]), 0);
}

#[test]
fn the_lock_bit_page_needs_upper_page_access() {
    const FCTL: u64 = 0x400D_3008;
    let mut m = machine();
    put(&mut m, CCA_PAGE + 2016, &[0xFF; 32]);
    put_words(&mut m, CCA_PAGE, &[0xFFFF_FFFF]);
    put_words(&mut m, BUF, &[0xA5A5_A5A5]);
    // SWRU319C §8.2.2: write and erase of the lock-bit page abort unless
    // FCTL.UPPER_PAGE_ACCESS is set ...
    assert_eq!(call(&mut m, PROGRAM_FLASH, &[BUF, CCA_PAGE, 4]) as i32, -1);
    assert_eq!(m.bus.read_u32(FCTL).unwrap() & 0x20, 0x20, "FCTL.ABORT");
    assert_eq!(words(&mut m, CCA_PAGE, 1), [0xFFFF_FFFF]);
    assert_eq!(call(&mut m, PAGE_ERASE, &[CCA_PAGE, PAGE]) as i32, -1);
    // ... and the ROM keeps the bit a caller sets (driverlib
    // FlashUpperPageProgram sets it around the ROM call).
    let fctl = m.bus.read_u32(FCTL).unwrap();
    m.bus.write_u32(FCTL, fctl | 0x200).unwrap();
    assert_eq!(call(&mut m, PROGRAM_FLASH, &[BUF, CCA_PAGE, 4]), 0);
    assert_eq!(words(&mut m, CCA_PAGE, 1), [0xA5A5_A5A5]);
    assert_eq!(
        m.bus.read_u32(FCTL).unwrap() & 0x220,
        0x200,
        "ABORT cleared on restart"
    );
}

#[test]
fn memset_memcpy_memcmp() {
    let mut m = machine();
    put(&mut m, BUF, &[0u8; 16]);
    // memset converts C to unsigned char and returns Dst.
    assert_eq!(call(&mut m, MEMSET, &[BUF + 2, 0x1A5, 5]), BUF + 2);
    assert_eq!(get(&mut m, BUF, 8), [0, 0, 0xA5, 0xA5, 0xA5, 0xA5, 0xA5, 0]);

    put(&mut m, BUF + 0x100, b"labwired");
    assert_eq!(
        call(&mut m, MEMCPY, &[BUF + 0x200, BUF + 0x100, 8]),
        BUF + 0x200
    );
    assert_eq!(get(&mut m, BUF + 0x200, 8), b"labwired");

    assert_eq!(call(&mut m, MEMCMP, &[BUF + 0x100, BUF + 0x200, 8]), 0);
    put(&mut m, BUF + 0x200, b"labwirea");
    assert!((call(&mut m, MEMCMP, &[BUF + 0x100, BUF + 0x200, 8]) as i32) > 0);
    assert!((call(&mut m, MEMCMP, &[BUF + 0x200, BUF + 0x100, 8]) as i32) < 0);
    // Only Length bytes are compared.
    assert_eq!(call(&mut m, MEMCMP, &[BUF + 0x100, BUF + 0x200, 7]), 0);
}

#[test]
fn memmove_handles_both_overlaps() {
    let mut m = machine();
    // Dst above Src inside the source: must copy from the end.
    put(&mut m, BUF, b"0123456789");
    assert_eq!(call(&mut m, MEMMOVE, &[BUF + 2, BUF, 6]), BUF + 2);
    assert_eq!(get(&mut m, BUF, 10), b"0101234589");
    // Dst below Src: a forward copy is correct.
    put(&mut m, BUF, b"0123456789");
    assert_eq!(call(&mut m, MEMMOVE, &[BUF, BUF + 3, 6]), BUF);
    assert_eq!(get(&mut m, BUF, 10), b"3456786789");
}

/// A flash image with its vector table at `0x0020_2000` (as Contiki-NG links
/// it) and a CCA whose Image Valid word is `image_valid`.
fn cca_image(image_valid: u32) -> ProgramImage {
    const TABLE: u32 = 0x0020_2000;
    const RESET: u32 = 0x0020_2100;
    let mut table = Vec::new();
    table.extend_from_slice(&0x2000_7000u32.to_le_bytes());
    table.extend_from_slice(&(RESET | 1).to_le_bytes());
    let mut cca = Vec::new();
    cca.extend_from_slice(&0xF3FF_FFFFu32.to_le_bytes()); // backdoor config
    cca.extend_from_slice(&image_valid.to_le_bytes());
    cca.extend_from_slice(&TABLE.to_le_bytes()); // Application Entry Point
    cca.extend_from_slice(&[0xFF; 32]); // lock bits: unlocked
    let mut image = ProgramImage::new(u64::from(RESET), Arch::Arm);
    image.segments.push(Segment {
        start_addr: u64::from(TABLE),
        data: table,
    });
    image.segments.push(Segment {
        start_addr: u64::from(RESET),
        data: vec![0xFE, 0xE7], // b .
    });
    image.segments.push(Segment {
        start_addr: 0x0027_FFD4,
        data: cca,
    });
    image
}

#[test]
fn rom_boot_hands_a_valid_cca_image_its_vector_table() {
    let mut m = machine();
    m.load_firmware(&cca_image(0)).unwrap();
    // Reset entered the ROM (address 0 holds the ROM's own vectors).
    assert!(m.cpu.get_pc() < ROM_END, "reset did not enter the ROM");
    for _ in 0..100 {
        if m.cpu.get_pc() == 0x0020_2100 {
            break;
        }
        m.step().unwrap();
    }
    assert_eq!(
        m.cpu.get_pc(),
        0x0020_2100,
        "ROM did not branch to the flash reset ISR"
    );
    assert_eq!(m.cpu.sp, 0x2000_7000, "SP not taken from the flash table");
    assert_eq!(m.bus.read_u32(0xE000_ED08).unwrap(), 0x0020_2000, "VTOR");
}

#[test]
fn rom_boot_stays_in_rom_without_a_valid_image() {
    let mut m = machine();
    m.load_firmware(&cca_image(0xFFFF_FFFF)).unwrap();
    for _ in 0..100 {
        m.step().unwrap();
    }
    // SWRU333A §2.1: the serial boot loader would run here; this ROM parks.
    assert!(m.cpu.get_pc() < ROM_END, "left the ROM with an invalid CCA");
    assert_eq!(m.bus.read_u32(0xE000_ED08).unwrap(), 0, "VTOR moved");
}
