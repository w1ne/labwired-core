// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Thumb semihosting trap for `bkpt #0xAB` (ARM IHI 0042).
//!
//! Not inlined into `exec_bkpt`: that arm stays a branch so `step_execute`
//! does not grow. `SYS_EXIT` latches a code and does not return to the guest;
//! `Machine::advance` observes it through `Cpu::take_firmware_exit`.

use super::{CortexM, PcAdvance};
use crate::{Bus, SimResult};

const SYS_WRITEC: u32 = 0x03;
const SYS_WRITE0: u32 = 0x04;
const SYS_WRITE: u32 = 0x05;
const SYS_READ: u32 = 0x06;
const SYS_EXIT: u32 = 0x18;
/// `ADP_Stopped_ApplicationExit`. newlib `report_exception` passes this in `r1`.
const ADP_STOPPED_APPLICATION_EXIT: u32 = 0x20026;
/// Missing NUL must not walk guest RAM.
const WRITE0_SCAN_CAP: u32 = 4096;
const ERR: u32 = 0xFFFF_FFFF;

/// Serve one retired `bkpt #0xAB`. `r0` is the operation, `r1` the argument.
/// The return value is written to `r0`; every other register is left alone.
/// The caller retires the halfword (`PcAdvance::Keep`).
///
/// Generic over `B: Bus + ?Sized` so `step_execute`'s already-generic bus
/// (including `dyn Bus`) can call it. `#[inline(never)]` keeps the body out
/// of the inlined `exec_bkpt` arm.
#[inline(never)]
pub(in crate::cpu::cortex_m) fn handle<B: Bus + ?Sized>(
    cpu: &mut CortexM,
    bus: &mut B,
) -> SimResult<PcAdvance> {
    bus.semihost_note_attached();
    let op = cpu.r0;
    let arg = cpu.r1;
    match op {
        SYS_WRITEC => cpu.r0 = sys_writec(bus, arg),
        SYS_WRITE0 => cpu.r0 = sys_write0(bus, arg),
        SYS_WRITE => cpu.r0 = sys_write(bus, arg),
        SYS_READ => cpu.r0 = sys_read(bus, arg),
        // Reason code is `r1` itself. Do not load through it: `0x20026` sits
        // in STM32 SRAM, and truncating to 8 bits turns a failed magic check
        // into exit code 0.
        SYS_EXIT => {
            let code = if arg == ADP_STOPPED_APPLICATION_EXIT {
                0
            } else {
                1
            };
            cpu.firmware_exit = Some(code);
        }
        // `SYS_EXIT_EXTENDED` (0x20) and every other operation.
        _ => cpu.r0 = ERR,
    }
    Ok(PcAdvance::Keep)
}

fn sys_writec<B: Bus + ?Sized>(bus: &mut B, ptr: u32) -> u32 {
    match bus.read_u8(u64::from(ptr)) {
        Ok(byte) => {
            bus.semihost_write(&[byte]);
            0
        }
        Err(_) => ERR,
    }
}

fn sys_write0<B: Bus + ?Sized>(bus: &mut B, ptr: u32) -> u32 {
    let mut buf = Vec::new();
    for i in 0..WRITE0_SCAN_CAP {
        match bus.read_u8(u64::from(ptr.wrapping_add(i))) {
            Ok(0) => break,
            Ok(byte) => buf.push(byte),
            Err(_) => return ERR,
        }
    }
    bus.semihost_write(&buf);
    0
}

fn sys_write<B: Bus + ?Sized>(bus: &mut B, block: u32) -> u32 {
    let Some((handle, buf, count)) = read_block(bus, block) else {
        return ERR;
    };
    // stdout and stderr share the semihosting stream. Anything else writes nothing.
    if handle != 1 && handle != 2 {
        return count;
    }
    let Some(bytes) = read_guest(bus, buf, count) else {
        return ERR;
    };
    bus.semihost_write(&bytes);
    0
}

fn sys_read<B: Bus + ?Sized>(bus: &mut B, block: u32) -> u32 {
    let Some((handle, buf, count)) = read_block(bus, block) else {
        return ERR;
    };
    // Only stdin (handle 0) copies. Do not block: an empty host buffer is EOF.
    if handle != 0 || count == 0 {
        return count;
    }
    let mut pending = count;
    let mut addr = buf;
    let mut got_total = 0u32;
    let mut chunk = [0u8; 256];
    while pending > 0 {
        let want = (pending as usize).min(chunk.len());
        let got = bus.semihost_read(&mut chunk[..want]);
        if got == 0 {
            break;
        }
        for &byte in &chunk[..got] {
            if bus.write_u8(u64::from(addr), byte).is_err() {
                return ERR;
            }
            addr = addr.wrapping_add(1);
        }
        got_total += got as u32;
        pending -= got as u32;
        if got < want {
            break;
        }
    }
    count - got_total
}

/// Three little-endian words `{handle, buf, count}`. Unaligned or unmapped is a bad pointer.
fn read_block<B: Bus + ?Sized>(bus: &B, block: u32) -> Option<(u32, u32, u32)> {
    if block & 3 != 0 {
        return None;
    }
    let handle = bus.read_u32(u64::from(block)).ok()?;
    let buf = bus.read_u32(u64::from(block.wrapping_add(4))).ok()?;
    let count = bus.read_u32(u64::from(block.wrapping_add(8))).ok()?;
    Some((handle, buf, count))
}

/// Guest bytes for `SYS_WRITE`. A fault drops the copy so nothing is appended.
fn read_guest<B: Bus + ?Sized>(bus: &B, buf: u32, count: u32) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    for i in 0..count {
        out.push(bus.read_u8(u64::from(buf.wrapping_add(i))).ok()?);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::CortexM;
    use crate::machine::AdvanceStop;
    use crate::{AdvanceRequest, Bus, Cpu, Machine, SimulationConfig, SimulationError};

    const BKPT_AB: u16 = 0xBEAB;
    /// `movs r2, #1` — retires only if `SYS_EXIT` returned to the guest.
    const MOVS_R2_1: u16 = 0x2201;

    fn machine() -> Machine<CortexM> {
        Machine::new(CortexM::new(), crate::bus::SystemBus::new())
    }

    fn plant(m: &mut Machine<CortexM>, op: u32, arg: u32) {
        m.bus.write_u16(0, BKPT_AB).unwrap();
        m.bus.write_u16(2, MOVS_R2_1).unwrap();
        m.cpu.pc = 0;
        m.cpu.r0 = op;
        m.cpu.r1 = arg;
        m.cpu.r2 = 0;
        m.cpu.r4 = 0xA5A5_A5A5;
        m.cpu.lr = 0xFFFF_FFF9;
    }

    fn step(cpu: &mut CortexM, bus: &mut crate::bus::SystemBus) {
        cpu.step(bus, &[], &SimulationConfig::default()).unwrap();
    }

    #[test]
    fn writec_appends_one_byte_and_leaves_other_registers() {
        let mut m = machine();
        m.bus.write_u8(0x2000_0100, b'Z').unwrap();
        plant(&mut m, SYS_WRITEC, 0x2000_0100);
        step(&mut m.cpu, &mut m.bus);
        assert_eq!(m.cpu.r0, 0);
        assert_eq!(m.cpu.r1, 0x2000_0100);
        assert_eq!(m.cpu.r2, 0);
        assert_eq!(m.cpu.r4, 0xA5A5_A5A5);
        assert_eq!(m.cpu.lr, 0xFFFF_FFF9);
        assert_eq!(m.cpu.pc, 2, "the halfword retires; PC is not rewound");
        assert_eq!(m.bus.drain_semihosting_output(), b"Z");
        assert!(m.bus.semihosting_attached());
        assert!(m.bus.drain_semihosting_output().is_empty());
    }

    #[test]
    fn write0_stops_at_nul_and_caps_the_scan() {
        let mut m = machine();
        m.bus.write_u8(0x2000_0200, b'h').unwrap();
        m.bus.write_u8(0x2000_0201, b'i').unwrap();
        m.bus.write_u8(0x2000_0202, 0).unwrap();
        plant(&mut m, SYS_WRITE0, 0x2000_0200);
        step(&mut m.cpu, &mut m.bus);
        assert_eq!(m.cpu.r0, 0);
        assert_eq!(m.bus.semihost_captured(), b"hi");

        let mut m = machine();
        for i in 0..4100u32 {
            m.bus.write_u8(u64::from(0x2000_1000 + i), b'X').unwrap();
        }
        plant(&mut m, SYS_WRITE0, 0x2000_1000);
        step(&mut m.cpu, &mut m.bus);
        assert_eq!(m.cpu.r0, 0);
        assert_eq!(m.bus.semihost_captured().len(), 4096);
    }

    #[test]
    fn write_appends_stdout_and_stderr_and_rejects_other_handles() {
        let mut m = machine();
        m.bus.write_u8(0x2000_0300, b'A').unwrap();
        m.bus.write_u32(0x2000_0400, 1).unwrap();
        m.bus.write_u32(0x2000_0404, 0x2000_0300).unwrap();
        m.bus.write_u32(0x2000_0408, 1).unwrap();
        plant(&mut m, SYS_WRITE, 0x2000_0400);
        step(&mut m.cpu, &mut m.bus);
        assert_eq!(m.cpu.r0, 0);

        m.bus.write_u8(0x2000_0301, b'B').unwrap();
        m.bus.write_u32(0x2000_0400, 2).unwrap();
        m.bus.write_u32(0x2000_0404, 0x2000_0301).unwrap();
        m.cpu.pc = 0;
        m.cpu.r0 = SYS_WRITE;
        m.cpu.r1 = 0x2000_0400;
        step(&mut m.cpu, &mut m.bus);
        assert_eq!(m.cpu.r0, 0);
        assert_eq!(m.bus.semihost_captured(), b"AB");

        m.bus.write_u32(0x2000_0400, 0).unwrap();
        m.bus.write_u32(0x2000_0408, 7).unwrap();
        m.cpu.pc = 0;
        m.cpu.r0 = SYS_WRITE;
        m.cpu.r1 = 0x2000_0400;
        step(&mut m.cpu, &mut m.bus);
        assert_eq!(m.cpu.r0, 7);
        assert_eq!(m.bus.semihost_captured(), b"AB");
    }

    #[test]
    fn read_copies_stdin_without_blocking() {
        let mut m = machine();
        m.bus.write_u8(0x2000_0500, 0xFF).unwrap();
        m.bus.write_u8(0x2000_0501, 0xFF).unwrap();
        m.bus.write_u8(0x2000_0502, 0xFF).unwrap();
        m.bus.write_u8(0x2000_0503, 0xFF).unwrap();
        m.bus.write_u32(0x2000_0600, 0).unwrap();
        m.bus.write_u32(0x2000_0604, 0x2000_0500).unwrap();
        m.bus.write_u32(0x2000_0608, 4).unwrap();
        m.bus.write_semihosting_input(b"ab");
        plant(&mut m, SYS_READ, 0x2000_0600);
        step(&mut m.cpu, &mut m.bus);
        assert_eq!(m.cpu.r0, 2, "two bytes were not available");
        assert_eq!(m.bus.read_u8(0x2000_0500).unwrap(), b'a');
        assert_eq!(m.bus.read_u8(0x2000_0501).unwrap(), b'b');
        assert_eq!(m.bus.read_u8(0x2000_0502).unwrap(), 0xFF);

        m.cpu.pc = 0;
        m.cpu.r0 = SYS_READ;
        m.cpu.r1 = 0x2000_0600;
        step(&mut m.cpu, &mut m.bus);
        assert_eq!(m.cpu.r0, 4, "empty host buffer is EOF, not a stall");

        m.bus.write_u32(0x2000_0600, 5).unwrap();
        m.bus.write_semihosting_input(b"zzzz");
        m.cpu.pc = 0;
        m.cpu.r0 = SYS_READ;
        m.cpu.r1 = 0x2000_0600;
        step(&mut m.cpu, &mut m.bus);
        assert_eq!(m.cpu.r0, 4);
        assert_eq!(m.bus.read_u8(0x2000_0500).unwrap(), b'a');
    }

    #[test]
    fn bad_pointer_returns_minus_one_and_does_not_halt() {
        let mut m = machine();
        plant(&mut m, SYS_WRITEC, 0x1000_0000);
        step(&mut m.cpu, &mut m.bus);
        assert_eq!(m.cpu.r0, ERR);
        assert!(m.bus.semihost_captured().is_empty());
        assert!(m.bus.semihosting_attached());

        m.cpu.pc = 0;
        m.cpu.r0 = SYS_WRITE;
        m.cpu.r1 = 0x2000_0001;
        step(&mut m.cpu, &mut m.bus);
        assert_eq!(m.cpu.r0, ERR);
        assert_eq!(m.cpu.pc, 2);
    }

    #[test]
    fn other_bkpt_immediate_still_halts_and_does_not_attach() {
        let mut m = machine();
        m.bus.write_u16(0, 0xBE00).unwrap();
        m.cpu.pc = 0;
        m.cpu.r0 = 0x1111_1111;
        let err = m.cpu.step(&mut m.bus, &[], &SimulationConfig::default());
        assert!(matches!(err, Err(SimulationError::Halt)));
        assert!(!m.bus.semihosting_attached());
        assert_eq!(m.cpu.r0, 0x1111_1111);
    }

    #[test]
    fn application_exit_stops_with_code_zero_and_does_not_return() {
        let mut m = machine();
        plant(&mut m, SYS_EXIT, ADP_STOPPED_APPLICATION_EXIT);
        let report = m.advance(AdvanceRequest::run(Some(8))).unwrap();
        assert_eq!(report.stop, AdvanceStop::FirmwareExit { code: 0 });
        assert_eq!(report.primary_steps, 1);
        assert_eq!(
            m.cpu.r2, 0,
            "the instruction after SYS_EXIT must not retire"
        );
        assert_eq!(m.cpu.pc, 2);
        assert!(
            m.cpu.take_firmware_exit().is_none(),
            "the code is taken once"
        );
        assert!(m.bus.semihosting_attached());
    }

    #[test]
    fn runtime_error_and_any_other_immediate_exit_one() {
        for reason in [0x20023u32, 0x26, 0, 0xFFFF_FFFF, 0x20027] {
            let mut m = machine();
            plant(&mut m, SYS_EXIT, reason);
            let report = m.advance(AdvanceRequest::run(Some(8))).unwrap();
            assert_eq!(
                report.stop,
                AdvanceStop::FirmwareExit { code: 1 },
                "r1={reason:#x} must exit 1, not a truncated code"
            );
            assert_eq!(m.cpu.r2, 0);
        }
    }

    #[test]
    fn exit_extended_does_not_exit() {
        let mut m = machine();
        plant(&mut m, 0x20, ADP_STOPPED_APPLICATION_EXIT);
        let report = m.advance(AdvanceRequest::run(Some(2))).unwrap();
        assert_eq!(report.stop, AdvanceStop::FuelLimit);
        assert_eq!(m.cpu.r0, ERR);
        assert_eq!(m.cpu.r2, 1, "0x20 retires and the next instruction runs");
        assert!(m.cpu.take_firmware_exit().is_none());
        assert!(m.bus.semihosting_attached());
    }

    #[test]
    fn unknown_operation_returns_minus_one() {
        let mut m = machine();
        plant(&mut m, 0x01, 0);
        step(&mut m.cpu, &mut m.bus);
        assert_eq!(m.cpu.r0, ERR);
        assert!(m.cpu.firmware_exit.is_none());
        assert!(m.bus.semihosting_attached());
    }

    #[test]
    fn input_queue_caps_a_single_write() {
        let mut m = machine();
        let big = vec![b'Q'; 70 * 1024];
        m.bus.write_semihosting_input(&big);
        m.bus.write_semihosting_input(&big);
        let mut dst = vec![0u8; 80 * 1024];
        let n = m.bus.semihost_read(&mut dst);
        assert_eq!(n, 64 * 1024);
    }
}
