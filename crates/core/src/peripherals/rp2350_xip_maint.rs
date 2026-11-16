// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RP2350 XIP cache maintenance window (datasheet Release 8, section 4.4.1.2;
//! base `XIP_MAINTENANCE_BASE` 0x18000000, 64 MiB). A write's address selects
//! the cache line (set/way or address) and its low three bits the operation:
//! 0 invalidate by set/way, 1 clean by set/way, 2 invalidate by address,
//! 3 clean by address (pico-sdk `hardware_xip_cache`, `xip_cache_op_t`).
//!
//! The simulated flash is always coherent with what the CPU reads: there is
//! no cache data, tag RAM or dirty line, so cleaning and invalidating have
//! nothing to change and are accepted as the no-ops they are on a coherent
//! backing. Pinning (op 7, cache-as-SRAM) and the reserved ops 4..6 are
//! refused. The datasheet defines the window by its writes; a read performs
//! no maintenance and returns 0 here (the read data is not specified).

use crate::{Peripheral, SimResult, SimulationError};

const WINDOW: u64 = 0x0400_0000;

fn refuse(what: impl std::fmt::Display) -> SimulationError {
    SimulationError::NotImplemented(format!("RP2350 XIP maintenance: {what}"))
}

#[derive(Debug, Default)]
pub struct Rp2350XipMaintenance {
    /// Operations accepted, by op code 0..=3.
    ops: [u64; 4],
}

impl Rp2350XipMaintenance {
    pub fn new() -> Self {
        Self::default()
    }

    fn operate(&mut self, offset: u64) -> SimResult<()> {
        if offset >= WINDOW {
            return Err(refuse(format!("offset {offset:#x} outside the window")));
        }
        match offset & 7 {
            op @ 0..=3 => {
                self.ops[op as usize] += 1;
                Ok(())
            }
            7 => Err(refuse(
                "pinning cache lines (cache-as-SRAM) is not modelled",
            )),
            op => Err(refuse(format!("reserved operation {op}"))),
        }
    }
}

impl Peripheral for Rp2350XipMaintenance {
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }

    fn read_u32(&self, _offset: u64) -> SimResult<u32> {
        Ok(0)
    }

    fn write(&mut self, offset: u64, _value: u8) -> SimResult<()> {
        self.operate(offset)
    }

    fn write_u16(&mut self, offset: u64, _value: u16) -> SimResult<()> {
        self.operate(offset)
    }

    fn write_u32(&mut self, offset: u64, _value: u32) -> SimResult<()> {
        self.operate(offset)
    }

    fn peek(&self, _offset: u64) -> Option<u8> {
        Some(0)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "ops": self.ops,
            "scope": "coherent flash: clean/invalidate accepted as no-ops; pinning refused",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_and_invalidate_are_accepted_pin_and_reserved_refused() {
        let mut m = Rp2350XipMaintenance::new();
        // pico-sdk xip_cache_clean_all: byte stores at 0x1bffc001 + 8*i.
        m.write(0x03FF_C001, 0).unwrap();
        m.write(0x03FF_C000, 0).unwrap();
        m.write_u32(0x0000_1002, 0).unwrap();
        assert_eq!(m.ops, [1, 1, 1, 0]);
        assert!(m
            .write(0x0000_0007, 0)
            .unwrap_err()
            .to_string()
            .contains("pinning"));
        assert!(m
            .write(0x0000_0005, 0)
            .unwrap_err()
            .to_string()
            .contains("reserved"));
        assert_eq!(m.read_u32(0).unwrap(), 0, "a read performs no operation");
        assert_eq!(m.ops, [1, 1, 1, 0]);
    }
}
