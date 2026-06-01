// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 USBREGULATOR (USB 3.3 V regulator).
//!
//! Source: nRF52840 PS rev 1.7 §6.36 (USBREGULATOR). On real silicon
//! the regulator is enabled before the USBD core; firmware checks
//! its status to know when VBUS is steady. We synthesise OUTPUTRDY=1
//! immediately on enable so init loops proceed.

use crate::{Peripheral, SimResult};

const OFF_EVENTS_USBPWRRDY: u64 = 0x108;
const OFF_INTEN: u64 = 0x300;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;
const OFF_USBREGSTATUS: u64 = 0x400;
const OFF_USBPWRRDY_TASKS_START: u64 = 0x000;

#[derive(Debug, Default)]
pub struct Nrf52UsbRegulator {
    events_usbpwrrdy: u32,
    inten: u32,
    usbregstatus: u32,
}

impl Nrf52UsbRegulator {
    pub fn new() -> Self {
        // VBUSDETECT = bit 0; OUTPUTRDY = bit 1. Default: both visible
        // (we treat VBUS as steady because USBREG is enabled in sim).
        Self {
            usbregstatus: 0x3,
            ..Self::default()
        }
    }
}

impl Peripheral for Nrf52UsbRegulator {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            OFF_USBPWRRDY_TASKS_START => 0,
            OFF_EVENTS_USBPWRRDY => self.events_usbpwrrdy,
            OFF_INTEN | OFF_INTENSET | OFF_INTENCLR => self.inten,
            OFF_USBREGSTATUS => self.usbregstatus & 0x3,
            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_USBPWRRDY_TASKS_START if value & 1 != 0 => {
                self.events_usbpwrrdy = 1;
            }
            OFF_EVENTS_USBPWRRDY => self.events_usbpwrrdy = value & 1,
            OFF_INTEN => self.inten = value & 1,
            OFF_INTENSET => self.inten |= value & 1,
            OFF_INTENCLR => self.inten &= !value,
            OFF_USBREGSTATUS => {} // RO
            _ => {}
        }
        Ok(())
    }
}
