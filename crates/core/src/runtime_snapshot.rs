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
///
/// v2 added the self-key fields (`chip` + `firmware_sha256`) so a resume
/// snapshot can be validated against the firmware it is loaded on top of.
/// No v1 production blobs exist, so v1 is rejected cleanly on read.
pub const RUNTIME_SNAPSHOT_VERSION: u32 = 2;

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
    /// Self-key (v2): the chip this snapshot was captured on (e.g.
    /// `"esp32c3"`). A fresh snapshot from [`Self::new`] leaves this empty
    /// until [`Self::set_self_key`] stamps it. Resume validates it against
    /// the machine being restored so a snapshot cannot be applied to the
    /// wrong chip.
    #[serde(default)]
    pub chip: String,
    /// Self-key (v2): SHA-256 of the firmware the snapshot was captured
    /// against (for rom-boot, the flash-image bytes). Resume re-hashes the
    /// firmware it loads and refuses to apply a snapshot whose hash differs
    /// — the runtime snapshot deliberately omits the flash/rom mirrors and
    /// relies on the same firmware being reloaded first, so a mismatch would
    /// silently corrupt state without this gate. All-zero when unkeyed.
    #[serde(default)]
    pub firmware_sha256: [u8; 32],
    /// Flat linear memory windows (`base_addr` -> bytes) that hold live program
    /// state which is NOT peripheral-backed. RISC-V (ESP32-C3 rom-boot) keeps
    /// its `.data`/`.bss`/stack in the SRAM/IRAM linear windows (`bus.ram` +
    /// `bus.extra_mem`) rather than in `RamPeripheral`s, so those must be
    /// captured here for a resume to see the running program's memory. Empty
    /// for Xtensa/Arm snapshots, whose RAM is peripheral-backed (captured in
    /// `peripherals`) and whose linear windows are firmware mirrors re-derived
    /// on load. Untouched (all-zero) windows are skipped to stay compact.
    #[serde(default)]
    pub memories: Vec<(u64, Vec<u8>)>,
}

impl MachineRuntimeSnapshot {
    /// Build a fresh snapshot frame with magic + version baked in. The
    /// self-key fields (`chip`, `firmware_sha256`) start empty/zero — call
    /// [`Self::set_self_key`] to stamp them before serializing a resume
    /// snapshot.
    pub fn new(cpu_kind: CpuKind, cpu_data: Vec<u8>, peripherals: Vec<(String, Vec<u8>)>) -> Self {
        Self {
            magic: RUNTIME_SNAPSHOT_MAGIC,
            version: RUNTIME_SNAPSHOT_VERSION,
            cpu_kind,
            cpu_data,
            peripherals,
            chip: String::new(),
            firmware_sha256: [0u8; 32],
            memories: Vec::new(),
        }
    }

    /// Stamp the self-key: the chip the snapshot was captured on and the
    /// SHA-256 of the firmware it was captured against. Callers set this on
    /// a snapshot returned by `Machine::take_runtime_snapshot` (which cannot
    /// know the chip identity or firmware bytes) before writing it out.
    pub fn set_self_key(&mut self, chip: impl Into<String>, firmware_sha256: [u8; 32]) {
        self.chip = chip.into();
        self.firmware_sha256 = firmware_sha256;
    }

    /// Validate the self-key against the chip + firmware a resume is about
    /// to apply this snapshot on top of. Returns [`RuntimeSnapshotError::SelfKeyMismatch`]
    /// on any divergence so the caller can fall back to a cold boot instead
    /// of restoring incompatible state.
    pub fn validate_self_key(
        &self,
        chip: &str,
        firmware_sha256: &[u8; 32],
    ) -> Result<(), RuntimeSnapshotError> {
        if self.chip != chip {
            return Err(RuntimeSnapshotError::SelfKeyMismatch(format!(
                "chip mismatch: snapshot is for '{}', run is '{chip}'",
                self.chip
            )));
        }
        if &self.firmware_sha256 != firmware_sha256 {
            return Err(RuntimeSnapshotError::SelfKeyMismatch(format!(
                "firmware mismatch: snapshot sha256 {} != run sha256 {}",
                hex32(&self.firmware_sha256),
                hex32(firmware_sha256)
            )));
        }
        Ok(())
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
    #[error("snapshot self-key mismatch: {0}")]
    SelfKeyMismatch(String),
}

/// Lower-case hex of a 32-byte digest, for self-key mismatch diagnostics.
fn hex32(bytes: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(64);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
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

/// Full RISC-V (RV32, ESP32-C3) CPU state needed for mid-flight resume.
///
/// The RISC-V core is flat — no register windows, no shadow stacks — so the
/// whole architectural + timer/CSR state fits in these fields. Unlike the
/// JSON-side `RiscVCpuSnapshot`, this also captures the LR/SC `reservation`
/// so a snapshot taken between an `LR.W` and its `SC.W` resumes coherently.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiscVRuntimeSnapshot {
    pub x: [u32; 32],
    pub pc: u32,
    pub mstatus: u32,
    pub mie: u32,
    pub mip: u32,
    pub mtvec: u32,
    pub mscratch: u32,
    pub mepc: u32,
    pub mcause: u32,
    pub mtval: u32,
    pub mtime: u64,
    pub mtimecmp: u64,
    pub reservation: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyed(chip: &str, sha: [u8; 32]) -> MachineRuntimeSnapshot {
        let mut s = MachineRuntimeSnapshot::new(CpuKind::RiscV, vec![], vec![]);
        s.set_self_key(chip, sha);
        s
    }

    #[test]
    fn self_key_roundtrips_through_bytes() {
        let sha = [7u8; 32];
        let snap = keyed("esp32c3", sha);
        let decoded = MachineRuntimeSnapshot::from_bytes(&snap.to_bytes()).expect("decode");
        assert_eq!(decoded.chip, "esp32c3");
        assert_eq!(decoded.firmware_sha256, sha);
        // Matching key validates.
        decoded.validate_self_key("esp32c3", &sha).expect("match");
    }

    #[test]
    fn self_key_rejects_chip_mismatch() {
        let snap = keyed("esp32c3", [1u8; 32]);
        let err = snap.validate_self_key("esp32s3", &[1u8; 32]).unwrap_err();
        assert!(
            matches!(err, RuntimeSnapshotError::SelfKeyMismatch(_)),
            "got {err}"
        );
        assert!(format!("{err}").contains("chip mismatch"), "got {err}");
    }

    #[test]
    fn self_key_rejects_firmware_hash_mismatch() {
        let snap = keyed("esp32c3", [1u8; 32]);
        let err = snap.validate_self_key("esp32c3", &[2u8; 32]).unwrap_err();
        assert!(
            matches!(err, RuntimeSnapshotError::SelfKeyMismatch(_)),
            "got {err}"
        );
        assert!(format!("{err}").contains("firmware mismatch"), "got {err}");
    }

    #[test]
    fn v1_blob_is_rejected() {
        // Craft a v1-shaped blob (no self-key fields) and prove from_bytes
        // rejects it now that the version is 2 — a v1 blob either fails to
        // deserialize into the v2 struct or trips the version gate.
        let mut snap = MachineRuntimeSnapshot::new(CpuKind::RiscV, vec![], vec![]);
        snap.version = 1;
        let bytes = snap.to_bytes();
        let err = MachineRuntimeSnapshot::from_bytes(&bytes).unwrap_err();
        assert!(
            matches!(err, RuntimeSnapshotError::VersionMismatch { .. }),
            "got {err}"
        );
    }
}
