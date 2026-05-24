// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Binary, mid-flight resume snapshots for a `Machine`.
//!
//! This is **separate from** the JSON-based [`crate::snapshot::MachineSnapshot`],
//! which is used by the determinism gates and the CLI's snapshot/replay
//! tools — those want human-inspectable, schema-versioned, JSON-friendly
//! state. The runtime snapshot in this module is the opposite end of the
//! tradeoff: a compact binary blob produced by a one-time offline boot run
//! and shipped alongside firmware ELFs so the playground can resume from
//! mid-execution instead of paying the ~30 s cold-start every time.
//!
//! # The deal
//!
//! - `MachineRuntimeSnapshot` captures **full CPU state** (including the
//!   64-entry physical AR file, every shadow-spill frame, and the full
//!   256-entry SR file — none of which the JSON snapshot keeps) plus a
//!   per-peripheral `Vec<u8>` blob.
//! - Each `Peripheral` exposes `runtime_snapshot() -> Vec<u8>` /
//!   `restore_runtime_snapshot(&[u8])`. Default impl in the trait returns
//!   an empty blob — peripherals with no resume-critical state are free
//!   to ignore it. Override for stateful peripherals (RAM regions, the
//!   SSD1680 panel, SystemStub-backed register banks).
//! - Format is bincode-encoded — fast deserialize, compact on the wire,
//!   no schema versioning beyond the top-level `version` field. Bump
//!   `RUNTIME_SNAPSHOT_VERSION` when adding fields; consumers reject
//!   mismatched versions outright (snapshots regenerate cheaply).

use serde::{Deserialize, Serialize};

/// Magic bytes at the start of every serialized snapshot. Lets the
/// playground's loader distinguish a snapshot blob from random bytes.
pub const RUNTIME_SNAPSHOT_MAGIC: [u8; 4] = *b"LWRS";

/// Bump when the on-disk format changes. Old snapshots are rejected (we
/// re-generate them as part of the firmware-release pipeline, so there's
/// no compat burden).
pub const RUNTIME_SNAPSHOT_VERSION: u32 = 1;

/// Peripherals deliberately excluded from runtime snapshots — their
/// contents are content-addressable from the ELF (re-applying
/// `load_firmware` restores them) and including them inflates the blob
/// by ~12 MiB per snapshot. The snapshot consumer must guarantee the
/// same firmware ELF is loaded before `apply_runtime_snapshot`.
pub const RUNTIME_SNAPSHOT_SKIPPED_PERIPHERALS: &[&str] = &[
    "flash_icache", // 4 MiB ELF .text mirror
    "flash_dcache", // 4 MiB ELF .rodata mirror
    "psram",        // 4 MiB stub, probed-only
    "rom",          // BROM thunks, deterministic from configure_*
];

/// CPU-arch discriminator inside [`MachineRuntimeSnapshot::cpu_data`].
/// One byte so the on-disk format stays compact.
#[repr(u8)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CpuKind {
    XtensaLx7 = 0,
    ArmCortexM = 1,
    RiscV = 2,
}

/// Top-level container. CPU state is opaque bytes (interpreted via
/// `cpu_kind`); peripherals are a flat `(name, bytes)` list — looked up
/// by `SystemBus` peripheral name at restore time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineRuntimeSnapshot {
    pub magic: [u8; 4],
    pub version: u32,
    pub cpu_kind: CpuKind,
    pub cpu_data: Vec<u8>,
    pub peripherals: Vec<(String, Vec<u8>)>,
}

impl MachineRuntimeSnapshot {
    /// Build a fresh snapshot frame with magic + version baked in.
    pub fn new(cpu_kind: CpuKind, cpu_data: Vec<u8>, peripherals: Vec<(String, Vec<u8>)>) -> Self {
        Self {
            magic: RUNTIME_SNAPSHOT_MAGIC,
            version: RUNTIME_SNAPSHOT_VERSION,
            cpu_kind,
            cpu_data,
            peripherals,
        }
    }

    /// Serialize to a self-contained byte blob.
    pub fn to_bytes(&self) -> Vec<u8> {
        bincode::serialize(self).expect("bincode serialize MachineRuntimeSnapshot")
    }

    /// Parse from a byte blob, validating magic + version.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, RuntimeSnapshotError> {
        let snap: Self =
            bincode::deserialize(bytes).map_err(|e| RuntimeSnapshotError::Decode(e.to_string()))?;
        if snap.magic != RUNTIME_SNAPSHOT_MAGIC {
            return Err(RuntimeSnapshotError::BadMagic(snap.magic));
        }
        if snap.version != RUNTIME_SNAPSHOT_VERSION {
            return Err(RuntimeSnapshotError::VersionMismatch {
                expected: RUNTIME_SNAPSHOT_VERSION,
                got: snap.version,
            });
        }
        Ok(snap)
    }
}

/// Errors from snapshot encode/decode. Restoration errors during
/// `apply_runtime_snapshot` flow through the regular `SimulationError`
/// path instead.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeSnapshotError {
    #[error("bincode decode failed: {0}")]
    Decode(String),
    #[error("bad magic bytes: {0:?}")]
    BadMagic([u8; 4]),
    #[error("snapshot version mismatch: expected {expected}, got {got}")]
    VersionMismatch { expected: u32, got: u32 },
    #[error("missing peripheral '{0}' in snapshot")]
    MissingPeripheral(String),
}

// ── CPU-side payloads ────────────────────────────────────────────────────────

/// Full Xtensa LX7 CPU state needed for mid-flight resume.
///
/// Critically different from the JSON-side `XtensaLx7CpuSnapshot`:
/// * captures **all 64 physical AR registers** (the JSON snapshot only
///   has 16 logical regs — useless after windowed CALLs have rotated WB)
/// * captures the **full 256-entry SR file** (the JSON snapshot has only
///   `vecbase`)
/// * captures the **shadow-spill state** (a per-WB stack of saved frames
///   the sim uses for transparent overflow handling)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XtensaLx7RuntimeSnapshot {
    pub pc: u32,
    pub ps_raw: u32,
    /// 64-entry physical AR file.
    pub phys: Vec<u32>,
    pub window_base: u8,
    pub window_start: u16,
    /// Sim-level shadow-spill stacks — 16 slots, each is a stack of saved
    /// 4-register frames. Stored as `Vec<Vec<[u32; 4]>>` so bincode can
    /// roundtrip without const-generic dance.
    pub shadow: Vec<Vec<[u32; 4]>>,
    /// 256-entry SR file. Most entries are zero; bincode varint-style
    /// encoding keeps the payload modest.
    pub sr: Vec<u32>,
}
