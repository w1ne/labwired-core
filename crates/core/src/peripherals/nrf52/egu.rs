// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 EGU (Event Generation Unit).
//!
//! Source: nRF52840 PS rev 1.7 §6.8 (EGU). 6 instances (EGU0..EGU5).
//! Each instance has 16 software-triggered channels: TASKS_TRIGGER[i]
//! fires EVENTS_TRIGGERED[i]. Heavily used as PPI source by drivers
//! that need to chain software events to hardware actions.
//!
//! tick() drains pending triggers, emitting fired_events so PPI routes
//! them and pending the configured NVIC IRQ when INTEN is set.

use crate::{Peripheral, PeripheralTickResult, SimResult};

const OFF_TASKS_TRIGGER_0: u64 = 0x000;
const OFF_TASKS_TRIGGER_15: u64 = 0x03C;
const OFF_EVENTS_TRIGGERED_0: u64 = 0x100;
const OFF_EVENTS_TRIGGERED_15: u64 = 0x13C;
const OFF_INTEN: u64 = 0x300;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;

#[derive(Debug, Default)]
pub struct Nrf52Egu {
    events_triggered: [u32; 16],
    inten: u32,
    /// Bitmask of channels whose TASKS_TRIGGER fired since the last
    /// tick(). Drained into fired_events + IRQ pend on tick.
    pending_triggers: u32,
}

impl Nrf52Egu {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Peripheral for Nrf52Egu {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            OFF_TASKS_TRIGGER_0..=OFF_TASKS_TRIGGER_15 if offset.is_multiple_of(4) => 0,
            OFF_EVENTS_TRIGGERED_0..=OFF_EVENTS_TRIGGERED_15 if offset.is_multiple_of(4) => {
                self.events_triggered[((offset - OFF_EVENTS_TRIGGERED_0) / 4) as usize]
            }
            OFF_INTEN | OFF_INTENSET | OFF_INTENCLR => self.inten,
            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_TASKS_TRIGGER_0..=OFF_TASKS_TRIGGER_15
                if offset.is_multiple_of(4) && value & 1 != 0 =>
            {
                let i = ((offset - OFF_TASKS_TRIGGER_0) / 4) as usize;
                self.events_triggered[i] = 1;
                self.pending_triggers |= 1 << i;
            }
            OFF_EVENTS_TRIGGERED_0..=OFF_EVENTS_TRIGGERED_15 if offset.is_multiple_of(4) => {
                let i = ((offset - OFF_EVENTS_TRIGGERED_0) / 4) as usize;
                self.events_triggered[i] = value & 1;
            }
            OFF_INTEN => self.inten = value & 0xFFFF,
            OFF_INTENSET => self.inten |= value & 0xFFFF,
            OFF_INTENCLR => self.inten &= !value,
            _ => {}
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        if self.pending_triggers == 0 {
            return PeripheralTickResult::default();
        }
        let pending = self.pending_triggers;
        self.pending_triggers = 0;

        let mut fired = Vec::with_capacity(pending.count_ones() as usize);
        for i in 0..16u32 {
            if pending & (1 << i) != 0 {
                fired.push(OFF_EVENTS_TRIGGERED_0 as u32 + 4 * i);
            }
        }
        let irq = pending & self.inten != 0;
        PeripheralTickResult {
            irq,
            cycles: 1,
            fired_events: fired,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_sets_event() {
        let mut e = Nrf52Egu::new();
        e.write_u32(OFF_TASKS_TRIGGER_0, 1).unwrap();
        assert_eq!(e.read_u32(OFF_EVENTS_TRIGGERED_0).unwrap(), 1);
        let res = e.tick();
        assert_eq!(res.fired_events, vec![OFF_EVENTS_TRIGGERED_0 as u32]);
    }

    #[test]
    fn intenset_pends_irq_on_trigger() {
        let mut e = Nrf52Egu::new();
        e.write_u32(OFF_INTENSET, 1).unwrap();
        e.write_u32(OFF_TASKS_TRIGGER_0, 1).unwrap();
        assert!(e.tick().irq);
    }
}
