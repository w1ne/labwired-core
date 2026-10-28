// SPDX-License-Identifier: MIT
//! Opt-in SAMD51 CMCC control-only slice (DS60001507F 11.6/11.10).
//! No cache data/tags/hit timing are modelled; backing memory remains coherent.
use crate::{Peripheral, SimResult, SimulationError};
use std::any::Any;
#[derive(Debug, Default)]
pub struct Samd51Cmcc {
    enabled: bool,
    invalidations: u64,
}
impl Samd51Cmcc {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn invalidations(&self) -> u64 {
        self.invalidations
    }
    fn refuse<T>() -> SimResult<T> {
        Err(SimulationError::NotImplemented(
            "samd51 CMCC control-only slice".into(),
        ))
    }
}
impl Peripheral for Samd51Cmcc {
    fn on_system_reset(&mut self) {
        // Cache disabled at power-on; the invalidation count is evidence.
        self.enabled = false;
    }
    fn read(&self, offset: u64) -> SimResult<u8> {
        match offset {
            8 | 12 => Ok(u8::from(self.enabled)),
            9..=11 | 13..=15 | 0x20..=0x23 => Ok(0),
            _ => Self::refuse(),
        }
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Self::refuse()
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match (offset, value) {
            (8, 0 | 1) => {
                self.enabled = value != 0;
                Ok(())
            }
            (0x20, 1) if !self.enabled => {
                self.invalidations += 1;
                Ok(())
            }
            _ => Self::refuse(),
        }
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }
    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn control_sequence_and_unadmitted_accesses() {
        let mut c = Samd51Cmcc::new();
        assert_eq!(c.read_u32(12).unwrap(), 0);
        c.write_u32(0x20, 1).unwrap();
        assert_eq!(c.invalidations(), 1);
        c.write_u32(8, 1).unwrap();
        assert_eq!(c.read_u32(12).unwrap(), 1);
        assert!(c.write_u32(0x20, 1).is_err());
        assert_eq!(c.invalidations(), 1);
        assert!(c.write_u32(8, 2).is_err());
        assert_eq!(c.read_u32(8).unwrap(), 1);
        c.write_u32(8, 0).unwrap();
        c.write_u32(0x20, 1).unwrap();
        assert_eq!(c.invalidations(), 2);
        assert!(c.read(0).is_err());
        assert!(c.write(8, 1).is_err());
    }
}
