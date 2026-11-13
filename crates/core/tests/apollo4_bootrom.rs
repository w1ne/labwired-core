// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! The in-tree clean-room Apollo4 Plus boot-ROM helper table
//! (`crates/core/roms/apollo4/apollo4_bootrom.S`), executed on the Cortex-M4
//! model through the chip descriptor's `bootrom` region. Every entry is
//! called at the address AmbiqSuite's `g_am_hal_bootrom_helper` uses.

mod common;
use common::root;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Cpu, Machine};

/// `b .` in SRAM: where LR returns.
const SENTINEL: u32 = 0x1000_6000;
const STACK: u32 = 0x1000_7F00;
const WORD: u32 = 0x1000_1000;

// am_hal_bootrom_helper.c (Thumb entry addresses).
const NV_MASS_ERASE: u32 = 0x0800_004D;
const NV_PAGE_ERASE: u32 = 0x0800_0051;
const NV_PROGRAM_MAIN: u32 = 0x0800_0055;
const NV_PROGRAM_INFO_AREA: u32 = 0x0800_0059;
const NV_PROGRAM_MAIN2: u32 = 0x0800_006D;
const READ_WORD: u32 = 0x0800_0075;
const WRITE_WORD: u32 = 0x0800_0079;
const NV_INFO_ERASE: u32 = 0x0800_0081;
const NV_RECOVERY: u32 = 0x0800_0099;
const DELAY_CYCLES: u32 = 0x0800_009D;

fn machine() -> Machine<CortexM> {
    let chip_path = root("configs/chips/amap42kp.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("amap42kp chip");
    let mut manifest =
        SystemManifest::from_file(root("configs/systems/apollo4p-evb.yaml")).expect("apollo4p-evb");
    manifest.chip = chip_path.to_str().expect("utf8").to_string();
    let mut bus = labwired_core::bus::SystemBus::from_config(&chip, &manifest).expect("bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut m = Machine::new(cpu, bus);
    m.bus.write_u16(u64::from(SENTINEL), 0xE7FE).unwrap();
    m
}

/// Call the ROM entry at `entry` with `args` in r0-r1; return r0 and the
/// cycles the call took.
fn call(m: &mut Machine<CortexM>, entry: u32, args: &[u32]) -> (u32, u64) {
    m.cpu.set_pc(entry & !1);
    m.cpu.set_sp(STACK);
    m.cpu.lr = SENTINEL | 1;
    let regs = [&mut m.cpu.r0, &mut m.cpu.r1];
    for (reg, &arg) in regs.into_iter().zip(args) {
        *reg = arg;
    }
    let start = m.total_cycles;
    for _ in 0..1_000_000 {
        if m.cpu.get_pc() == SENTINEL {
            assert_eq!(m.cpu.sp, STACK, "entry {entry:#x} unbalanced the stack");
            return (m.cpu.r0, m.total_cycles - start);
        }
        m.step()
            .unwrap_or_else(|e| panic!("entry {entry:#x}: {e:?}"));
    }
    panic!("entry {entry:#x} did not return (pc {:#x})", m.cpu.get_pc());
}

#[test]
fn read_and_write_word_are_single_accesses() {
    let mut m = machine();
    m.bus.write_u32(u64::from(WORD), 0xA5A5_5A5A).unwrap();
    assert_eq!(call(&mut m, READ_WORD, &[WORD]).0, 0xA5A5_5A5A);
    call(&mut m, WRITE_WORD, &[WORD, 0x1234_5678]);
    assert_eq!(m.bus.read_u32(u64::from(WORD)).unwrap(), 0x1234_5678);
}

#[test]
fn delay_cycles_spins_one_loop_per_iteration() {
    let mut m = machine();
    let (_, short) = call(&mut m, DELAY_CYCLES, &[1_000]);
    let (_, long) = call(&mut m, DELAY_CYCLES, &[11_000]);
    // am_hal_utils.h: "3 cycles per iteration" on silicon. The CPU model
    // charges subs 1 + taken bne 1 (no pipeline refill), so 2 here; see
    // apollo4_bootrom.S.
    let per_iteration = (long - short) as f64 / 10_000.0;
    assert_eq!(per_iteration, 2.0, "cycles per iteration");
    // Zero iterations returns at once.
    let (_, zero) = call(&mut m, DELAY_CYCLES, &[0]);
    assert!(zero < 20, "delay_cycles(0) took {zero} cycles");
}

#[test]
fn mram_helpers_report_failure_and_change_nothing() {
    let mut m = machine();
    let before = m.bus.read_u32(0x0002_0000).unwrap();
    for entry in [
        NV_MASS_ERASE,
        NV_PAGE_ERASE,
        NV_PROGRAM_MAIN,
        NV_PROGRAM_INFO_AREA,
        NV_PROGRAM_MAIN2,
        NV_INFO_ERASE,
        NV_RECOVERY,
    ] {
        assert_eq!(call(&mut m, entry, &[0, 0]).0 as i32, -1, "{entry:#x}");
    }
    assert_eq!(m.bus.read_u32(0x0002_0000).unwrap(), before);
}
