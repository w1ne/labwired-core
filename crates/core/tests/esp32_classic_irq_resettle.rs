// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Classic ESP32: a level-1 ISR must see its own source in `RSR.INTERRUPT`.
//!
//! On interrupt entry `XtensaLx7::dispatch_irq` clears the ROUTED bit of every
//! slot at the taken level, then calls `Bus::resettle_cpu_irq_levels` so a
//! source that is still asserting routes straight back. ESP-IDF's level-1
//! dispatcher (`_xt_lowint1`) depends on that: it reads `RSR.INTERRUPT &
//! INTENABLE` to pick the handler, and a zero read calls no handler at all.
//!
//! The resettle used to cover only the walk-free ESP32-S3. On the classic
//! (DPORT) bus a scheduler-driven source such as I2C0 lives only in the
//! scheduler-source set, and no per-cycle walk re-derives it. So the routed bit
//! stayed cleared, the I2C ISR never ran, and every Arduino `Wire` transfer on
//! ESP32 waited out its timeout. The arduino matrix's `esp32 × L3_i2c_sensor`
//! cell printed `LW_L3_FAIL nack` (labwired-core#1712).
//!
//! # Negative control
//!
//! Remove the `recompute_esp32_classic_irq_lines()` call from
//! `resettle_cpu_irq_levels` in `crates/core/src/bus/accessors.rs`. The ISR
//! then stores 0 and `level1_isr_reads_the_i2c_slot_in_rsr_interrupt` fails.

#![cfg(feature = "event-scheduler")]

use labwired_core::bus::SystemBus;
use labwired_core::cpu::xtensa_lx7::XtensaLx7;
use labwired_core::system::xtensa::configure_xtensa_esp32;
use labwired_core::{Bus, Cpu, DebugControl, Machine};

const IRAM_BASE: u32 = 0x4008_0000;
const ISR_PC: u32 = IRAM_BASE + 0x1000;
/// PS.UM = 0, so a level-1 interrupt enters the kernel vector at VECBASE+0x300.
const VECBASE_VALUE: u32 = ISR_PC - 0x300;
/// The ISR stores `RSR.INTERRUPT` here.
const WITNESS: u32 = 0x3FFB_0000;
const WITNESS_UNSET: u32 = 0xDEAD_BEEF;

const DPORT_BASE: u32 = 0x3FF0_0000;
/// `DPORT_PRO_MAC_INTR_MAP_REG`; source `s` lives at `+ s*4`.
const DPORT_PRO_MAP_BASE: u32 = DPORT_BASE + 0x104;
/// `ETS_I2C_EXT0_INTR_SOURCE`.
const I2C0_SOURCE: u32 = 49;
/// A level-1 CPU interrupt (`IRQ_LEVELS[3] == 1`), the slot the Arduino
/// sketch's I2C driver got on the APP core.
const CPU_IRQ_SLOT: u32 = 3;

const I2C0_BASE: u32 = 0x3FF5_3000;
const I2C_CTR: u32 = I2C0_BASE + 0x04;
const I2C_INT_CLR: u32 = I2C0_BASE + 0x24;
const I2C_INT_ENA: u32 = I2C0_BASE + 0x28;
const I2C_CMD0: u32 = I2C0_BASE + 0x58;
const CTR_TRANS_START: u32 = 1 << 5;
const INT_TRANS_COMPLETE: u32 = 1 << 7;
/// Classic opcode STOP (3) in COMD[13:11]: a one-command list that completes.
const CMD_STOP: u32 = 3 << 11;

/// `j 0`: spin in place.
const SPIN_BYTES: &[u8] = &[0x06, 0xff, 0xff];
/// The ISR: `rsr a2, interrupt` (SR 226), `s32i.n a2, a3, 0`, then spin.
/// It does not ack the I2C source, so the level stays asserted.
const ISR_BYTES: &[u8] = &[
    0x20, 0xE2, 0x03, // rsr.interrupt a2
    0x29, 0x03, // s32i.n a2, a3, 0
    0x06, 0xff, 0xff, // j .
];

fn build(start_transfer: bool) -> Machine<XtensaLx7> {
    let mut bus = SystemBus::empty();
    let mut cpu = configure_xtensa_esp32(&mut bus);

    for (i, &b) in SPIN_BYTES.iter().enumerate() {
        bus.write_u8((IRAM_BASE + i as u32) as u64, b).unwrap();
    }
    for (i, &b) in ISR_BYTES.iter().enumerate() {
        bus.write_u8((ISR_PC + i as u32) as u64, b).unwrap();
    }
    bus.write_u32(WITNESS as u64, WITNESS_UNSET).unwrap();

    bus.write_u32((DPORT_PRO_MAP_BASE + I2C0_SOURCE * 4) as u64, CPU_IRQ_SLOT)
        .unwrap();
    bus.write_u32(I2C_INT_CLR as u64, 0xFFFF_FFFF).unwrap();
    bus.write_u32(I2C_INT_ENA as u64, INT_TRANS_COMPLETE)
        .unwrap();
    bus.write_u32(I2C_CMD0 as u64, CMD_STOP).unwrap();
    if start_transfer {
        // The command list runs inside this write and latches TRANS_COMPLETE.
        bus.write_u32(I2C_CTR as u64, CTR_TRANS_START).unwrap();
    }

    use labwired_core::cpu::xtensa_sr::{INTENABLE, VECBASE};
    cpu.sr.write(VECBASE, VECBASE_VALUE);
    cpu.sr.write(INTENABLE, 1u32 << CPU_IRQ_SLOT);
    cpu.regs.write_logical(3, WITNESS);
    cpu.ps.set_intlevel(0);
    cpu.ps.set_excm(false);
    cpu.set_pc(IRAM_BASE);

    Machine::new(cpu, bus)
}

fn witness_after(machine: &mut Machine<XtensaLx7>, steps: u32) -> u32 {
    machine.run(Some(steps)).expect("run classic ESP32 machine");
    machine.bus.read_u32(WITNESS as u64).unwrap()
}

/// Control: with no transfer there is no source, so the ISR never runs.
#[test]
fn no_transfer_no_isr() {
    let mut machine = build(false);
    assert_eq!(
        witness_after(&mut machine, 2_000),
        WITNESS_UNSET,
        "the ISR ran with no I2C0 level asserted"
    );
}

/// The regression.
#[test]
fn level1_isr_reads_the_i2c_slot_in_rsr_interrupt() {
    let mut machine = build(true);
    let seen = witness_after(&mut machine, 2_000);
    assert_ne!(
        seen, WITNESS_UNSET,
        "the I2C0 interrupt was never taken: the level did not reach PRO slot {CPU_IRQ_SLOT}"
    );
    assert_ne!(
        seen & (1 << CPU_IRQ_SLOT),
        0,
        "the ISR read RSR.INTERRUPT = {seen:#x}: dispatch cleared the routed I2C0 bit \
         and nothing re-derived it while the source still asserts, so `_xt_lowint1` \
         would call no handler (labwired-core#1712)"
    );
}
