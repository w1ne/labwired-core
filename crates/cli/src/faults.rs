// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Apply declarative faults to a built bus and record per-fault evidence.
//!
//! Faults are declared in the test script (schema_version 1.1) and validated at
//! load. This module lowers each onto the engine and reports whether it actually
//! fired — the false-pass gate. Only `wrong_reset_value` is wired today; other
//! kinds report `fired: false` with a reason rather than silently passing.

use labwired_config::{FaultKind, FaultSpec};
use labwired_core::bus::SystemBus;
use serde::Serialize;

/// Per-fault evidence: did the injected fault actually take effect?
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FaultEvidence {
    pub id: String,
    pub kind: String,
    pub fired: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Apply each fault to the bus in order, returning one evidence record per fault.
pub fn apply_faults(bus: &mut SystemBus, faults: &[FaultSpec]) -> Vec<FaultEvidence> {
    faults.iter().map(|f| apply_one(bus, f)).collect()
}

fn apply_one(bus: &mut SystemBus, f: &FaultSpec) -> FaultEvidence {
    let kind = format!("{:?}", f.kind);
    let (fired, error) = match &f.kind {
        FaultKind::WrongResetValue => {
            match (f.target.peripheral.as_deref(), f.target.register.as_deref()) {
                (Some(p), Some(r)) => {
                    let value = f.value.unwrap_or(0) as u32;
                    match bus.inject_wrong_reset_value(p, r, value) {
                        Ok(()) => (true, None),
                        Err(e) => (false, Some(e)),
                    }
                }
                _ => (
                    false,
                    Some("missing target.peripheral/register".to_string()),
                ),
            }
        }
        FaultKind::StuckAtBit => {
            match (
                f.target.peripheral.as_deref(),
                f.target.register.as_deref(),
                f.target.bit,
                f.level,
            ) {
                (Some(p), Some(r), Some(bit), Some(level)) => {
                    match bus.inject_stuck_bit(p, r, bit, level) {
                        Ok(()) => (true, None),
                        Err(e) => (false, Some(e)),
                    }
                }
                _ => (
                    false,
                    Some("stuck_at_bit needs target.register, target.bit and level".to_string()),
                ),
            }
        }
        FaultKind::MissingClock => match f.target.peripheral.as_deref() {
            // fired is provisional here — it is finalised after the run, since
            // missing_clock only fires if the firmware accessed the peripheral.
            Some(p) => match bus.inject_missing_clock(p) {
                Ok(()) => (false, None),
                Err(e) => (false, Some(e)),
            },
            None => (false, Some("missing target.peripheral".to_string())),
        },
        other => (
            false,
            Some(format!("fault kind {other:?} not yet implemented")),
        ),
    };
    FaultEvidence {
        id: f.id.clone(),
        kind,
        fired,
        error,
    }
}

/// Finalise runtime-observed fault outcomes after the run. For `missing_clock`,
/// the fault fired only if the firmware actually accessed the unclocked
/// peripheral (an access was suppressed). Apply-time-known kinds are left as-is.
pub fn finalize_fault_evidence(
    bus: &SystemBus,
    faults: &[FaultSpec],
    evidence: &mut [FaultEvidence],
) {
    for f in faults {
        if !matches!(f.kind, FaultKind::MissingClock) {
            continue;
        }
        let Some(ev) = evidence.iter_mut().find(|e| e.id == f.id) else {
            continue;
        };
        if ev.error.is_some() {
            continue; // could not be applied; keep it not-fired with its reason
        }
        if let Some(p) = f.target.peripheral.as_deref() {
            ev.fired = bus.missing_clock_suppressed(p) > 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use labwired_config::{FaultKind, FaultSpec, FaultTarget, FaultTrigger};
    use labwired_core::Bus;

    fn spec(id: &str, kind: FaultKind, target: FaultTarget, value: Option<u64>) -> FaultSpec {
        FaultSpec {
            id: id.to_string(),
            kind,
            target,
            trigger: FaultTrigger::AtStart,
            level: None,
            value,
            xor: None,
            to: None,
            deny: None,
            delay_cycles: None,
            interrupt: None,
            bits: None,
            size: None,
        }
    }

    #[test]
    fn wrong_reset_value_on_missing_peripheral_does_not_fire() {
        let mut bus = SystemBus::new();
        let f = spec(
            "x",
            FaultKind::WrongResetValue,
            FaultTarget {
                peripheral: Some("nope".to_string()),
                register: Some("cr".to_string()),
                bit: None,
                address: None,
            },
            Some(1),
        );
        let ev = apply_faults(&mut bus, std::slice::from_ref(&f));
        assert_eq!(ev.len(), 1);
        assert!(!ev[0].fired);
        assert!(ev[0].error.as_ref().unwrap().contains("not found"));
    }

    #[test]
    fn unsupported_kind_reports_not_implemented() {
        let mut bus = SystemBus::new();
        let f = spec(
            "y",
            FaultKind::DelayedIrq,
            FaultTarget {
                peripheral: Some("usart1".to_string()),
                ..Default::default()
            },
            None,
        );
        let ev = apply_faults(&mut bus, std::slice::from_ref(&f));
        assert!(!ev[0].fired);
        assert!(ev[0]
            .error
            .as_ref()
            .unwrap()
            .contains("not yet implemented"));
    }

    #[test]
    fn missing_clock_apply_then_finalize_on_access() {
        use labwired_config::{Access, PeripheralDescriptor, RegisterDescriptor};
        use labwired_core::peripherals::declarative::GenericPeripheral;

        let desc = PeripheralDescriptor {
            peripheral: "t".to_string(),
            version: "1.0".to_string(),
            registers: vec![RegisterDescriptor {
                id: "r".to_string(),
                address_offset: 0,
                size: 32,
                access: Access::ReadWrite,
                reset_value: 0xAB,
                fields: vec![],
                side_effects: None,
            }],
            interrupts: None,
            timing: None,
        };
        let mut bus = SystemBus::new();
        bus.add_peripheral(
            "usart1",
            0x4000_0000,
            0x400,
            None,
            Box::new(GenericPeripheral::new(desc)),
        );

        let f = spec(
            "mc",
            FaultKind::MissingClock,
            FaultTarget {
                peripheral: Some("usart1".to_string()),
                ..Default::default()
            },
            None,
        );
        let mut ev = apply_faults(&mut bus, std::slice::from_ref(&f));
        assert!(!ev[0].fired, "fired is provisional before the run");

        // No access yet: still not fired.
        finalize_fault_evidence(&bus, std::slice::from_ref(&f), &mut ev);
        assert!(!ev[0].fired);

        // The firmware accesses the unclocked peripheral: now it fires.
        let _ = bus.read_u32(0x4000_0000);
        finalize_fault_evidence(&bus, std::slice::from_ref(&f), &mut ev);
        assert!(
            ev[0].fired,
            "missing_clock fires once the peripheral is accessed"
        );
    }

    #[test]
    fn stuck_at_bit_applies_and_reads_stuck() {
        use labwired_config::{Access, PeripheralDescriptor, RegisterDescriptor};
        use labwired_core::peripherals::declarative::GenericPeripheral;

        let desc = PeripheralDescriptor {
            peripheral: "t".to_string(),
            version: "1.0".to_string(),
            registers: vec![RegisterDescriptor {
                id: "sr".to_string(),
                address_offset: 0,
                size: 32,
                access: Access::ReadWrite,
                reset_value: 0,
                fields: vec![],
                side_effects: None,
            }],
            interrupts: None,
            timing: None,
        };
        let mut bus = SystemBus::new();
        bus.add_peripheral(
            "usart1",
            0x4000_0000,
            0x400,
            None,
            Box::new(GenericPeripheral::new(desc)),
        );

        let f = FaultSpec {
            id: "s".to_string(),
            kind: FaultKind::StuckAtBit,
            target: FaultTarget {
                peripheral: Some("usart1".to_string()),
                register: Some("sr".to_string()),
                bit: Some(7),
                address: None,
            },
            trigger: FaultTrigger::AtStart,
            level: Some(1),
            value: None,
            xor: None,
            to: None,
            deny: None,
            delay_cycles: None,
            interrupt: None,
            bits: None,
            size: None,
        };
        let ev = apply_faults(&mut bus, std::slice::from_ref(&f));
        assert!(ev[0].fired);
        assert!(ev[0].error.is_none());
        assert_eq!(bus.read_u32(0x4000_0000).unwrap() & (1 << 7), 1 << 7);
    }
}
