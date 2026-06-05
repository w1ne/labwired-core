// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Behavioral register-coverage probe.
//!
//! A register is judged purely by what the model DOES, never by what an author
//! claims. For each register we compare its read/write behavior against the
//! peripheral's own *unmapped-offset* behavior (the catch-all baseline):
//!
//! * If unmapped offsets round-trip writes, the peripheral is generic storage —
//!   write-readback proves nothing, so we fall back to read-vs-reset only.
//! * Otherwise a register that retains a written sentinel (distinct from the
//!   catch-all) is `Modelled`; a read-write register that behaves exactly like
//!   an unmapped offset is an accept-and-ignore stub → `Unmodelled`.
//! * Cases we genuinely cannot decide behaviorally (a write-only trigger with no
//!   read-back, a read-only status reading the catch-all value) are
//!   `Indeterminate` — the per-peripheral FSM tests confirm those.

/// Register access type (mirror of `labwired_config::Access`, kept dep-free here).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    ReadWrite,
    ReadOnly,
    WriteOnly,
}

/// How faithfully a single register is modelled, by observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegStatus {
    Modelled,
    Unmodelled,
    Indeterminate,
}

/// A register to probe.
#[derive(Debug, Clone)]
pub struct ProbeReg {
    pub name: String,
    pub offset: u64,
    pub access: Access,
    pub reset_value: u32,
}

/// Probe result for one register.
#[derive(Debug, Clone)]
pub struct RegResult {
    pub name: String,
    pub offset: u64,
    pub status: RegStatus,
}

/// Anything we can read/write u32s on at byte offsets. Errors map to `None`/false.
pub trait ProbeTarget {
    fn probe_read(&self, offset: u64) -> Option<u32>;
    fn probe_write(&mut self, offset: u64, value: u32) -> bool;
}

const SENTINEL: u32 = 0xA5A5_A5A5;
const SENTINEL_ALT: u32 = 0x5A5A_5A5A;

struct Baseline {
    read: u32,
    write_roundtrips: bool,
}

fn compute_baseline(target: &mut dyn ProbeTarget, regs: &[ProbeReg], window_size: u64) -> Baseline {
    let used: std::collections::HashSet<u64> = regs.iter().map(|r| r.offset & !3).collect();
    let mut unmapped: Vec<u64> = Vec::new();
    let mut off = (window_size.saturating_sub(4)) & !3;
    while unmapped.len() < 4 && off >= 4 {
        if !used.contains(&off) {
            unmapped.push(off);
        }
        off -= 4;
    }
    if unmapped.is_empty() {
        unmapped.push(window_size & !3);
    }

    let reads: Vec<u32> = unmapped
        .iter()
        .map(|&o| target.probe_read(o).unwrap_or(0))
        .collect();
    let read = mode(&reads);

    let mut write_roundtrips = false;
    for &o in &unmapped {
        let orig = target.probe_read(o).unwrap_or(read);
        let s = if read == SENTINEL { SENTINEL_ALT } else { SENTINEL };
        if target.probe_write(o, s) && target.probe_read(o) == Some(s) {
            write_roundtrips = true;
        }
        target.probe_write(o, orig);
    }

    Baseline { read, write_roundtrips }
}

fn mode(vals: &[u32]) -> u32 {
    let mut best = vals.first().copied().unwrap_or(0);
    let mut best_n = 0usize;
    for &v in vals {
        let n = vals.iter().filter(|&&x| x == v).count();
        if n > best_n {
            best_n = n;
            best = v;
        }
    }
    best
}

fn classify(target: &mut dyn ProbeTarget, reg: &ProbeReg, base: &Baseline) -> RegStatus {
    let r0 = target.probe_read(reg.offset).unwrap_or(base.read);
    let read_distinct = r0 != base.read;

    let sentinel = if base.read == SENTINEL { SENTINEL_ALT } else { SENTINEL };
    let wrote = target.probe_write(reg.offset, sentinel);
    let r1 = target.probe_read(reg.offset).unwrap_or(base.read);
    target.probe_write(reg.offset, r0); // restore
    let retains = wrote && r1 != r0 && r1 != base.read;

    if base.write_roundtrips {
        if read_distinct && reg.reset_value != 0 && r0 == reg.reset_value {
            RegStatus::Modelled
        } else {
            RegStatus::Indeterminate
        }
    } else if retains {
        RegStatus::Modelled
    } else if read_distinct {
        RegStatus::Modelled
    } else {
        match reg.access {
            Access::ReadWrite => RegStatus::Unmodelled,
            Access::WriteOnly | Access::ReadOnly => RegStatus::Indeterminate,
        }
    }
}

pub fn probe_peripheral(
    target: &mut dyn ProbeTarget,
    regs: &[ProbeReg],
    window_size: u64,
) -> Vec<RegResult> {
    let base = compute_baseline(target, regs, window_size);
    regs.iter()
        .map(|r| RegResult {
            name: r.name.clone(),
            offset: r.offset,
            status: classify(target, r, &base),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct RealModel {
        regs: HashMap<u64, u32>,
        modeled_offsets: std::collections::HashSet<u64>,
    }
    impl ProbeTarget for RealModel {
        fn probe_read(&self, offset: u64) -> Option<u32> {
            if self.modeled_offsets.contains(&offset) {
                Some(*self.regs.get(&offset).unwrap_or(&0))
            } else {
                Some(0)
            }
        }
        fn probe_write(&mut self, offset: u64, value: u32) -> bool {
            if self.modeled_offsets.contains(&offset) {
                self.regs.insert(offset, value);
            }
            true
        }
    }

    #[derive(Default)]
    struct StorageStub {
        mem: HashMap<u64, u32>,
    }
    impl ProbeTarget for StorageStub {
        fn probe_read(&self, offset: u64) -> Option<u32> {
            Some(*self.mem.get(&offset).unwrap_or(&0))
        }
        fn probe_write(&mut self, offset: u64, value: u32) -> bool {
            self.mem.insert(offset, value);
            true
        }
    }

    fn rw(name: &str, offset: u64) -> ProbeReg {
        ProbeReg { name: name.into(), offset, access: Access::ReadWrite, reset_value: 0 }
    }

    #[test]
    fn real_register_scores_modelled_stub_scores_unmodelled() {
        let mut m = RealModel::default();
        m.modeled_offsets.insert(0x00);
        let regs = vec![rw("CTRL", 0x00), rw("DATA", 0x04)];
        let out = probe_peripheral(&mut m, &regs, 0x100);
        assert_eq!(out[0].status, RegStatus::Modelled, "CTRL retains writes");
        assert_eq!(out[1].status, RegStatus::Unmodelled, "DATA is accept-and-ignore");
    }

    #[test]
    fn nonzero_reset_value_read_scores_modelled() {
        struct ResetModel;
        impl ProbeTarget for ResetModel {
            fn probe_read(&self, offset: u64) -> Option<u32> {
                if offset == 0x08 { Some(0x11) } else { Some(0) }
            }
            fn probe_write(&mut self, _o: u64, _v: u32) -> bool { true }
        }
        let regs = vec![ProbeReg {
            name: "SR".into(), offset: 0x08, access: Access::ReadOnly, reset_value: 0x11,
        }];
        let out = probe_peripheral(&mut ResetModel, &regs, 0x100);
        assert_eq!(out[0].status, RegStatus::Modelled);
    }

    #[test]
    fn storage_stub_is_indeterminate_not_modelled() {
        let mut s = StorageStub::default();
        let regs = vec![rw("CTRL", 0x00), rw("DATA", 0x04)];
        let out = probe_peripheral(&mut s, &regs, 0x100);
        assert!(out.iter().all(|r| r.status == RegStatus::Indeterminate),
            "generic storage must score Indeterminate, never Modelled");
    }

    #[test]
    fn readonly_zero_reset_reading_catchall_is_indeterminate() {
        let mut m = RealModel::default();
        let regs = vec![ProbeReg {
            name: "STATUS".into(), offset: 0x0C, access: Access::ReadOnly, reset_value: 0,
        }];
        let out = probe_peripheral(&mut m, &regs, 0x100);
        assert_eq!(out[0].status, RegStatus::Indeterminate);
    }
}
