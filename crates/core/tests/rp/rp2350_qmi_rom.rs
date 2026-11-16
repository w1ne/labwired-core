// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! RP2350 flash path: the QMI in direct mode driving the CS0 serial NOR
//! (datasheet 12.14), the boot-ROM flash APIs on top of it (5.4.8), the ROM
//! boot handoff through an IMAGE_DEF (5.2, 5.9) and UF2 images.
//!
//! ROM APIs are called the way pico-sdk calls them: the address comes from
//! `rom_table_lookup_val` (the 16-bit pointer at 0x16, datasheet Table 453),
//! the call runs on the simulated core and returns to a `b .` sentinel.

use crate::common::root;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::CortexM;
use labwired_core::memory::ProgramImage;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Arch, Bus, Cpu, Machine, SimulationError};

const XIP: u64 = 0x1000_0000;
const QMI: u64 = 0x400D_0000;
const BOOTRAM: u64 = 0x400E_0000;
const VTOR: u64 = 0xE000_ED08;
const SENTINEL: u32 = 0x2000_0000;
const STACK: u32 = 0x2004_0000;
const RT_FLAG_FUNC_ARM_SEC: u32 = 0x0004;

fn code(a: u8, b: u8) -> u32 {
    u32::from(a) | (u32::from(b) << 8)
}

fn bus(chip_rel: &str, system_rel: &str) -> SystemBus {
    let chip_path = root(chip_rel);
    let chip = ChipDescriptor::from_file(&chip_path).expect("chip");
    let mut manifest = SystemManifest::from_file(root(system_rel)).expect("system");
    manifest.chip = chip_path.to_str().unwrap().to_string();
    SystemBus::from_config(&chip, &manifest).expect("bus")
}

fn machine(chip_rel: &str, system_rel: &str, image: &ProgramImage) -> Machine<CortexM> {
    let mut b = bus(chip_rel, system_rel);
    let (cpu, _) = configure_cortex_m(&mut b);
    let mut m = Machine::new(cpu, b);
    m.load_firmware(image).expect("load");
    m
}

/// A flash image: vector table at the image start, `b .` at `+0x100`, and an
/// IMAGE_DEF block at `+0x40` when `image_def` (EXE, Secure, Arm, RP2350;
/// pico-sdk boot/picobin.h).
fn tiny_image(image_def: bool) -> ProgramImage {
    let mut flash = vec![0xFFu8; 0x200];
    let w = |f: &mut Vec<u8>, at: usize, v: u32| f[at..at + 4].copy_from_slice(&v.to_le_bytes());
    w(&mut flash, 0, 0x2000_1000);
    w(&mut flash, 4, 0x1000_0101);
    if image_def {
        for (i, v) in [0xffff_ded3u32, 0x1021_0142, 0x0000_01ff, 0, 0xab12_3579]
            .into_iter()
            .enumerate()
        {
            w(&mut flash, 0x40 + i * 4, v);
        }
    }
    flash[0x100..0x102].copy_from_slice(&0xe7feu16.to_le_bytes());
    let mut image = ProgramImage::new(0x1000_0101, Arch::Arm);
    image.add_segment(XIP, flash);
    image
}

fn run_until_pc(m: &mut Machine<CortexM>, pc: u32, max: u32) -> Result<(), SimulationError> {
    for _ in 0..max {
        if m.cpu.get_pc() == pc {
            return Ok(());
        }
        m.step()?;
    }
    panic!("PC never reached {pc:#x}; stuck at {:#x}", m.cpu.get_pc());
}

/// Call a ROM function on the simulated core and return r0.
fn call(m: &mut Machine<CortexM>, pc: u32, args: [u32; 4]) -> u32 {
    m.bus.write_u16(u64::from(SENTINEL), 0xe7fe).unwrap();
    m.cpu.set_pc(pc & !1);
    m.cpu.set_sp(STACK);
    m.cpu.lr = SENTINEL | 1;
    m.cpu.r0 = args[0];
    m.cpu.r1 = args[1];
    m.cpu.r2 = args[2];
    m.cpu.r3 = args[3];
    for _ in 0..2_000_000 {
        if m.cpu.get_pc() == SENTINEL {
            return m.cpu.r0;
        }
        m.step()
            .unwrap_or_else(|e| panic!("ROM call {pc:#x}: step at {:#x}: {e:?}", m.cpu.get_pc()));
    }
    panic!("ROM call {pc:#x} did not return");
}

fn rom_fn(m: &mut Machine<CortexM>, c: u32) -> u32 {
    let lookup = u32::from(m.bus.read_u16(0x16).unwrap());
    let f = call(m, lookup, [c, RT_FLAG_FUNC_ARM_SEC, 0, 0]);
    assert_ne!(f, 0, "ROM function {c:#06x} missing from the table");
    f
}

#[test]
fn rom_flash_apis_erase_and_program_through_the_qmi() {
    let mut m = machine(
        "configs/chips/rp2350.yaml",
        "configs/systems/rp2350-zero.yaml",
        &tiny_image(true),
    );
    // Something to erase at flash offset 0x1000.
    for i in 0..4096u64 {
        m.bus.flash.data[0x1000 + i as usize] = (i as u8) ^ 0x5A;
    }
    let exit_xip = rom_fn(&mut m, code(b'E', b'X'));
    let erase = rom_fn(&mut m, code(b'R', b'E'));
    let program = rom_fn(&mut m, code(b'R', b'P'));
    let flush = rom_fn(&mut m, code(b'F', b'C'));
    let enter_xip = rom_fn(&mut m, code(b'C', b'X'));

    call(&mut m, exit_xip, [0; 4]);
    call(&mut m, erase, [0x1000, 4096, 1 << 16, 0xd8]);
    for i in 0..4096u64 {
        assert_eq!(
            m.bus.read_u8(XIP + 0x1000 + i).unwrap(),
            0xFF,
            "erased byte {i}"
        );
    }
    // 300 bytes from RAM: crosses the 256-byte page boundary at 0x1100.
    let data: Vec<u8> = (0..300u32).map(|i| (i * 7 + 3) as u8).collect();
    for (i, b) in data.iter().enumerate() {
        m.bus.write_u8(0x2001_0000 + i as u64, *b).unwrap();
    }
    call(&mut m, program, [0x1000, 0x2001_0000, 300, 0]);
    call(&mut m, flush, [0; 4]);
    call(&mut m, enter_xip, [0; 4]);
    for (i, b) in data.iter().enumerate() {
        assert_eq!(
            m.bus.read_u8(XIP + 0x1000 + i as u64).unwrap(),
            *b,
            "byte {i}"
        );
    }
    assert_eq!(m.bus.read_u8(XIP + 0x1000 + 300).unwrap(), 0xFF);
    // NOR program only clears bits: 0x0F over 0x07 stays 0x07 & 0x0F.
    m.bus.write_u8(0x2001_0000, 0x0F).unwrap();
    call(&mut m, exit_xip, [0; 4]);
    call(&mut m, program, [0x1000, 0x2001_0000, 1, 0]);
    call(&mut m, enter_xip, [0; 4]);
    assert_eq!(m.bus.read_u8(XIP + 0x1000).unwrap(), data[0] & 0x0F);
    // flash_enter_cmd_xip leaves direct mode off and M0 on 03h serial reads.
    assert_eq!(m.bus.read_u32(QMI).unwrap() & 1, 0, "DIRECT_CSR.EN");
    assert_eq!(m.bus.read_u32(QMI + 0x14).unwrap(), 0xa003, "M0_RCMD");
}

#[test]
fn qmi_direct_mode_reads_status_and_refuses_what_it_does_not_model() {
    let mut b = bus(
        "configs/chips/rp2350.yaml",
        "configs/systems/rp2350-zero.yaml",
    );
    b.flash.data[0x20..0x24].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
    b.write_u32(QMI, 0x0180_0005).unwrap(); // EN | ASSERT_CS0N
    for byte in [0x03, 0x00, 0x00, 0x20] {
        b.write_u32(QMI + 4, byte | (1 << 20)).unwrap(); // NOPUSH
        let _ = b.tick_peripherals_fully();
    }
    assert_ne!(
        b.read_u32(QMI).unwrap() & (1 << 16),
        0,
        "RXEMPTY after NOPUSH"
    );
    // A 16-bit entry returns two array bytes, first on the wire high.
    b.write_u32(QMI + 4, 1 << 18).unwrap();
    assert_ne!(
        b.read_u32(QMI).unwrap() & 2,
        0,
        "BUSY until the shift completes"
    );
    let _ = b.tick_peripherals_fully();
    assert_eq!(b.read_u32(QMI + 8).unwrap(), 0xDEAD);
    b.write_u32(QMI, 0x0180_0001).unwrap(); // CS0 released
    let _ = b.tick_peripherals_fully();

    // Read status 1 after WREN: WEL set.
    b.write_u32(QMI, 0x0180_0005).unwrap();
    b.write_u32(QMI + 4, 0x06).unwrap();
    let _ = b.tick_peripherals_fully();
    let _ = b.read_u32(QMI + 8).unwrap();
    b.write_u32(QMI, 0x0180_0001).unwrap();
    let _ = b.tick_peripherals_fully();
    b.write_u32(QMI, 0x0180_0005).unwrap();
    b.write_u32(QMI + 4, 0x05).unwrap();
    b.write_u32(QMI + 4, 0xFF).unwrap();
    let _ = b.tick_peripherals_fully();
    let _ = b.read_u32(QMI + 8).unwrap();
    assert_eq!(b.read_u32(QMI + 8).unwrap() & 0x3, 0x2, "SR1: WEL, not WIP");
    b.write_u32(QMI, 0x0180_0001).unwrap();
    let _ = b.tick_peripherals_fully();

    let refused = |r: Result<(), SimulationError>, what: &str| match r {
        Err(SimulationError::NotImplemented(m)) => assert!(m.contains(what), "{m}"),
        other => panic!("expected a refusal naming {what:?}, got {other:?}"),
    };
    b.write_u32(QMI, 0x0180_0005).unwrap();
    refused(b.write_u32(QMI + 4, 0xEB | (2 << 16)), "IWIDTH");
    refused(b.write_u32(QMI + 4, 0x9F), "jedec_id");
    refused(b.write_u32(QMI, 0x0180_0041), "AUTO_CS");
    refused(b.write_u32(QMI, 0x0180_0009), "CS1");
    refused(b.write_u32(QMI + 0x34, 0), "ATRANS0");
}

#[test]
fn rom_boot_hands_off_through_the_image_def() {
    let mut m = machine(
        "configs/chips/rp2354b.yaml",
        "configs/systems/rp2354b.yaml",
        &tiny_image(true),
    );
    assert!(m.cpu.get_pc() < 0x8000, "reset enters the boot ROM");
    run_until_pc(&mut m, 0x1000_0100, 20_000).expect("handoff");
    assert_eq!(m.bus.read_u32(VTOR).unwrap(), 0x1000_0000);
    assert_eq!(m.cpu.get_register(13), 0x2000_1000);
    // The XIP setup function sits in boot RAM, as pico-sdk expects.
    let cx = rom_fn(&mut m, code(b'C', b'X')) & !1;
    for i in (0..256u64).step_by(4) {
        assert_eq!(
            m.bus.read_u32(BOOTRAM + i).unwrap(),
            m.bus.read_u32(u64::from(cx) + i).unwrap(),
            "boot RAM word {i:#x}"
        );
    }
}

#[test]
fn rom_boot_halts_on_an_image_without_an_image_def() {
    let mut m = machine(
        "configs/chips/rp2354b.yaml",
        "configs/systems/rp2354b.yaml",
        &tiny_image(false),
    );
    let mut halted = false;
    for _ in 0..20_000 {
        match m.step() {
            Ok(_) => assert_ne!(m.cpu.get_pc(), 0x1000_0100, "booted without IMAGE_DEF"),
            Err(SimulationError::Halt) => {
                halted = true;
                break;
            }
            Err(e) => panic!("unexpected {e:?}"),
        }
    }
    assert!(halted, "the ROM must refuse the image");
    assert!(m.cpu.get_pc() < 0x8000, "halted inside the ROM");
}

/// Build a UF2 (RP2350 Arm-S family) from an image's segments.
fn to_uf2(image: &ProgramImage) -> Vec<u8> {
    let mut payload: Vec<(u32, Vec<u8>)> = Vec::new();
    for s in &image.segments {
        for (i, c) in s.data.chunks(256).enumerate() {
            let mut c = c.to_vec();
            c.resize(256, 0);
            payload.push((s.start_addr as u32 + i as u32 * 256, c));
        }
    }
    let total = payload.len() as u32;
    let mut out = Vec::new();
    for (n, (addr, data)) in payload.into_iter().enumerate() {
        for w in [
            0x0A32_4655u32,
            0x9E5D_5157,
            0x2000,
            addr,
            256,
            n as u32,
            total,
            0xE48B_FF59,
        ] {
            out.extend_from_slice(&w.to_le_bytes());
        }
        out.extend_from_slice(&data);
        out.resize(out.len() + 476 - 256, 0);
        out.extend_from_slice(&0x0AB1_6F30u32.to_le_bytes());
    }
    out
}

#[test]
fn uf2_image_boots_through_the_rom_and_runs() {
    let elf = labwired_loader::load_elf(&root("tests/fixtures/rp2354b-smoke.elf")).unwrap();
    let flash_only = {
        let mut img = ProgramImage::new(0, Arch::Arm);
        for s in &elf.segments {
            if (XIP..XIP + 0x0100_0000).contains(&s.start_addr) {
                img.add_segment(s.start_addr, s.data.clone());
            }
        }
        img
    };
    let image = labwired_loader::load_elf_bytes(&to_uf2(&flash_only)).expect("UF2 parses");
    assert_eq!(image.arch, Arch::Arm);
    let mut b = bus("configs/chips/rp2354b.yaml", "configs/systems/rp2354b.yaml");
    let uart = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    b.attach_uart_tx_sink(uart.clone(), false);
    let (cpu, _) = configure_cortex_m(&mut b);
    let mut m = Machine::new(cpu, b);
    m.load_firmware(&image).unwrap();
    for _ in 0..1_000_000 {
        m.step().unwrap();
        if String::from_utf8_lossy(&uart.lock().unwrap()).contains("GPIO47 OK\n") {
            return;
        }
    }
    panic!(
        "UF2 image did not run: uart={:?}",
        String::from_utf8_lossy(&uart.lock().unwrap())
    );
}

/// pico-sdk `xip_cache_clean_all` (hardware_xip_cache): byte stores to the
/// maintenance window at `0x1bffc001 + 8*i` (clean by set/way, RP2350-E11
/// workaround). Betaflight's flash config save runs it. Pinning is refused.
#[test]
fn xip_cache_maintenance_stores_are_accepted_and_pinning_refused() {
    let mut b = bus(
        "configs/chips/rp2350.yaml",
        "configs/systems/rp2350-zero.yaml",
    );
    for i in 0..2048u64 {
        b.write_u8(0x1BFF_C001 + 8 * i, 0)
            .expect("clean by set/way");
    }
    b.write_u8(0x1800_1002, 0).expect("invalidate by address");
    match b.write_u8(0x1800_0007, 0) {
        Err(SimulationError::NotImplemented(m)) => assert!(m.contains("pinning"), "{m}"),
        other => panic!("pin must be refused, got {other:?}"),
    }
}
