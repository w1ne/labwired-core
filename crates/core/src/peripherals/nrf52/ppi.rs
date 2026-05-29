// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 PPI peripheral — register-surface model.
//!
//! Source: nRF52840 PS rev 1.7 §6.19 (PPI). 32 channels, plus 6 channel
//! groups. CHENSET/CHENCLR are write-through aliases that modify CHEN.
//! Each channel has an EEP (event endpoint) and TEP (task endpoint)
//! register; we keep them so firmware can configure routing even though
//! the simulator doesn't yet propagate events through PPI.

use crate::{Peripheral, SimResult};

// ── PPI routing ───────────────────────────────────────────────────────────────
//
// The bus collects EVENTS_* register transitions (0→1) into a global list of
// absolute addresses each tick.  Nrf52Ppi observes that list via the
// `route_ppi_events` trait method (default-empty on every other peripheral)
// and returns the list of TASKS_* addresses to trigger.  Bus then writes 1
// to each task address, which (for GPIOTE) drives a GPIO pin via the
// mmio_writes channel in the next phase.
//
// The trait hook is defined on `Peripheral` so the bus can iterate without
// downcasting.  Only PPI overrides it.

const OFF_CHG_EN0: u64 = 0x000;
const OFF_CHG_DIS_LAST: u64 = 0x02C; // CHG[5].DIS

const OFF_CHEN: u64 = 0x500;
const OFF_CHENSET: u64 = 0x504;
const OFF_CHENCLR: u64 = 0x508;
const OFF_CH0_EEP: u64 = 0x510;
const OFF_CH31_TEP: u64 = 0x60C; // 0x510 + 32*8 - 4
const OFF_CHG0: u64 = 0x800;
const OFF_CHG5: u64 = 0x814;
const OFF_FORK0_TEP: u64 = 0x910;
const OFF_FORK31_TEP: u64 = 0x98C;

#[derive(Debug, Default)]
pub struct Nrf52Ppi {
    chen: u32,
    ch_eep: [u32; 32],
    ch_tep: [u32; 32],
    chg: [u32; 6],
    fork_tep: [u32; 32],
}

impl Nrf52Ppi {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Peripheral for Nrf52Ppi {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            OFF_CHG_EN0..=OFF_CHG_DIS_LAST if offset.is_multiple_of(4) => 0, // TASKS_*

            OFF_CHEN => self.chen,
            OFF_CHENSET | OFF_CHENCLR => self.chen,

            OFF_CH0_EEP..=OFF_CH31_TEP if offset.is_multiple_of(4) => {
                let rel = offset - OFF_CH0_EEP;
                let ch = (rel / 8) as usize;
                if ch >= 32 {
                    0
                } else if rel.is_multiple_of(8) {
                    self.ch_eep[ch]
                } else {
                    self.ch_tep[ch]
                }
            }

            OFF_CHG0..=OFF_CHG5 if offset.is_multiple_of(4) => {
                self.chg[((offset - OFF_CHG0) / 4) as usize]
            }

            OFF_FORK0_TEP..=OFF_FORK31_TEP if offset.is_multiple_of(4) => {
                self.fork_tep[((offset - OFF_FORK0_TEP) / 4) as usize]
            }

            _ => 0,
        })
    }

    fn route_ppi_events(&mut self, fired_global: &[u32]) -> Vec<u32> {
        if self.chen == 0 || fired_global.is_empty() {
            return Vec::new();
        }
        let mut tasks = Vec::with_capacity(fired_global.len());
        for &event_addr in fired_global {
            for ch in 0..32 {
                if self.chen & (1u32 << ch) == 0 {
                    continue;
                }
                if self.ch_eep[ch] == event_addr {
                    if self.ch_tep[ch] != 0 {
                        tasks.push(self.ch_tep[ch]);
                    }
                    if self.fork_tep[ch] != 0 {
                        tasks.push(self.fork_tep[ch]);
                    }
                }
            }
        }
        tasks
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_CHG_EN0..=OFF_CHG_DIS_LAST if offset.is_multiple_of(4) => {
                // CHG[i].EN / CHG[i].DIS — set/clear the channels in CHEN
                // belonging to channel-group CHG[i].
                let rel = offset - OFF_CHG_EN0;
                let group = (rel / 8) as usize;
                if group < 6 {
                    let mask = self.chg[group];
                    if rel.is_multiple_of(8) {
                        // EN
                        self.chen |= mask;
                    } else {
                        // DIS
                        self.chen &= !mask;
                    }
                }
            }
            OFF_CHEN => self.chen = value,
            OFF_CHENSET => self.chen |= value,
            OFF_CHENCLR => self.chen &= !value,
            OFF_CH0_EEP..=OFF_CH31_TEP if offset.is_multiple_of(4) => {
                let rel = offset - OFF_CH0_EEP;
                let ch = (rel / 8) as usize;
                if ch < 32 {
                    if rel.is_multiple_of(8) {
                        self.ch_eep[ch] = value;
                    } else {
                        self.ch_tep[ch] = value;
                    }
                }
            }
            OFF_CHG0..=OFF_CHG5 if offset.is_multiple_of(4) => {
                self.chg[((offset - OFF_CHG0) / 4) as usize] = value;
            }
            OFF_FORK0_TEP..=OFF_FORK31_TEP if offset.is_multiple_of(4) => {
                self.fork_tep[((offset - OFF_FORK0_TEP) / 4) as usize] = value;
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chenset_chenclr_alias_chen() {
        let mut p = Nrf52Ppi::new();
        p.write_u32(OFF_CHENSET, 0b1010).unwrap();
        assert_eq!(p.read_u32(OFF_CHEN).unwrap(), 0b1010);
        assert_eq!(p.read_u32(OFF_CHENSET).unwrap(), 0b1010);
        assert_eq!(p.read_u32(OFF_CHENCLR).unwrap(), 0b1010);
        p.write_u32(OFF_CHENCLR, 0b0010).unwrap();
        assert_eq!(p.read_u32(OFF_CHEN).unwrap(), 0b1000);
    }

    #[test]
    fn route_ppi_events_returns_tep_for_enabled_channels() {
        let mut p = Nrf52Ppi::new();
        // CH[0] routes TIMER0 EVENTS_COMPARE[0] -> GPIOTE TASKS_OUT[0].
        p.write_u32(OFF_CH0_EEP + 0, 0x4000_8140).unwrap(); // TIMER0+0x140
        p.write_u32(OFF_CH0_EEP + 4, 0x4000_6000).unwrap(); // GPIOTE+0x000
        p.write_u32(OFF_CHENSET, 1).unwrap();

        let tasks = p.route_ppi_events(&[0x4000_8140]);
        assert_eq!(tasks, vec![0x4000_6000]);
    }

    #[test]
    fn route_ppi_events_ignores_disabled_channels() {
        let mut p = Nrf52Ppi::new();
        p.write_u32(OFF_CH0_EEP + 0, 0x4000_8140).unwrap();
        p.write_u32(OFF_CH0_EEP + 4, 0x4000_6000).unwrap();
        // CHEN bit 0 not set.
        let tasks = p.route_ppi_events(&[0x4000_8140]);
        assert!(tasks.is_empty());
    }

    #[test]
    fn route_ppi_events_emits_fork_tep() {
        let mut p = Nrf52Ppi::new();
        p.write_u32(OFF_CH0_EEP + 0, 0x4000_8140).unwrap();
        p.write_u32(OFF_CH0_EEP + 4, 0x4000_6000).unwrap();
        p.write_u32(OFF_FORK0_TEP, 0x4000_6004).unwrap();
        p.write_u32(OFF_CHENSET, 1).unwrap();
        let tasks = p.route_ppi_events(&[0x4000_8140]);
        assert_eq!(tasks, vec![0x4000_6000, 0x4000_6004]);
    }

    #[test]
    fn channel_eep_tep_round_trip() {
        let mut p = Nrf52Ppi::new();
        p.write_u32(OFF_CH0_EEP + 0, 0x4000_8140).unwrap();
        p.write_u32(OFF_CH0_EEP + 4, 0x4001_F500).unwrap();
        assert_eq!(p.read_u32(OFF_CH0_EEP + 0).unwrap(), 0x4000_8140);
        assert_eq!(p.read_u32(OFF_CH0_EEP + 4).unwrap(), 0x4001_F500);
    }
}
