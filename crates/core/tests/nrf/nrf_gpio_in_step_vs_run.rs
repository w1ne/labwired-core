// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Single-step versus batched run on the nRF GPIO input the step path reads
//! every instruction.
//!
//! The perf gate's step column calls `AdvanceRequest::single` (tick interval
//! 1). The shipped fast path calls `AdvanceRequest::run` at
//! `max_safe_tick_interval`. Both must retire the same Thumb program to the
//! same PC, registers, SRAM bytes, and GPIO port registers. The input word is
//! the one the bus edge pass samples; a cached pull mask that disagrees with
//! PIN_CNF shows up as a wrong IN bit, on either lane.

use crate::common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{AdvanceRequest, AdvanceStop, Bus, Machine};

const CODE: u64 = 0x2000_0000;
const P0: u64 = 0x5000_0000;
const P1: u64 = 0x5000_1000;
const IN: u64 = 0x510;
const OUT: u64 = 0x504;
const DIR: u64 = 0x514;
const PULLUP: u32 = 3 << 2;
const PULLDOWN: u32 = 1 << 2;
const STEPS: u64 = 1536;

fn pin_cnf(port: u64, pin: u64) -> u64 {
    port + 0x700 + 4 * pin
}

fn machine() -> Machine<labwired_core::cpu::CortexM> {
    let chip_path = root("configs/chips/nrf52840.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("nrf52840 chip");
    let manifest = SystemManifest {
        name: "nrf-gpio-step-vs-run".into(),
        chip: chip_path.to_string_lossy().into_owned(),
        ..SystemManifest::default()
    };
    let mut bus = labwired_core::bus::SystemBus::from_config(&chip, &manifest).expect("nRF bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    // Product default: idle fast-forward is on, and `single()` ignores it.
    machine.config.idle_fast_forward_enabled = true;
    machine.bus.write_u16(CODE, 0x2000).unwrap(); // movs r0, #0
    machine.bus.write_u16(CODE + 2, 0x3001).unwrap(); // adds r0, #1
    machine.bus.write_u16(CODE + 4, 0xE7FD).unwrap(); // b to the adds
    machine.cpu.pc = CODE as u32;
    machine.cpu.sp = 0x2001_0000;
    machine.cpu.primask = true;
    machine.bus.write_u32(pin_cnf(P0, 19), PULLUP).unwrap();
    machine.bus.write_u32(pin_cnf(P0, 1), PULLDOWN).unwrap();
    machine.bus.write_u32(pin_cnf(P0, 8), 1 | PULLUP).unwrap();
    machine.bus.write_u32(P0 + OUT, 1 << 8).unwrap();
    machine.bus.write_u32(pin_cnf(P1, 0), PULLUP).unwrap();
    machine
}

fn retire(machine: &mut Machine<labwired_core::cpu::CortexM>, batched: bool, steps: u64) {
    if batched {
        let interval = machine.bus.max_safe_tick_interval();
        machine.config.peripheral_tick_interval = interval;
        machine.bus.config.peripheral_tick_interval = interval;
        let report = machine
            .advance(AdvanceRequest::run(Some(steps)))
            .expect("batched advance");
        assert_eq!(report.stop, AdvanceStop::FuelLimit, "{report:?}");
        assert_eq!(report.fuel_consumed, steps, "{report:?}");
    } else {
        machine.config.peripheral_tick_interval = 1;
        machine.bus.config.peripheral_tick_interval = 1;
        for _ in 0..steps {
            let report = machine
                .advance(AdvanceRequest::single())
                .expect("single advance");
            assert_eq!(report.fuel_consumed, 1, "{report:?}");
        }
    }
}

struct Arch {
    pc: u32,
    r0: u32,
    regs: [u32; 12],
    sp: u32,
    lr: u32,
    xpsr: u32,
    cycles: u64,
    p0_in: u32,
    p0_out: u32,
    p0_dir: u32,
    p1_in: u32,
    p1_dir: u32,
    code: [u16; 3],
}

fn arch(machine: &Machine<labwired_core::cpu::CortexM>) -> Arch {
    let cpu = &machine.cpu;
    Arch {
        pc: cpu.pc,
        r0: cpu.r0,
        regs: [
            cpu.r1, cpu.r2, cpu.r3, cpu.r4, cpu.r5, cpu.r6, cpu.r7, cpu.r8, cpu.r9, cpu.r10,
            cpu.r11, cpu.r12,
        ],
        sp: cpu.sp,
        lr: cpu.lr,
        xpsr: cpu.xpsr,
        cycles: machine.total_cycles,
        p0_in: machine.bus.read_u32(P0 + IN).unwrap(),
        p0_out: machine.bus.read_u32(P0 + OUT).unwrap(),
        p0_dir: machine.bus.read_u32(P0 + DIR).unwrap(),
        p1_in: machine.bus.read_u32(P1 + IN).unwrap(),
        p1_dir: machine.bus.read_u32(P1 + DIR).unwrap(),
        code: [
            machine.bus.read_u16(CODE).unwrap(),
            machine.bus.read_u16(CODE + 2).unwrap(),
            machine.bus.read_u16(CODE + 4).unwrap(),
        ],
    }
}

fn assert_same(left: &Arch, right: &Arch, what: &str) {
    assert_eq!(left.pc, right.pc, "{what} pc");
    assert_eq!(left.r0, right.r0, "{what} r0");
    assert_eq!(left.regs, right.regs, "{what} r1-r12");
    assert_eq!(left.sp, right.sp, "{what} sp");
    assert_eq!(left.lr, right.lr, "{what} lr");
    assert_eq!(left.xpsr, right.xpsr, "{what} xpsr");
    assert_eq!(left.cycles, right.cycles, "{what} cycles");
    assert_eq!(left.p0_in, right.p0_in, "{what} P0.IN");
    assert_eq!(left.p0_out, right.p0_out, "{what} P0.OUT");
    assert_eq!(left.p0_dir, right.p0_dir, "{what} P0.DIR");
    assert_eq!(left.p1_in, right.p1_in, "{what} P1.IN");
    assert_eq!(left.p1_dir, right.p1_dir, "{what} P1.DIR");
    assert_eq!(left.code, right.code, "{what} SRAM program");
}

fn assert_pulls(state: &Arch, pin19_high: bool, adds: u32, what: &str) {
    let bit19 = state.p0_in & (1 << 19);
    if pin19_high {
        assert_eq!(bit19, 1 << 19, "{what}: P0.19 pull-up reads high");
    } else {
        assert_eq!(bit19, 0, "{what}: P0.19 pull-down reads low");
    }
    assert_eq!(state.p0_in & (1 << 1), 0, "{what}: P0.01 pull-down");
    assert_eq!(state.p0_in & (1 << 8), 1 << 8, "{what}: P0.08 output high");
    assert_eq!(
        state.p0_dir & (1 << 8),
        1 << 8,
        "{what}: P0.08 is an output"
    );
    assert_eq!(state.p1_in & 1, 1, "{what}: P1.00 pull-up reads high");
    assert_eq!(state.r0, adds, "{what}: adds retired");
    assert_eq!(state.pc, (CODE + 4) as u32, "{what}: stopped on the branch");
}

#[test]
fn nrf52840_gpio_in_single_step_matches_batched_run() {
    let mut stepped = machine();
    let mut batched = machine();
    let interval = batched.bus.max_safe_tick_interval();
    if interval > 1 {
        assert!(
            interval >= 1024,
            "the batched lane must actually widen the tick, got {interval}"
        );
    }

    retire(&mut stepped, false, STEPS);
    retire(&mut batched, true, STEPS);
    let step_mid = arch(&stepped);
    let batch_mid = arch(&batched);
    assert_same(&step_mid, &batch_mid, "after the first run");
    assert_pulls(&step_mid, true, 768, "after the first run");

    stepped.bus.write_u32(pin_cnf(P0, 19), PULLDOWN).unwrap();
    batched.bus.write_u32(pin_cnf(P0, 19), PULLDOWN).unwrap();
    retire(&mut stepped, false, STEPS);
    retire(&mut batched, true, STEPS);
    let step_end = arch(&stepped);
    let batch_end = arch(&batched);
    assert_same(&step_end, &batch_end, "after the pull change");
    assert_pulls(&step_end, false, 1536, "after the pull change");
}
