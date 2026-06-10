// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 QDEC (Quadrature Decoder).
//!
//! Source: nRF52840 PS rev 1.7 §6.20 (QDEC). Register-surface model;
//! ACC/ACCREAD/SAMPLE registers track sampled deltas. No physical
//! pin observation — firmware reading ACC will see 0 unless test
//! code writes it directly.

use crate::{Peripheral, SimResult};

const OFF_TASKS_START: u64 = 0x000;
const OFF_TASKS_STOP: u64 = 0x004;
const OFF_TASKS_READCLRACC: u64 = 0x008;
const OFF_TASKS_RDCLRACC: u64 = 0x00C;
const OFF_TASKS_RDCLRDBL: u64 = 0x010;
const OFF_EVENTS_SAMPLERDY: u64 = 0x100;
const OFF_EVENTS_REPORTRDY: u64 = 0x104;
const OFF_EVENTS_ACCOF: u64 = 0x108;
const OFF_EVENTS_DBLRDY: u64 = 0x10C;
const OFF_EVENTS_STOPPED: u64 = 0x110;
const OFF_SHORTS: u64 = 0x200;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;
const OFF_ENABLE: u64 = 0x500;
const OFF_LEDPOL: u64 = 0x504;
const OFF_SAMPLEPER: u64 = 0x508;
const OFF_SAMPLE: u64 = 0x50C;
const OFF_REPORTPER: u64 = 0x510;
const OFF_ACC: u64 = 0x514;
const OFF_ACCREAD: u64 = 0x518;
const OFF_PSEL_LED: u64 = 0x51C;
const OFF_PSEL_A: u64 = 0x520;
const OFF_PSEL_B: u64 = 0x524;
const OFF_DBFEN: u64 = 0x528;
const OFF_LEDPRE: u64 = 0x540;
const OFF_ACCDBL: u64 = 0x544;
const OFF_ACCDBLREAD: u64 = 0x548;

#[derive(Debug, Default)]
pub struct Nrf52Qdec {
    events_samplerdy: u32,
    events_reportrdy: u32,
    events_accof: u32,
    events_dblrdy: u32,
    events_stopped: u32,
    shorts: u32,
    inten: u32,
    enable: u32,
    ledpol: u32,
    sampleper: u32,
    sample: u32,
    reportper: u32,
    acc: u32,
    accread: u32,
    psel_led: u32,
    psel_a: u32,
    psel_b: u32,
    dbfen: u32,
    ledpre: u32,
    accdbl: u32,
    accdblread: u32,
}

impl Nrf52Qdec {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Peripheral for Nrf52Qdec {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            OFF_TASKS_START | OFF_TASKS_STOP | OFF_TASKS_READCLRACC | OFF_TASKS_RDCLRACC
            | OFF_TASKS_RDCLRDBL => 0,
            OFF_EVENTS_SAMPLERDY => self.events_samplerdy,
            OFF_EVENTS_REPORTRDY => self.events_reportrdy,
            OFF_EVENTS_ACCOF => self.events_accof,
            OFF_EVENTS_DBLRDY => self.events_dblrdy,
            OFF_EVENTS_STOPPED => self.events_stopped,
            OFF_SHORTS => self.shorts,
            OFF_INTENSET | OFF_INTENCLR => self.inten,
            OFF_ENABLE => self.enable & 1,
            OFF_LEDPOL => self.ledpol & 1,
            OFF_SAMPLEPER => self.sampleper & 0xF,
            OFF_SAMPLE => self.sample,
            OFF_REPORTPER => self.reportper & 0xF,
            OFF_ACC => self.acc,
            OFF_ACCREAD => self.accread,
            OFF_PSEL_LED => self.psel_led,
            OFF_PSEL_A => self.psel_a,
            OFF_PSEL_B => self.psel_b,
            OFF_DBFEN => self.dbfen & 1,
            OFF_LEDPRE => self.ledpre & 0x1FF,
            OFF_ACCDBL => self.accdbl,
            OFF_ACCDBLREAD => self.accdblread,
            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_TASKS_START | OFF_TASKS_STOP | OFF_TASKS_READCLRACC | OFF_TASKS_RDCLRACC
            | OFF_TASKS_RDCLRDBL => {}
            // EVENTS_*: hardware-generated. SW write-1 ignored; write-0 clears.
            OFF_EVENTS_SAMPLERDY if value == 0 => self.events_samplerdy = 0,
            OFF_EVENTS_REPORTRDY if value == 0 => self.events_reportrdy = 0,
            OFF_EVENTS_ACCOF if value == 0 => self.events_accof = 0,
            OFF_EVENTS_DBLRDY if value == 0 => self.events_dblrdy = 0,
            OFF_EVENTS_STOPPED if value == 0 => self.events_stopped = 0,
            OFF_SHORTS => self.shorts = value & 0x3,
            OFF_INTENSET => self.inten |= value & 0x1F,
            OFF_INTENCLR => self.inten &= !value,
            OFF_ENABLE => self.enable = value & 1,
            OFF_LEDPOL => self.ledpol = value & 1,
            OFF_SAMPLEPER => self.sampleper = value & 0xF,
            OFF_SAMPLE => {} // RO
            OFF_REPORTPER => self.reportper = value & 0xF,
            OFF_ACC => self.acc = value,
            OFF_ACCREAD => {} // RO
            OFF_PSEL_LED => self.psel_led = value,
            OFF_PSEL_A => self.psel_a = value,
            OFF_PSEL_B => self.psel_b = value,
            OFF_DBFEN => self.dbfen = value & 1,
            OFF_LEDPRE => self.ledpre = value & 0x1FF,
            OFF_ACCDBL => self.accdbl = value,
            OFF_ACCDBLREAD => {} // RO
            _ => {}
        }
        Ok(())
    }
}
