// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! The MIMXRT595S secure view, as the stock Zephyr board
//! `mimxrt595_evk/mimxrt595s/cm33` uses it.
//!
//! - The IDAU secure aliases (`memory_regions[].alias_of` in
//!   `configs/chips/mimxrt595s.yaml`): FlexSPI `0x1800_0000`, SRAM
//!   `0x3000_0000`, peripherals `0x5000_0000` and the boot ROM `0x1300_0000`
//!   are the same bytes / the same peripheral as the non-secure address.
//! - The in-tree clean-room boot-ROM window (`crates/core/roms/rt500`),
//!   executed on the Cortex-M33 model: the three OTP entry points are called
//!   at the fixed secure addresses the SDK's `fsl_clock.c` uses.

mod common;
use common::root;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::cpu::CortexM;
use labwired_core::memory::{ProgramImage, Segment};
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Arch, Bus, Cpu, Machine};

/// `b .` in SRAM (secure view): where LR returns.
const SENTINEL: u32 = 0x3000_6000;
const STACK: u32 = 0x3000_7F00;
const FUSE_WORD: u32 = 0x3000_1000;

// MCUXpresso SDK devices/MIMXRT595S/drivers/fsl_clock.c.
const OTP_INIT_API: u32 = 0x1300_7FFF;
const OTP_DEINIT_API: u32 = 0x1300_804D;
const OTP_FUSE_READ_API: u32 = 0x1300_805D;
const FRO_192MHZ_SC_TRIM: u32 = 0x2C;

fn chip() -> (ChipDescriptor, SystemManifest) {
    let chip_path = root("configs/chips/mimxrt595s.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("mimxrt595s chip");
    let mut manifest =
        SystemManifest::from_file(root("configs/systems/mimxrt595-evk.yaml")).expect("evk");
    manifest.chip = chip_path.to_str().expect("utf8").to_string();
    (chip, manifest)
}

fn machine() -> Machine<CortexM> {
    let (chip, manifest) = chip();
    let mut bus = labwired_core::bus::SystemBus::from_config(&chip, &manifest).expect("bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut m = Machine::new(cpu, bus);
    m.bus.write_u16(u64::from(SENTINEL), 0xE7FE).unwrap();
    m
}

/// Call the Thumb function at `entry` with `args` in r0-r1; return r0.
fn call(m: &mut Machine<CortexM>, entry: u32, args: &[u32]) -> u32 {
    m.cpu.set_pc(entry & !1);
    m.cpu.set_sp(STACK);
    m.cpu.lr = SENTINEL | 1;
    let regs = [&mut m.cpu.r0, &mut m.cpu.r1];
    for (reg, &arg) in regs.into_iter().zip(args) {
        *reg = arg;
    }
    for _ in 0..1_000 {
        if m.cpu.get_pc() == SENTINEL {
            assert_eq!(m.cpu.sp, STACK, "{entry:#x} unbalanced the stack");
            return m.cpu.r0;
        }
        m.step().unwrap_or_else(|e| panic!("{entry:#x}: {e:?}"));
    }
    panic!("{entry:#x} did not return (pc {:#x})", m.cpu.get_pc());
}

#[test]
fn secure_aliases_share_the_non_secure_backing() {
    let mut m = machine();
    // SRAM: a store through either view is visible through the other.
    m.bus.write_u32(0x3018_0000, 0xA5A5_1234).unwrap();
    assert_eq!(m.bus.read_u32(0x2018_0000).unwrap(), 0xA5A5_1234);
    m.bus.write_u16(0x2000_0010, 0xBEEF).unwrap();
    assert_eq!(m.bus.read_u16(0x3000_0010).unwrap(), 0xBEEF);
    assert_eq!(m.bus.read_u8(0x3000_0011).unwrap(), 0xBE);
    // Peripherals: FLEXCOMM0 PSELID (0xFF8) reads the same through 0x5010_6000.
    assert_eq!(
        m.bus.read_u32(0x5010_6FF8).unwrap(),
        m.bus.read_u32(0x4010_6FF8).unwrap()
    );
    // Past a mirror's end nothing is translated: SRAM is 5 MiB.
    assert!(m.bus.read_u32(0x3050_0000).is_err());
}

#[test]
fn an_image_linked_at_the_secure_alias_loads_into_flash() {
    let mut m = machine();
    // A Zephyr-style vector table at the secure FlexSPI alias + 0x1000.
    let mut table = Vec::new();
    table.extend_from_slice(&0x3018_1F58u32.to_le_bytes());
    table.extend_from_slice(&0x1800_2001u32.to_le_bytes());
    let image = ProgramImage {
        entry_point: 0x1800_2001,
        segments: vec![
            Segment {
                start_addr: 0x1800_1000,
                data: table,
            },
            // `b .` at the reset handler.
            Segment {
                start_addr: 0x1800_2000,
                data: vec![0xFE, 0xE7],
            },
        ],
        arch: Arch::Arm,
    };
    m.load_firmware(&image).expect("load");
    // The bytes are in the non-secure FlexSPI window...
    assert_eq!(m.bus.read_u32(0x0800_1000).unwrap(), 0x3018_1F58);
    // ...and the reset vector (reset_vector_offset 0x1000) was accepted even
    // though SP and PC are secure-alias addresses.
    assert_eq!(m.cpu.get_pc(), 0x1800_2000);
    assert_eq!(m.cpu.sp, 0x3018_1F58);
    m.step().expect("execute from the secure alias");
    assert_eq!(m.cpu.get_pc(), 0x1800_2000);
}

#[test]
fn secure_alias_code_fetch_serves_the_target_bytes() {
    let m = machine();
    let (base, end, slice) = m
        .bus
        .plain_memory_slice(0x1800_1004)
        .expect("FlexSPI secure alias is plain memory");
    // Reported in the alias's own address space, clipped to the mirror.
    assert_eq!((base, end), (0x1800_0000, 0x1880_0000));
    let (nbase, _, nslice) = m.bus.plain_memory_slice(0x0800_1004).unwrap();
    assert_eq!(nbase, 0x0800_0000);
    assert_eq!(slice.as_ptr(), nslice.as_ptr());
}

#[test]
fn otp_api_reads_blank_fuses() {
    let mut m = machine();
    assert_eq!(call(&mut m, OTP_INIT_API, &[0x0BB8_0000]), 0);
    m.bus.write_u32(u64::from(FUSE_WORD), 0xDEAD_BEEF).unwrap();
    assert_eq!(
        call(&mut m, OTP_FUSE_READ_API, &[FRO_192MHZ_SC_TRIM, FUSE_WORD]),
        0
    );
    assert_eq!(m.bus.read_u32(u64::from(FUSE_WORD)).unwrap(), 0);
    assert_eq!(call(&mut m, OTP_DEINIT_API, &[]), 0);
    // The same routine through the non-secure ROM address.
    m.bus.write_u32(u64::from(FUSE_WORD), 0xDEAD_BEEF).unwrap();
    assert_eq!(
        call(&mut m, OTP_FUSE_READ_API - 0x1000_0000, &[0x2B, FUSE_WORD]),
        0
    );
    assert_eq!(m.bus.read_u32(u64::from(FUSE_WORD)).unwrap(), 0);
}

#[test]
fn a_chip_without_mirrors_keeps_the_secure_range_unmapped() {
    let (mut chip, manifest) = chip();
    chip.memory_regions.retain(|r| r.alias_of.is_none());
    let bus = labwired_core::bus::SystemBus::from_config(&chip, &manifest).expect("bus");
    assert!(bus.mem_aliases.is_empty());
    assert!(bus.read_u32(0x3000_0000).is_err());
    assert!(bus.read_u32(0x1800_1000).is_err());
}
