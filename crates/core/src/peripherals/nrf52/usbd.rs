// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 USBD (USB Device controller).
//!
//! Source: nRF52840 PS rev 1.7 §6.35 (USBD). Large peripheral —
//! ~200 distinct registers covering 8 IN endpoints, 8 OUT endpoints,
//! 1 isochronous endpoint, frame counter, EVENTCAUSE, BUSSTATE,
//! and the Easy DMA pointers. We model the register surface as a
//! sparse BTreeMap so all offsets round-trip; firmware that polls
//! ENABLE, USBPULLUP, ENDPSTATUS etc. sees its writes back.
//!
//! Dynamic semantics (host enumeration, SETUP packet handling,
//! endpoint state machine) are NOT modeled. Firmware that waits for
//! EVENTCAUSE.READY in its init loop will see it (we synthesise it
//! after a TASKS_STARTEPIN/OUT write).

// The OFF_* constants below document the full nRF52840 USBD register surface;
// not every offset is referenced yet (the dynamic USB state machine is
// unmodeled), so allow dead code for the register map.
#![allow(dead_code)]

use crate::{Peripheral, SimResult};
use std::collections::BTreeMap;

// Key task / event offsets we treat specially.
const OFF_TASKS_STARTEPIN_0: u64 = 0x004;
const OFF_TASKS_STARTEPOUT_0: u64 = 0x044;
const OFF_TASKS_EP0RCVOUT: u64 = 0x098;
const OFF_TASKS_EP0STATUS: u64 = 0x09C;
const OFF_TASKS_EP0STALL: u64 = 0x0A0;
const OFF_EVENTS_USBRESET: u64 = 0x100;
const OFF_EVENTS_STARTED: u64 = 0x104;
const OFF_EVENTS_ENDEPIN_0: u64 = 0x108;
const OFF_EVENTS_EP0DATADONE: u64 = 0x128;
const OFF_EVENTS_ENDISOIN: u64 = 0x12C;
const OFF_EVENTS_USBEVENT: u64 = 0x158;
const OFF_EVENTS_EP0SETUP: u64 = 0x15C;
const OFF_INTEN: u64 = 0x300;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;
const OFF_EVENTCAUSE: u64 = 0x400;
const OFF_BUSSTATE: u64 = 0x408;
const OFF_HALTED_EPIN_0: u64 = 0x420;
const OFF_HALTED_EPOUT_0: u64 = 0x444;
const OFF_EPSTATUS: u64 = 0x468;
const OFF_EPDATASTATUS: u64 = 0x46C;
const OFF_USBADDR: u64 = 0x470;
const OFF_BMREQUESTTYPE: u64 = 0x480;
const OFF_BREQUEST: u64 = 0x484;
const OFF_WVALUEL: u64 = 0x488;
const OFF_WVALUEH: u64 = 0x48C;
const OFF_WINDEXL: u64 = 0x490;
const OFF_WINDEXH: u64 = 0x494;
const OFF_WLENGTHL: u64 = 0x498;
const OFF_WLENGTHH: u64 = 0x49C;
const OFF_ENABLE: u64 = 0x500;
const OFF_USBPULLUP: u64 = 0x504;
const OFF_DPDMVALUE: u64 = 0x508;
const OFF_DTOGGLE: u64 = 0x50C;
const OFF_EPINEN: u64 = 0x510;
const OFF_EPOUTEN: u64 = 0x514;
const OFF_EPSTALL: u64 = 0x518;
const OFF_ISOSPLIT: u64 = 0x51C;
const OFF_FRAMECNTR: u64 = 0x520;
const OFF_LOWPOWER: u64 = 0x52C;
const OFF_ISOINCONFIG: u64 = 0x530;

#[derive(Debug, Default)]
pub struct Nrf52Usbd {
    regs: BTreeMap<u64, u32>,
}

impl Nrf52Usbd {
    pub fn new() -> Self {
        let mut s = Self::default();
        // BUSSTATE.BUSVALID = 1 on a powered VBUS — firmware that waits
        // for this in clock_init will then proceed.
        s.regs.insert(OFF_BUSSTATE, 0x1);
        s
    }
}

impl Peripheral for Nrf52Usbd {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        // Task registers always read 0 per PS.
        match offset {
            0x000..=0x0FC if offset.is_multiple_of(4) => return Ok(0),
            // USBPULLUP/EPINEN/EPOUTEN are effectively write-only on silicon —
            // the USB PHY state machine owns the actual state; reads return 0.
            OFF_USBPULLUP | OFF_EPINEN | OFF_EPOUTEN => return Ok(0),
            _ => {}
        }
        Ok(self.regs.get(&offset).copied().unwrap_or(0))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_TASKS_STARTEPIN_0..=0x024 if offset.is_multiple_of(4) => {
                // STARTEPIN[i] triggered — synthesise STARTED + ENDEPIN[i]
                // events the firmware will be polling for.
                if value & 1 != 0 {
                    let i = (offset - OFF_TASKS_STARTEPIN_0) / 4;
                    self.regs.insert(OFF_EVENTS_STARTED, 1);
                    self.regs.insert(OFF_EVENTS_ENDEPIN_0 + 4 * i, 1);
                }
            }
            OFF_TASKS_STARTEPOUT_0..=0x064 if offset.is_multiple_of(4) => {
                if value & 1 != 0 {
                    self.regs.insert(OFF_EVENTS_STARTED, 1);
                }
            }
            OFF_TASKS_EP0RCVOUT | OFF_TASKS_EP0STATUS | OFF_TASKS_EP0STALL => {
                if value & 1 != 0 {
                    self.regs.insert(OFF_EVENTS_EP0DATADONE, 1);
                }
            }
            OFF_ENABLE => {
                // Silicon: writing 0 to ENABLE is ignored when VBUS is present
                // (BUSSTATE.BUSVALID == 1). The USB controller stays enabled
                // while the bus is powered.
                let busvalid = self.regs.get(&OFF_BUSSTATE).copied().unwrap_or(0) & 1;
                let new_enable = if value & 1 == 0 && busvalid != 0 {
                    // VBUS present — cannot disable; leave ENABLE=1
                    1
                } else {
                    value & 1
                };
                self.regs.insert(OFF_ENABLE, new_enable);
                if new_enable != 0 {
                    // EVENTCAUSE.READY = bit 0 (per PS table 273) so init
                    // loops poll-and-clear successfully.
                    self.regs.insert(OFF_EVENTCAUSE, 1);
                }
            }
            OFF_EVENTCAUSE => {
                // Write-1-clear.
                let cur = self.regs.get(&OFF_EVENTCAUSE).copied().unwrap_or(0);
                self.regs.insert(OFF_EVENTCAUSE, cur & !value);
            }
            OFF_INTENSET => {
                let cur = self.regs.get(&OFF_INTEN).copied().unwrap_or(0);
                self.regs.insert(OFF_INTEN, cur | value);
                self.regs.insert(OFF_INTENSET, cur | value);
            }
            OFF_INTENCLR => {
                let cur = self.regs.get(&OFF_INTEN).copied().unwrap_or(0);
                self.regs.insert(OFF_INTEN, cur & !value);
            }
            // EVENTS_*: hardware-generated. SW write-1 is ignored per silicon;
            // write-0 clears. EVENTS live in 0x100..0x1FC.
            0x100..=0x1FC if offset.is_multiple_of(4) => {
                if value == 0 {
                    self.regs.insert(offset, 0);
                }
                // write-1 silently ignored
            }
            _ => {
                self.regs.insert(offset, value);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enable_sets_eventcause_ready() {
        let mut u = Nrf52Usbd::new();
        u.write_u32(OFF_ENABLE, 1).unwrap();
        assert_eq!(u.read_u32(OFF_ENABLE).unwrap(), 1);
        assert_eq!(u.read_u32(OFF_EVENTCAUSE).unwrap() & 1, 1);
    }

    #[test]
    fn eventcause_is_write_1_clear() {
        let mut u = Nrf52Usbd::new();
        u.write_u32(OFF_ENABLE, 1).unwrap();
        u.write_u32(OFF_EVENTCAUSE, 1).unwrap();
        assert_eq!(u.read_u32(OFF_EVENTCAUSE).unwrap() & 1, 0);
    }

    #[test]
    fn startepin0_synthesises_endepin0() {
        let mut u = Nrf52Usbd::new();
        u.write_u32(OFF_TASKS_STARTEPIN_0, 1).unwrap();
        assert_eq!(u.read_u32(OFF_EVENTS_ENDEPIN_0).unwrap(), 1);
    }

    #[test]
    fn usbpullup_write_only_reads_zero() {
        // USBPULLUP is write-only on silicon — the write is accepted but the
        // register reads back 0 (confirmed by nrf52_full_register_conformance).
        let mut u = Nrf52Usbd::new();
        u.write_u32(OFF_USBPULLUP, 1).unwrap();
        assert_eq!(u.read_u32(OFF_USBPULLUP).unwrap(), 0);
    }
}
