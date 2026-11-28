// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF54L15 RRAMC (resistive RAM controller).
//!
//! Sources: Nordic `nrf54l15.svd` peripheral `GLOBAL_RRAMC_S` (register
//! offsets, reset values, field enums); Nordic product specification, RRAMC
//! chapter, sections "Writing to RRAM", "Buffered RRAM write" and "Erasing
//! RRAM"; nrfx `nrf_rramc.h` and Zephyr `soc_flash_nrf_rram.c` for the access
//! sequence real firmware uses.
//!
//! Fidelity contract:
//!   * CONFIG.WEN gates CPU stores into the RRAM array. With WEN set, a store
//!     of any natural width commits as written: RRAM writes both 0 and 1 to
//!     any bit, so there is no erase-before-write and no 1→0 rule. With WEN
//!     clear ("Write is disabled") the store is dropped. Whether silicon also
//!     raises a bus error or ACCESSERROR for that store is not established by
//!     the sources above, so neither is raised here.
//!   * READY and READYNEXT read 1: write latency is not modelled, so the
//!     controller is never observed busy.
//!   * The write-buffer is modelled as STATE only. Data lands in the array on
//!     the store (silicon commits on a read of a buffered line, so firmware
//!     cannot read stale data either way). BUFSTATUS.WRITEBUFEMPTY reads
//!     NotEmpty from the first buffered store until the buffer commits: the
//!     last byte of the buffer area is written, a store lands outside the
//!     buffer area, or TASKS_COMMITWRITEBUF is triggered. The buffer area is
//!     taken as CONFIG.WRITEBUFSIZE 128-bit lines starting at the first
//!     buffered line. The commit on a CPU read and the READYNEXTTIMEOUT
//!     commit are not modelled, so WRITEBUFEMPTY can read NotEmpty where
//!     silicon already committed.
//!     Setting WRITEBUFSIZE to Unbuffered empties the modelled buffer; the
//!     sources do not state what silicon does with pending data there.
//!   * No snapshot/restore of the register state.
//!   * EVENTS_READY latches when a commit or a wake-up completes (with
//!     WRITEBUFSIZE Unbuffered, every store is a commit);
//!     EVENTS_WOKENUP latches on TASKS_WAKEUP. INTEN/INTENSET/INTENCLR/INTPEND
//!     are register-accurate, but no NVIC line is driven.
//!   * ERASE.ERASEALL is not modelled. A write of Erase fails the access
//!     instead of reporting an erase that did not happen.
//!   * REGION[n] protection, POWER modes, ECC errors, DPPI subscribe/publish
//!     and UICR/SICR programming are stored or reset-valued registers only.

use crate::{FlashStoreOutcome, Peripheral, SimResult, SimulationError};

const OFF_TASKS_WAKEUP: u64 = 0x000;
const OFF_TASKS_COMMITWRITEBUF: u64 = 0x008;
const OFF_SUBSCRIBE_WAKEUP: u64 = 0x080;
const OFF_SUBSCRIBE_COMMITWRITEBUF: u64 = 0x088;
const OFF_EVENTS_WOKENUP: u64 = 0x100;
const OFF_EVENTS_READY: u64 = 0x104;
const OFF_EVENTS_READYNEXT: u64 = 0x108;
const OFF_EVENTS_ACCESSERROR: u64 = 0x10C;
const OFF_PUBLISH_WOKENUP: u64 = 0x180;
const OFF_INTEN: u64 = 0x300;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;
const OFF_INTPEND: u64 = 0x30C;
const OFF_READY: u64 = 0x400;
const OFF_READYNEXT: u64 = 0x404;
const OFF_ACCESSERRORADDR: u64 = 0x408;
const OFF_WRITEBUFEMPTY: u64 = 0x418;
const OFF_ECC_ERRORADDR: u64 = 0x420;
const OFF_CONFIG: u64 = 0x500;
const OFF_READYNEXTTIMEOUT: u64 = 0x50C;
const OFF_POWER_CONFIG: u64 = 0x510;
const OFF_POWER_LOWPOWERCONFIG: u64 = 0x518;
const OFF_ERASEALL: u64 = 0x540;
const OFF_REGION0: u64 = 0x550;
const REGIONS: usize = 5;

const CONFIG_WEN: u32 = 1;
const CONFIG_WRITEBUFSIZE_POS: u32 = 8;
const CONFIG_WRITEBUFSIZE_MASK: u32 = 0x3F << CONFIG_WRITEBUFSIZE_POS;
const CONFIG_MASK: u32 = CONFIG_WEN | CONFIG_WRITEBUFSIZE_MASK;
/// WOKENUP, READY, READYNEXT, ACCESSERROR (SVD field order).
const INTEN_MASK: u32 = 0xF;
const DPPI_MASK: u32 = 0x8000_00FF;
const READYNEXTTIMEOUT_MASK: u32 = 0x8000_0FFF;
const POWER_CONFIG_MASK: u32 = 0x0001_FFFF;
/// REGION[n].CONFIG: READ/WRITE/EXECUTE/SECURE, OWNER, WRITEONCE, LOCK, SIZE.
const REGION_CONFIG_MASK: u32 = 0x001F_30FF;
/// Bytes in one 128-bit write-buffer line.
const LINE: u64 = 16;

#[derive(Debug)]
pub struct Nrf54lRramc {
    events: [u32; 4],
    inten: u32,
    subscribe_wakeup: u32,
    subscribe_commitwritebuf: u32,
    publish_wokenup: u32,
    config: u32,
    readynexttimeout: u32,
    power_config: u32,
    lowpowerconfig: u32,
    region_address: [u32; REGIONS],
    region_config: [u32; REGIONS],
    /// First 128-bit line (array offset / 16) of the uncommitted write-buffer.
    buffer_start: Option<u64>,
}

impl Default for Nrf54lRramc {
    fn default() -> Self {
        Self {
            events: [0; 4],
            inten: 0,
            subscribe_wakeup: 0,
            subscribe_commitwritebuf: 0,
            publish_wokenup: 0,
            config: 0,
            readynexttimeout: 0x80,
            power_config: 0x100,
            lowpowerconfig: 0,
            region_address: [0; REGIONS],
            region_config: [0; REGIONS],
            buffer_start: None,
        }
    }
}

impl Nrf54lRramc {
    pub fn new() -> Self {
        Self::default()
    }

    fn write_buffer_lines(&self) -> u64 {
        u64::from((self.config & CONFIG_WRITEBUFSIZE_MASK) >> CONFIG_WRITEBUFSIZE_POS)
    }

    /// The buffer went from holding data to committed: READY changes to Ready.
    fn commit_buffer(&mut self) {
        if self.buffer_start.take().is_some() {
            self.events[1] = 1;
        }
    }

    fn intpend(&self) -> u32 {
        let pending = self
            .events
            .iter()
            .enumerate()
            .fold(0, |acc, (bit, ev)| acc | ((ev & 1) << bit));
        pending & self.inten
    }

    fn region_index(offset: u64) -> Option<(usize, bool)> {
        let rel = offset.checked_sub(OFF_REGION0)?;
        let index = (rel / 8) as usize;
        (index < REGIONS && rel % 4 == 0).then_some((index, rel % 8 == 4))
    }
}

impl Peripheral for Nrf54lRramc {
    /// Every effect lands on an MMIO write or a flash store; nothing here is
    /// time-driven.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_u32(offset & !3)?;
        Ok((word >> (8 * (offset & 3))) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let shift = 8 * (offset & 3);
        let word = self.read_u32(offset & !3)?;
        let merged = (word & !(0xFF << shift)) | (u32::from(value) << shift);
        self.write_u32(offset & !3, merged)
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            OFF_TASKS_WAKEUP | OFF_TASKS_COMMITWRITEBUF => 0,
            OFF_SUBSCRIBE_WAKEUP => self.subscribe_wakeup,
            OFF_SUBSCRIBE_COMMITWRITEBUF => self.subscribe_commitwritebuf,
            OFF_EVENTS_WOKENUP => self.events[0],
            OFF_EVENTS_READY => self.events[1],
            OFF_EVENTS_READYNEXT => self.events[2],
            OFF_EVENTS_ACCESSERROR => self.events[3],
            OFF_PUBLISH_WOKENUP => self.publish_wokenup,
            OFF_INTEN | OFF_INTENSET | OFF_INTENCLR => self.inten,
            OFF_INTPEND => self.intpend(),
            // No write latency: the controller is never observed busy.
            OFF_READY | OFF_READYNEXT => 1,
            // SVD reset value; no access or ECC error is ever recorded.
            OFF_ACCESSERRORADDR | OFF_ECC_ERRORADDR => 0x00FF_FFFF,
            OFF_WRITEBUFEMPTY => u32::from(self.buffer_start.is_none()),
            OFF_CONFIG => self.config,
            OFF_READYNEXTTIMEOUT => self.readynexttimeout,
            OFF_POWER_CONFIG => self.power_config,
            OFF_POWER_LOWPOWERCONFIG => self.lowpowerconfig,
            OFF_ERASEALL => 0,
            _ => match Self::region_index(offset) {
                Some((n, false)) => self.region_address[n],
                Some((n, true)) => self.region_config[n],
                None => {
                    crate::census_reg!("nrf54l.rramc:Nrf54lRramc", offset, "read");
                    0
                }
            },
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_TASKS_WAKEUP if value & 1 != 0 => {
                // Wake-up latency is not modelled: the RRAM is awake at once.
                self.events[0] = 1;
                self.events[1] = 1;
            }
            OFF_TASKS_COMMITWRITEBUF if value & 1 != 0 => self.commit_buffer(),
            OFF_TASKS_WAKEUP | OFF_TASKS_COMMITWRITEBUF => {}
            OFF_SUBSCRIBE_WAKEUP => self.subscribe_wakeup = value & DPPI_MASK,
            OFF_SUBSCRIBE_COMMITWRITEBUF => self.subscribe_commitwritebuf = value & DPPI_MASK,
            OFF_EVENTS_WOKENUP => self.events[0] = value & 1,
            OFF_EVENTS_READY => self.events[1] = value & 1,
            OFF_EVENTS_READYNEXT => self.events[2] = value & 1,
            OFF_EVENTS_ACCESSERROR => self.events[3] = value & 1,
            OFF_PUBLISH_WOKENUP => self.publish_wokenup = value & DPPI_MASK,
            OFF_INTEN => self.inten = value & INTEN_MASK,
            OFF_INTENSET => self.inten |= value & INTEN_MASK,
            OFF_INTENCLR => self.inten &= !value,
            OFF_INTPEND | OFF_READY | OFF_READYNEXT | OFF_ACCESSERRORADDR | OFF_WRITEBUFEMPTY
            | OFF_ECC_ERRORADDR => {}
            OFF_CONFIG => {
                self.config = value & CONFIG_MASK;
                if self.write_buffer_lines() == 0 {
                    // Unbuffered: nothing can stay uncommitted in the model.
                    self.commit_buffer();
                }
            }
            OFF_READYNEXTTIMEOUT => self.readynexttimeout = value & READYNEXTTIMEOUT_MASK,
            OFF_POWER_CONFIG => self.power_config = value & POWER_CONFIG_MASK,
            OFF_POWER_LOWPOWERCONFIG => self.lowpowerconfig = value & 0x3,
            OFF_ERASEALL if value & 1 != 0 => {
                return Err(SimulationError::Other(
                    "nRF54L RRAMC: ERASE.ERASEALL is not modelled".to_string(),
                ));
            }
            OFF_ERASEALL => {}
            _ => match Self::region_index(offset) {
                Some((n, false)) => self.region_address[n] = value,
                Some((n, true)) => self.region_config[n] = value & REGION_CONFIG_MASK,
                None => {
                    crate::census_reg!("nrf54l.rramc:Nrf54lRramc", offset, "write");
                }
            },
        }
        Ok(())
    }

    fn on_system_reset(&mut self) {
        *self = Self::default();
    }

    fn owns_flash_stores(&self) -> bool {
        true
    }

    fn flash_cpu_store(
        &mut self,
        offset: u64,
        width: u8,
        _value: u32,
        _existing: u32,
    ) -> FlashStoreOutcome {
        if self.config & CONFIG_WEN == 0 {
            return FlashStoreOutcome::Consumed;
        }
        let lines = self.write_buffer_lines();
        if lines == 0 {
            // Unbuffered: the store is its own commit.
            self.events[1] = 1;
        } else {
            let first = offset / LINE;
            let last_byte = offset + u64::from(width) - 1;
            let start = match self.buffer_start {
                Some(start) if (start..start + lines).contains(&first) => start,
                // A store outside the buffer area commits the old buffer.
                Some(_) => {
                    self.commit_buffer();
                    first
                }
                None => first,
            };
            self.buffer_start = Some(start);
            if last_byte >= (start + lines) * LINE - 1 {
                // The write-buffer is full.
                self.commit_buffer();
            }
        }
        FlashStoreOutcome::Commit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(rramc: &mut Nrf54lRramc, offset: u64, width: u8) -> FlashStoreOutcome {
        rramc.flash_cpu_store(offset, width, 0, 0)
    }

    #[test]
    fn idle_controller_reads_ready_and_an_empty_write_buffer() {
        let rramc = Nrf54lRramc::new();
        assert_eq!(rramc.read_u32(OFF_READY).unwrap(), 1);
        assert_eq!(rramc.read_u32(OFF_READYNEXT).unwrap(), 1);
        assert_eq!(rramc.read_u32(OFF_WRITEBUFEMPTY).unwrap(), 1);
        assert_eq!(rramc.read_u32(OFF_CONFIG).unwrap(), 0);
        assert_eq!(rramc.read_u32(OFF_READYNEXTTIMEOUT).unwrap(), 0x80);
        assert_eq!(rramc.read_u32(OFF_POWER_CONFIG).unwrap(), 0x100);
        assert_eq!(rramc.read_u32(OFF_ACCESSERRORADDR).unwrap(), 0x00FF_FFFF);
        assert_eq!(rramc.read_u32(OFF_ECC_ERRORADDR).unwrap(), 0x00FF_FFFF);
    }

    #[test]
    fn stores_are_dropped_until_wen_is_set() {
        let mut rramc = Nrf54lRramc::new();
        assert_eq!(store(&mut rramc, 0x1000, 4), FlashStoreOutcome::Consumed);
        rramc.write_u32(OFF_CONFIG, CONFIG_WEN).unwrap();
        assert_eq!(store(&mut rramc, 0x1000, 4), FlashStoreOutcome::Commit);
        // Unbuffered: every store commits at once, the buffer stays empty.
        assert_eq!(rramc.read_u32(OFF_WRITEBUFEMPTY).unwrap(), 1);
        rramc.write_u32(OFF_CONFIG, 0).unwrap();
        assert_eq!(store(&mut rramc, 0x1000, 1), FlashStoreOutcome::Consumed);
    }

    #[test]
    fn buffered_store_holds_the_buffer_until_a_commit_condition() {
        let mut rramc = Nrf54lRramc::new();
        // WEN, one-line buffer.
        rramc.write_u32(OFF_CONFIG, CONFIG_WEN | (1 << 8)).unwrap();
        assert_eq!(store(&mut rramc, 0x2000, 4), FlashStoreOutcome::Commit);
        assert_eq!(rramc.read_u32(OFF_WRITEBUFEMPTY).unwrap(), 0);
        assert_eq!(rramc.read_u32(OFF_EVENTS_READY).unwrap(), 0);

        // COMMITWRITEBUF empties it and READY changes to Ready.
        rramc.write_u32(OFF_TASKS_COMMITWRITEBUF, 1).unwrap();
        assert_eq!(rramc.read_u32(OFF_WRITEBUFEMPTY).unwrap(), 1);
        assert_eq!(rramc.read_u32(OFF_EVENTS_READY).unwrap(), 1);

        // Writing the last byte of the line fills the buffer.
        for offset in (0x2000..0x2010).step_by(4) {
            store(&mut rramc, offset, 4);
        }
        assert_eq!(rramc.read_u32(OFF_WRITEBUFEMPTY).unwrap(), 1);

        // Switching to Unbuffered leaves nothing uncommitted.
        store(&mut rramc, 0x2000, 4);
        assert_eq!(rramc.read_u32(OFF_WRITEBUFEMPTY).unwrap(), 0);
        rramc.write_u32(OFF_CONFIG, CONFIG_WEN).unwrap();
        assert_eq!(rramc.read_u32(OFF_WRITEBUFEMPTY).unwrap(), 1);
        rramc.write_u32(OFF_CONFIG, CONFIG_WEN | (1 << 8)).unwrap();

        // A store outside the buffer area commits the old buffer and starts
        // a new one.
        store(&mut rramc, 0x2010, 1);
        rramc.write_u32(OFF_EVENTS_READY, 0).unwrap();
        store(&mut rramc, 0x3000, 1);
        assert_eq!(rramc.read_u32(OFF_EVENTS_READY).unwrap(), 1);
        assert_eq!(rramc.read_u32(OFF_WRITEBUFEMPTY).unwrap(), 0);
    }

    #[test]
    fn unbuffered_store_latches_ready() {
        let mut rramc = Nrf54lRramc::new();
        rramc.write_u32(OFF_CONFIG, CONFIG_WEN).unwrap();
        assert_eq!(rramc.read_u32(OFF_EVENTS_READY).unwrap(), 0);
        assert_eq!(store(&mut rramc, 0x2000, 4), FlashStoreOutcome::Commit);
        assert_eq!(rramc.read_u32(OFF_EVENTS_READY).unwrap(), 1);
        assert_eq!(rramc.read_u32(OFF_WRITEBUFEMPTY).unwrap(), 1);

        // A dropped store (WEN clear) is not a commit.
        rramc.write_u32(OFF_EVENTS_READY, 0).unwrap();
        rramc.write_u32(OFF_CONFIG, 0).unwrap();
        assert_eq!(store(&mut rramc, 0x2000, 4), FlashStoreOutcome::Consumed);
        assert_eq!(rramc.read_u32(OFF_EVENTS_READY).unwrap(), 0);
    }

    #[test]
    fn intpend_follows_events_masked_by_inten() {
        let mut rramc = Nrf54lRramc::new();
        rramc.write_u32(OFF_TASKS_WAKEUP, 1).unwrap();
        assert_eq!(rramc.read_u32(OFF_EVENTS_WOKENUP).unwrap(), 1);
        assert_eq!(rramc.read_u32(OFF_INTPEND).unwrap(), 0);
        rramc.write_u32(OFF_INTENSET, 0b0011).unwrap();
        assert_eq!(rramc.read_u32(OFF_INTPEND).unwrap(), 0b0011);
        rramc.write_u32(OFF_INTENCLR, 0b0001).unwrap();
        assert_eq!(rramc.read_u32(OFF_INTPEND).unwrap(), 0b0010);
    }

    #[test]
    fn eraseall_fails_instead_of_reporting_a_fake_erase() {
        let mut rramc = Nrf54lRramc::new();
        assert!(rramc.write_u32(OFF_ERASEALL, 0).is_ok());
        assert!(rramc.write_u32(OFF_ERASEALL, 1).is_err());
    }

    #[test]
    fn region_registers_store_and_system_reset_restores_defaults() {
        let mut rramc = Nrf54lRramc::new();
        rramc.write_u32(OFF_REGION0 + 8, 0x0001_0000).unwrap();
        rramc.write_u32(OFF_REGION0 + 12, 0xFFFF_FFFF).unwrap();
        assert_eq!(rramc.read_u32(OFF_REGION0 + 8).unwrap(), 0x0001_0000);
        assert_eq!(
            rramc.read_u32(OFF_REGION0 + 12).unwrap(),
            REGION_CONFIG_MASK
        );
        rramc.write_u32(OFF_CONFIG, CONFIG_WEN).unwrap();
        rramc.on_system_reset();
        assert_eq!(rramc.read_u32(OFF_CONFIG).unwrap(), 0);
        assert_eq!(rramc.read_u32(OFF_REGION0 + 12).unwrap(), 0);
    }
}
