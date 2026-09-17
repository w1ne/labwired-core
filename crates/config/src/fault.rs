// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

#![allow(dead_code)]
use crate::*;

/// Where a fault is applied. Either a peripheral (by `id`, optionally narrowed
/// to a `register` and `bit`) or a raw memory `address`. Resolved against the
/// built chip when the run starts.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct FaultTarget {
    #[serde(default)]
    pub peripheral: Option<String>,
    #[serde(default)]
    pub register: Option<String>,
    #[serde(default)]
    pub bit: Option<u8>,
    #[serde(default)]
    pub address: Option<u64>,
}

/// The access mode a `permission_flip` fault forces a register into.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccessMode {
    ReadOnly,
    WriteOnly,
}

/// The access direction a `permission_violation` fault denies.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccessDirection {
    Read,
    Write,
}

/// When a fault takes effect. Mirrors the declarative peripheral trigger
/// vocabulary so peripheral-class faults reuse the same evaluator.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FaultTrigger {
    /// Applied while the bus is built, before the firmware runs.
    #[default]
    AtStart,
    /// Applied once, `cycles` cycles into the run.
    AfterCycles { cycles: u64 },
    /// Applied when the firmware writes `register` (optionally matching value/mask).
    OnWrite {
        register: String,
        #[serde(default)]
        value: Option<u64>,
        #[serde(default)]
        mask: Option<u64>,
    },
    /// Applied when the firmware reads `register`.
    OnRead { register: String },
}

/// The taxonomy of injectable faults. Each maps to a documented silicon failure
/// mode; see the per-kind required parameters enforced in [`TestScript::validate`].
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FaultKind {
    MissingClock,
    StuckAtBit,
    WrongResetValue,
    PermissionFlip,
    BoundViolation,
    PermissionViolation,
    MemoryCorruption,
    DelayedIrq,
    NeverIrq,
    PeripheralErrorState,
    PeripheralTimeout,
}

/// A single injected fault. `kind`-specific parameters are the optional fields;
/// which are required is enforced structurally by [`TestScript::validate`].
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FaultSpec {
    pub id: String,
    pub kind: FaultKind,
    #[serde(default)]
    pub target: FaultTarget,
    #[serde(default)]
    pub trigger: FaultTrigger,
    /// `stuck_at_bit`: the level (0 or 1) the bit is held at.
    #[serde(default)]
    pub level: Option<u8>,
    /// `wrong_reset_value` / `memory_corruption`: the value written.
    #[serde(default)]
    pub value: Option<u64>,
    /// `memory_corruption`: XOR mask applied to the target instead of `value`.
    #[serde(default)]
    pub xor: Option<u64>,
    /// `permission_flip`: the mode to force the register into.
    #[serde(default)]
    pub to: Option<AccessMode>,
    /// `permission_violation`: the direction to deny.
    #[serde(default)]
    pub deny: Option<AccessDirection>,
    /// `delayed_irq`: how many cycles to delay the interrupt.
    #[serde(default)]
    pub delay_cycles: Option<u64>,
    /// `delayed_irq` / `never_irq`: the interrupt name on the peripheral.
    #[serde(default)]
    pub interrupt: Option<String>,
    /// `peripheral_error_state` / `peripheral_timeout`: the status bits to set.
    #[serde(default)]
    pub bits: Option<u64>,
    /// Memory-class faults: access width in bytes (1/2/4).
    #[serde(default)]
    pub size: Option<u8>,
}

/// The safe-behaviour judgment for a fault-injection run. `safe_when` reuses the
/// ordinary assertion vocabulary; the firmware passes iff every entry holds.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Verdict {
    #[serde(default)]
    pub safe_when: Vec<TestAssertion>,
    /// If true (default), the run is invalid — not a pass — unless every fault
    /// is observed to actually fire. The false-pass gate.
    #[serde(default = "default_true")]
    pub require_fault_fired: bool,
}

/// Per-kind structural validation of a fault spec: that the target shape and the
/// kind-specific parameters required to lower the fault are present. This is the
/// config-side half of the fault compiler; silicon-resolution guardrails (does
/// the peripheral exist, is the bit within the register) run against the built
/// bus at run time.
pub(crate) fn validate_fault(f: &FaultSpec) -> Result<()> {
    // Every implemented fault is lowered onto the bus before the firmware runs
    // (see `labwired_cli::faults`), and nothing evaluates a fault's trigger
    // after that. A later trigger would therefore fire at start while the
    // script said otherwise, so refuse it the way stimuli refuse the triggers
    // they do not wire.
    if f.trigger != FaultTrigger::AtStart {
        anyhow::bail!(
            "Fault '{}' ({:?}): trigger {:?} is not yet supported for faults; every fault is \
             applied when the bus is built (use at_start, or omit trigger)",
            f.id,
            f.kind,
            f.trigger
        );
    }
    let needs_peripheral = || -> Result<()> {
        if f.target.peripheral.is_none() {
            anyhow::bail!("Fault '{}' ({:?}) needs target.peripheral", f.id, f.kind);
        }
        Ok(())
    };
    let needs_register = || -> Result<()> {
        if f.target.register.is_none() {
            anyhow::bail!("Fault '{}' ({:?}) needs target.register", f.id, f.kind);
        }
        Ok(())
    };
    let needs_address = || -> Result<()> {
        if f.target.address.is_none() {
            anyhow::bail!("Fault '{}' ({:?}) needs target.address", f.id, f.kind);
        }
        Ok(())
    };

    match f.kind {
        FaultKind::MissingClock => needs_peripheral()?,
        FaultKind::StuckAtBit => {
            needs_peripheral()?;
            needs_register()?;
            if f.target.bit.is_none() {
                anyhow::bail!("Fault '{}' (stuck_at_bit) needs target.bit", f.id);
            }
            match f.level {
                Some(0) | Some(1) => {}
                _ => anyhow::bail!("Fault '{}' (stuck_at_bit) needs level: 0 or 1", f.id),
            }
        }
        FaultKind::WrongResetValue => {
            needs_peripheral()?;
            needs_register()?;
            if f.value.is_none() {
                anyhow::bail!("Fault '{}' (wrong_reset_value) needs 'value'", f.id);
            }
        }
        FaultKind::PermissionFlip => {
            needs_peripheral()?;
            needs_register()?;
            if f.to.is_none() {
                anyhow::bail!("Fault '{}' (permission_flip) needs 'to'", f.id);
            }
        }
        FaultKind::BoundViolation => needs_address()?,
        FaultKind::PermissionViolation => {
            needs_address()?;
            if f.deny.is_none() {
                anyhow::bail!("Fault '{}' (permission_violation) needs 'deny'", f.id);
            }
        }
        FaultKind::MemoryCorruption => {
            needs_address()?;
            if f.value.is_none() && f.xor.is_none() {
                anyhow::bail!(
                    "Fault '{}' (memory_corruption) needs 'value' or 'xor'",
                    f.id
                );
            }
        }
        FaultKind::DelayedIrq => {
            needs_peripheral()?;
            if f.interrupt.is_none() {
                anyhow::bail!("Fault '{}' (delayed_irq) needs 'interrupt'", f.id);
            }
            if f.delay_cycles.is_none() {
                anyhow::bail!("Fault '{}' (delayed_irq) needs 'delay_cycles'", f.id);
            }
        }
        FaultKind::NeverIrq => {
            needs_peripheral()?;
            if f.interrupt.is_none() {
                anyhow::bail!("Fault '{}' (never_irq) needs 'interrupt'", f.id);
            }
        }
        FaultKind::PeripheralErrorState | FaultKind::PeripheralTimeout => {
            needs_peripheral()?;
            needs_register()?;
            if f.bits.is_none() {
                anyhow::bail!("Fault '{}' ({:?}) needs 'bits'", f.id, f.kind);
            }
        }
    }
    Ok(())
}
