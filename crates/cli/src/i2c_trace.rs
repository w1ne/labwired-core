//! Compact I²C address evidence for `result.json`, taken from the universal
//! bus-trace ring already recorded during `labwired test`.
//!
//! The hosted oracle needs to judge "only address 0x68" without a full
//! `--bus-trace-out` sidecar. This summary is the smallest additive field
//! that can support that claim: the unique 7-bit addresses seen in I²C
//! address phases, plus whether the shared ring evicted any events (so an
//! exclusivity claim cannot be proved from a partial trace).

use labwired_core::bus::bus_trace::{BusPayload, BusTraceEvent, I2cSym};

/// Unique 7-bit I²C addresses observed on the bus-trace ring, plus whether
/// the ring dropped events and may therefore have lost an address phase.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct I2cTraceSummary {
    pub addresses: Vec<u8>,
    pub evicted: bool,
}

/// Collect unique 7-bit addresses from I²C address phases (`AddrWrite` /
/// `AddrRead`). Data bytes are ignored. `evicted` is the shared ring's
/// eviction count — UART/SPI/CAN traffic can evict an I²C address phase, so
/// any eviction makes an "only these addresses" claim inconclusive.
pub(crate) fn i2c_trace_summary(events: &[BusTraceEvent], evicted: u64) -> I2cTraceSummary {
    use std::collections::BTreeSet;
    let mut addresses = BTreeSet::new();
    for event in events {
        if let BusPayload::I2c {
            kind: I2cSym::AddrWrite | I2cSym::AddrRead,
            byte,
            ..
        } = event.payload
        {
            addresses.insert(byte >> 1);
        }
    }
    I2cTraceSummary {
        addresses: addresses.into_iter().collect(),
        evicted: evicted > 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use labwired_core::bus::bus_trace::BusTraceEvent;

    fn i2c_event(kind: I2cSym, byte: u8) -> BusTraceEvent {
        BusTraceEvent {
            seq: 1,
            cycle: 0,
            bus: "i2c0".into(),
            payload: BusPayload::I2c {
                kind,
                byte,
                ack: true,
            },
        }
    }

    #[test]
    fn address_phases_yield_unique_sorted_seven_bit_addresses() {
        let events = [
            i2c_event(I2cSym::AddrWrite, 0x68 << 1),
            i2c_event(I2cSym::Data, 0x00),
            i2c_event(I2cSym::AddrRead, (0x68 << 1) | 1),
            i2c_event(I2cSym::AddrWrite, 0x57 << 1),
        ];
        let summary = i2c_trace_summary(&events, 0);
        assert_eq!(summary.addresses, vec![0x57, 0x68]);
        assert!(!summary.evicted);
    }

    #[test]
    fn empty_trace_without_eviction_is_the_default_omitted_shape() {
        assert_eq!(i2c_trace_summary(&[], 0), I2cTraceSummary::default());
    }

    #[test]
    fn any_ring_eviction_marks_the_summary_incomplete() {
        let summary = i2c_trace_summary(&[i2c_event(I2cSym::AddrWrite, 0x68 << 1)], 3);
        assert_eq!(summary.addresses, vec![0x68]);
        assert!(summary.evicted);
        assert!(i2c_trace_summary(&[], 1).evicted);
    }
}
