// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! MSP430 core unit tests. Every instruction encoding below was produced by
//! the TI `msp430-elf-as` 2.34 (msp430-gcc 9.3.1.11) from the source in the
//! comment above it. Expected values follow the user's guides (SLAU049 for
//! the 16-bit CPU, SLAU144 for the CPUX).
//!
//! These tests hold the behaviour the GDB-simulator lockstep
//! (`crates/core/tests/vendor/msp430_gdb_sim_lockstep.rs`) cannot: the points
//! where that simulator departs from the guides, and interrupts, low-power
//! modes and reset, which it does not model.

use super::*;
use crate::peripherals::msp430::{sfr_source, Msp430Irq};
use crate::{Bus, Cpu, DmaRequest, SimulationConfig};

const CODE: u32 = 0x4000;
const STACK: u32 = 0x0A00;

/// 1 MiB of flat memory.
struct Flat {
    mem: Vec<u8>,
    config: SimulationConfig,
}

impl Bus for Flat {
    fn read_u8(&self, addr: u64) -> SimResult<u8> {
        self.mem
            .get(addr as usize)
            .copied()
            .ok_or(SimulationError::MemoryViolation(addr))
    }
    fn write_u8(&mut self, addr: u64, value: u8) -> SimResult<()> {
        *self
            .mem
            .get_mut(addr as usize)
            .ok_or(SimulationError::MemoryViolation(addr))? = value;
        Ok(())
    }
    fn tick_peripherals(&mut self) -> Vec<u32> {
        Vec::new()
    }
    fn execute_dma(&mut self, _requests: &[DmaRequest]) -> SimResult<()> {
        Ok(())
    }
    fn config(&self) -> &SimulationConfig {
        &self.config
    }
}

struct Rig {
    cpu: Msp430,
    bus: Flat,
}

impl Rig {
    fn with(mut cpu: Msp430, code: &[u8]) -> Self {
        let mut bus = Flat {
            mem: vec![0; 1 << 20],
            config: SimulationConfig::default(),
        };
        bus.mem[CODE as usize..CODE as usize + code.len()].copy_from_slice(code);
        cpu.r[0] = CODE;
        cpu.r[1] = STACK;
        Self { cpu, bus }
    }
    fn cpu(code: &[u8]) -> Self {
        Self::with(Msp430::new(), code)
    }
    fn cpux(code: &[u8]) -> Self {
        Self::with(Msp430::new_cpux(), code)
    }
    fn step(&mut self) {
        let config = SimulationConfig::default();
        self.cpu.step(&mut self.bus, &[], &config).unwrap();
    }
    fn run(&mut self, n: usize) {
        for _ in 0..n {
            self.step();
        }
    }
    fn w16(&mut self, addr: u32, v: u16) {
        self.bus.write_u16(u64::from(addr), v).unwrap();
    }
    fn r16(&self, addr: u32) -> u16 {
        self.bus.read_u16(u64::from(addr)).unwrap()
    }
    fn flags(&self) -> u32 {
        self.cpu.sr() & (SR_V | SR_N | SR_Z | SR_C)
    }
}

// sxt r6
const SXT: &[u8] = &[0x86, 0x11];

#[test]
fn sxt_sets_carry_to_not_zero() {
    let mut m = Rig::cpu(SXT);
    m.cpu.r[6] = 0xFF1F;
    m.step();
    assert_eq!(m.cpu.r[6], 0x001F);
    assert_eq!(m.flags(), SR_C);
    let mut m = Rig::cpu(SXT);
    m.cpu.r[6] = 0x1280;
    m.cpu.r[2] = SR_V;
    m.step();
    assert_eq!(m.cpu.r[6], 0xFF80);
    assert_eq!(m.flags(), SR_N | SR_C, "V is reset");
    let mut m = Rig::cpu(SXT);
    m.cpu.r[6] = 0x1200;
    m.cpu.r[2] = SR_C;
    m.step();
    assert_eq!(m.cpu.r[6], 0);
    assert_eq!(m.flags(), SR_Z);
}

#[test]
fn sxt_fills_a_cpux_register_to_bit_19() {
    let mut m = Rig::cpux(SXT);
    m.cpu.r[6] = 0x5_1280;
    m.step();
    assert_eq!(m.cpu.r[6], 0xF_FF80);
    let mut m = Rig::cpux(SXT);
    m.cpu.r[6] = 0xF_FF7F;
    m.step();
    assert_eq!(m.cpu.r[6], 0x0_007F);
}

// sub r5, r6 ; cmp r5, r6
const SUB_CMP: &[u8] = &[0x06, 0x85, 0x06, 0x95];

#[test]
fn subtract_and_compare_set_overflow_alike() {
    // 0 - (-32768) does not fit: V set, and the borrow clears C.
    let mut m = Rig::cpu(SUB_CMP);
    m.cpu.r[5] = 0x8000;
    m.cpu.r[6] = 0;
    m.step();
    assert_eq!(m.cpu.r[6], 0x8000);
    assert_eq!(m.flags(), SR_V | SR_N);
    // 0x8000 - 0x8000 = 0: no borrow, so C is set.
    m.step();
    assert_eq!(m.flags(), SR_Z | SR_C);
    let mut m = Rig::cpu(&SUB_CMP[2..]);
    m.cpu.r[5] = 0x8000;
    m.cpu.r[6] = 0;
    m.step();
    assert_eq!(m.cpu.r[6], 0, "CMP stores nothing");
    assert_eq!(m.flags(), SR_V | SR_N);
}

// rrc r6 ; rra r6
const RRC_RRA: &[u8] = &[0x06, 0x10, 0x06, 0x11];

#[test]
fn rotates_reset_overflow() {
    let mut m = Rig::cpu(RRC_RRA);
    m.cpu.r[6] = 0x322F;
    m.cpu.r[2] = SR_C | SR_V;
    m.step();
    assert_eq!(m.cpu.r[6], 0x9917);
    assert_eq!(m.flags(), SR_N | SR_C);
    m.cpu.r[2] |= SR_V;
    m.step();
    assert_eq!(m.cpu.r[6], 0xCC8B);
    assert_eq!(m.flags(), SR_N | SR_C);
}

// pop.b r7        (mov.b @sp+, r7)
const POP_B: &[u8] = &[0x77, 0x41];

#[test]
fn a_byte_pop_moves_sp_by_two() {
    let mut m = Rig::cpu(POP_B);
    m.w16(STACK, 0xABCD);
    m.step();
    assert_eq!(m.cpu.r[7], 0xCD);
    assert_eq!(m.cpu.r[1], STACK + 2);
}

// call <symbolic>: the assembler's `90 12` followed by an index word chosen
// here. The index word is at 0x4002, so the operand is at
// (0x4002 + 0xd0f2) mod 64 KiB = 0x10f4.
const CALL_SYMBOLIC: &[u8] = &[0x90, 0x12, 0xF2, 0xD0];

#[test]
fn a_symbolic_operand_wraps_at_64k() {
    for mut m in [Rig::cpu(CALL_SYMBOLIC), Rig::cpux(CALL_SYMBOLIC)] {
        m.w16(0x10F4, 0x5000);
        m.step();
        assert_eq!(m.cpu.get_pc(), 0x5000);
        assert_eq!(m.cpu.r[1], STACK - 2);
        assert_eq!(m.r16(STACK - 2), 0x4004, "return address");
    }
}

// push sp
const PUSH_SP: &[u8] = &[0x01, 0x12];

#[test]
fn push_stores_the_operand_read_before_the_decrement() {
    let mut m = Rig::cpu(PUSH_SP);
    m.step();
    assert_eq!(m.cpu.r[1], STACK - 2);
    assert_eq!(m.r16(STACK - 2), STACK as u16);
}

// mov.b r5, &0x0200 ; inc.b &0x0200
const BYTE_STORE: &[u8] = &[0xC2, 0x45, 0x00, 0x02, 0xD2, 0x53, 0x00, 0x02];

#[test]
fn byte_operations_leave_the_neighbouring_byte_alone() {
    let mut m = Rig::cpu(BYTE_STORE);
    m.w16(0x0200, 0x7700);
    m.cpu.r[5] = 0x12FF;
    m.run(2);
    assert_eq!(m.r16(0x0200), 0x7700);
    assert_eq!(m.flags(), SR_Z | SR_C);
}

// add r5, r6
const ADD: &[u8] = &[0x06, 0x55];

#[test]
fn the_16_bit_cpu_has_16_bit_registers() {
    let mut m = Rig::cpu(ADD);
    m.cpu.r[5] = 0xFFFF;
    m.cpu.r[6] = 0x0002;
    m.step();
    assert_eq!(m.cpu.r[6], 0x0001);
    assert_eq!(m.flags(), SR_C);
    // A non-extended instruction clears bits 19:16 of a CPUX register.
    let mut m = Rig::cpux(ADD);
    m.cpu.r[5] = 0xA_FFFF;
    m.cpu.r[6] = 0x5_0002;
    m.step();
    assert_eq!(m.cpu.r[6], 0x0_0001);
}

// mova #0x12345, r6
const MOVA: &[u8] = &[0x86, 0x01, 0x45, 0x23];

#[test]
fn the_16_bit_cpu_refuses_cpux_encodings() {
    let mut m = Rig::cpu(MOVA);
    let config = SimulationConfig::default();
    let err = m.cpu.step(&mut m.bus, &[], &config).unwrap_err();
    assert!(matches!(err, SimulationError::DecodeError(a) if a == u64::from(CODE)));
    assert_eq!(m.cpu.get_pc(), CODE, "PC stays on the instruction");
    let mut m = Rig::cpux(MOVA);
    m.step();
    assert_eq!(m.cpu.r[6], 0x1_2345);
}

// swpbx.a r6 ; swpbx.w r6
const SWPBX: &[u8] = &[0x00, 0x18, 0x86, 0x10, 0x40, 0x18, 0x86, 0x10];

#[test]
fn swpbx_a_keeps_the_upper_bits_and_swpbx_w_clears_them() {
    let mut m = Rig::cpux(SWPBX);
    m.cpu.r[6] = 0x7_12AB;
    m.step();
    assert_eq!(m.cpu.r[6], 0x7_AB12);
    m.step();
    assert_eq!(m.cpu.r[6], 0x0_12AB);
}

// daddx.a r4, r5
const DADDX_A: &[u8] = &[0x00, 0x18, 0x45, 0xA4];

#[test]
fn daddx_a_adds_five_decimal_digits() {
    let mut m = Rig::cpux(DADDX_A);
    m.cpu.r[4] = 0x4_5678;
    m.cpu.r[5] = 0x5_4321;
    m.step();
    assert_eq!(m.cpu.r[5], 0x9_9999);
    assert_eq!(m.flags() & (SR_C | SR_Z), 0);
    let mut m = Rig::cpux(DADDX_A);
    m.cpu.r[4] = 0x4_5678;
    m.cpu.r[5] = 0x5_4321;
    m.cpu.r[2] = SR_C;
    m.step();
    assert_eq!(m.cpu.r[5], 0);
    assert_eq!(m.flags() & (SR_C | SR_Z), SR_C | SR_Z);
}

// calla -4(r10)
const CALLA_INDEXED: &[u8] = &[0x5A, 0x13, 0xFC, 0xFF];

#[test]
fn calla_takes_a_signed_index_and_pushes_20_bits() {
    let mut m = Rig::cpux(CALLA_INDEXED);
    m.cpu.r[10] = 0x2160;
    m.w16(0x215C, 0x0008);
    m.w16(0x215E, 0x0001);
    m.step();
    assert_eq!(m.cpu.get_pc(), 0x1_0008);
    assert_eq!(m.cpu.r[1], STACK - 4);
    assert_eq!(m.r16(STACK - 4), 0x4004);
    assert_eq!(m.r16(STACK - 2), 0x0000);
}

// mov 0x1210(r10), r7
const INDEXED: &[u8] = &[0x17, 0x4A, 0x10, 0x12];

#[test]
fn a_16_bit_index_wraps_below_64k_and_sign_extends_above() {
    let mut m = Rig::cpux(INDEXED);
    m.cpu.r[10] = 0xFFF0;
    m.w16(0x1200, 0x7777);
    m.w16(0x1_1200, 0x1111);
    m.step();
    assert_eq!(m.cpu.r[7], 0x7777, "0xFFF0 + 0x1210 wraps to 0x1200");
    let mut m = Rig::cpux(INDEXED);
    m.cpu.r[10] = 0x1_0000;
    m.w16(0x1_1210, 0x2222);
    m.step();
    assert_eq!(m.cpu.r[7], 0x2222);
}

// rpt #4 { rrax.a r6 ; rpt r7 { rlax.a r6 ; rrux.w r6
const REPEATS: &[u8] = &[
    0x03, 0x18, 0x46, 0x11, 0x87, 0x18, 0x46, 0x56, 0x40, 0x19, 0x06, 0x10,
];

#[test]
fn the_extension_word_repeats_register_instructions() {
    let mut m = Rig::cpux(REPEATS);
    m.cpu.r[6] = 0x8_0018;
    m.step();
    assert_eq!(m.cpu.r[6], 0xF_8001, "four arithmetic shifts right");
    assert_eq!(m.flags() & SR_C, SR_C);
    // The count comes from R7[3:0] + 1.
    m.cpu.r[7] = 0x32;
    m.step();
    assert_eq!(m.cpu.r[6], 0xC_0008, "three shifts left");
    // RRUX.W: the carry in is zero whatever SR.C holds.
    m.cpu.r[2] |= SR_C;
    m.cpu.r[6] = 0xC_0009;
    m.step();
    assert_eq!(m.cpu.r[6], 0x0_0004);
    assert_eq!(m.flags(), SR_C);
}

// movx.a #0xabcde, &0x10200
const MOVX_A: &[u8] = &[0x01, 0x1D, 0xF2, 0x40, 0xDE, 0xBC, 0x00, 0x02];

#[test]
fn an_address_word_in_memory_is_two_words() {
    let mut m = Rig::cpux(MOVX_A);
    m.w16(0x1_0202, 0xFFFF);
    m.step();
    assert_eq!(m.r16(0x1_0200), 0xBCDE);
    assert_eq!(m.r16(0x1_0202), 0x000A);
    assert_eq!(m.cpu.get_pc(), CODE + 8);
}

// ---- interrupts, low-power modes, reset -----------------------------------

const VECTOR_6: u32 = RESET_VECTOR - 12;
const HANDLER: u16 = 0x6000;

/// A core with source IFG1.6 (single-source) routed to line 6.
fn with_irq(mut m: Rig) -> (Rig, std::sync::Arc<Msp430Irq>) {
    let irq = Msp430Irq::new();
    irq.route(sfr_source(1, 6), 6, true);
    irq.route(sfr_source(1, 0), 5, true);
    m.cpu.attach_irq(irq.clone());
    m.w16(VECTOR_6, HANDLER);
    m.w16(RESET_VECTOR - 10, 0x6100);
    (m, irq)
}

// nop ; nop
const NOPS: &[u8] = &[0x03, 0x43, 0x03, 0x43];

#[test]
fn an_interrupt_needs_its_flag_its_enable_and_gie() {
    let (mut m, irq) = with_irq(Rig::cpu(NOPS));
    let source = sfr_source(1, 6);
    irq.set_flag(source, true);
    m.cpu.r[2] = SR_GIE | SR_C;
    m.step();
    assert_eq!(m.cpu.get_pc(), CODE + 2, "the enable is clear");
    irq.set_enable(source, true);
    m.cpu.r[2] = SR_C;
    m.cpu.r[0] = CODE;
    m.step();
    assert_eq!(m.cpu.get_pc(), CODE + 2, "GIE is clear");
    m.cpu.r[2] = SR_GIE | SR_C;
    m.cpu.r[0] = CODE;
    m.step();
    assert_eq!(m.cpu.get_pc(), u32::from(HANDLER));
    assert_eq!(m.cpu.sr(), 0, "entry clears GIE and the status bits");
    assert_eq!(m.cpu.r[1], STACK - 4);
    assert_eq!(m.r16(STACK - 2), (CODE + 2) as u16, "PC pushed first");
    assert_eq!(m.r16(STACK - 4), (SR_GIE | SR_C) as u16);
    assert!(!irq.flag(source), "a single-source flag is cleared");
}

#[test]
fn the_lowest_line_number_wins() {
    let (mut m, irq) = with_irq(Rig::cpu(NOPS));
    for source in [sfr_source(1, 6), sfr_source(1, 0)] {
        irq.set_flag(source, true);
        irq.set_enable(source, true);
    }
    m.cpu.r[2] = SR_GIE;
    m.step();
    assert_eq!(m.cpu.get_pc(), 0x6100, "line 5 before line 6");
    assert!(irq.flag(sfr_source(1, 6)), "line 6 stays pending");
}

#[test]
fn the_nmi_ignores_gie_and_a_requested_line_is_taken_once() {
    let (mut m, irq) = with_irq(Rig::cpu(NOPS));
    irq.route(sfr_source(1, 4), 1, false);
    irq.set_flag(sfr_source(1, 4), true);
    irq.set_enable(sfr_source(1, 4), true);
    m.w16(RESET_VECTOR - 2, 0x6200);
    m.step();
    assert_eq!(m.cpu.get_pc(), 0x6200);
    assert!(
        !irq.enabled(sfr_source(1, 4)),
        "NMIIE is cleared on acceptance"
    );

    let (mut m, _irq) = with_irq(Rig::cpu(NOPS));
    m.cpu.set_exception_pending(6);
    m.cpu.r[2] = SR_GIE;
    m.step();
    assert_eq!(m.cpu.get_pc(), u32::from(HANDLER));
    m.cpu.r[2] = SR_GIE;
    m.cpu.r[0] = CODE;
    m.step();
    assert_eq!(m.cpu.get_pc(), CODE + 2, "the request was consumed");
}

// eint ; nop ; nop
const EINT: &[u8] = &[0x32, 0xD2, 0x03, 0x43, 0x03, 0x43];

#[test]
fn the_instruction_after_eint_runs_before_the_interrupt() {
    let (mut m, irq) = with_irq(Rig::cpu(EINT));
    irq.set_flag(sfr_source(1, 6), true);
    irq.set_enable(sfr_source(1, 6), true);
    m.step();
    assert_eq!(m.cpu.get_pc(), CODE + 2, "EINT itself is not interrupted");
    m.step();
    assert_eq!(m.cpu.get_pc(), u32::from(HANDLER));
    assert_eq!(
        m.r16(STACK - 2),
        (CODE + 4) as u16,
        "the NOP after EINT ran"
    );
}

// bis #0x18, sr ; nop        (GIE + CPUOFF: LPM0)
const LPM0: &[u8] = &[0x32, 0xD0, 0x18, 0x00, 0x03, 0x43];
// reti
const RETI: &[u8] = &[0x00, 0x13];

#[test]
fn cpuoff_stops_the_core_until_an_interrupt_and_reti_returns_to_sleep() {
    let (mut m, irq) = with_irq(Rig::cpu(LPM0));
    m.bus.mem[usize::from(HANDLER)..usize::from(HANDLER) + 2].copy_from_slice(RETI);
    irq.set_enable(sfr_source(1, 6), true);
    m.step();
    assert_eq!(m.cpu.sr(), SR_GIE | SR_CPUOFF);
    assert_eq!(m.cpu.idle_fast_forward_budget(&m.bus), Some(u64::MAX));
    m.run(5);
    assert_eq!(m.cpu.get_pc(), CODE + 4, "asleep: the NOP does not run");
    irq.set_flag(sfr_source(1, 6), true);
    assert_eq!(m.cpu.idle_fast_forward_budget(&m.bus), None);
    m.step();
    assert_eq!(m.cpu.get_pc(), u32::from(HANDLER));
    assert_eq!(m.cpu.sr(), 0, "the handler runs with the CPU on");
    m.step();
    assert_eq!(m.cpu.get_pc(), CODE + 4);
    assert_eq!(m.cpu.sr(), SR_GIE | SR_CPUOFF, "RETI restores the sleep");
    m.step();
    assert_eq!(m.cpu.get_pc(), CODE + 4);
}

#[test]
fn a_cpux_interrupt_frame_carries_the_upper_pc_bits_in_the_saved_sr() {
    let (mut m, irq) = with_irq(Rig::cpux(NOPS));
    m.bus.mem[0x1_2000..0x1_2002].copy_from_slice(&NOPS[..2]);
    m.bus.mem[usize::from(HANDLER)..usize::from(HANDLER) + 2].copy_from_slice(RETI);
    irq.set_flag(sfr_source(1, 6), true);
    irq.set_enable(sfr_source(1, 6), true);
    m.cpu.r[0] = 0x1_2000;
    m.cpu.r[1] = 0x1_1000;
    m.cpu.r[2] = SR_GIE | SR_N;
    m.step();
    assert_eq!(m.cpu.get_pc(), u32::from(HANDLER));
    assert_eq!(m.r16(0x1_0FFE), 0x2002);
    assert_eq!(m.r16(0x1_0FFC), 0x1000 | (SR_GIE | SR_N) as u16);
    m.step();
    assert_eq!(m.cpu.get_pc(), 0x1_2002);
    assert_eq!(m.cpu.sr(), SR_GIE | SR_N);
    assert_eq!(m.cpu.r[1], 0x1_1000);
}

#[test]
fn reset_starts_at_the_reset_vector_with_sr_clear() {
    let mut m = Rig::cpux(NOPS);
    m.w16(RESET_VECTOR, 0x4401);
    m.cpu.r[2] = SR_GIE | SR_CPUOFF;
    m.cpu.r[9] = 7;
    m.cpu.set_exception_pending(6);
    m.cpu.reset(&mut m.bus).unwrap();
    assert_eq!(m.cpu.get_pc(), 0x4400, "PC bit 0 is always clear");
    assert_eq!(m.cpu.sr(), 0);
    assert_eq!(m.cpu.r[9], 0);
    assert!(m.cpu.is_cpux(), "a reset keeps the CPU type");
}

#[test]
fn a_snapshot_restores_every_register() {
    let mut m = Rig::cpux(NOPS);
    m.cpu.r[4] = 0xA_BCDE;
    m.cpu.r[2] = SR_GIE | SR_V;
    m.cpu.set_exception_pending(9);
    let snap = m.cpu.snapshot();
    let mut other = Msp430::new();
    other.apply_snapshot(&snap);
    assert!(other.is_cpux());
    assert_eq!(other.r, m.cpu.r);
    assert_eq!(other.requested, 1 << 9);
}
