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

#[cfg(test)]
mod tests {
    use super::*;
    use labwired_config::{FaultKind, FaultSpec, FaultTarget, FaultTrigger};

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
}
