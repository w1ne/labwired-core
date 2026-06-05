# ESP32-S3 SVD Coverage Tool (behavioral probe) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** An objective, un-gameable coverage tool that measures, per ESP32-S3 peripheral, how many of its SVD registers the model *actually* models — judged by observable behavior, not author declaration — and ratchets it so coverage can't silently regress.

**Architecture:** A pure behavioral **probe engine** (in `labwired-core`) classifies each register as `Modelled` / `Unmodelled` / `Indeterminate` by driving reads/writes through a `ProbeTarget` and comparing against the peripheral's own unmapped-offset (catch-all) baseline. A **driver** (in the CLI crate) parses the ESP32-S3 SVD (`svd-parser`, discovered from the toolchain), builds the wired bus in fast-boot mode, maps SVD peripherals → bus peripherals by base address, runs the probe, and emits a coverage matrix (text + JSON). A committed JSON snapshot + a gated ratchet test guard against regression.

**Tech Stack:** Rust; `labwired-core` (`Peripheral`/`Bus`), `labwired-config` (`Access`/`RegisterDescriptor`), `svd-parser` 0.14 (already a workspace dep of `svd-ingestor`), `serde_json`.

**Spec:** `docs/superpowers/specs/2026-06-05-esp32s3-full-chip-model-design.md` (slice S3). Coverage method = behavioral probe (user decision 2026-06-05): a register counts as modelled ONLY if it behaves distinctly from the unmapped catch-all; accept-and-ignore stubs correctly score `Unmodelled`.

---

## File Structure

- **Create** `crates/core/src/coverage/mod.rs` — module root; re-exports the probe engine.
- **Create** `crates/core/src/coverage/probe.rs` — pure probe engine + classifier + `ProbeTarget` trait + synthetic-peripheral unit tests. NO SVD, NO bus — depends only on the trait.
- **Modify** `crates/core/src/lib.rs` — add `pub mod coverage;`.
- **Create** `crates/cli/src/coverage.rs` — SVD discovery + parse, bus build, SVD→bus mapping, matrix build, text/JSON output, the `coverage` subcommand handler.
- **Modify** `crates/cli/src/main.rs` — register the `coverage` subcommand.
- **Modify** `crates/cli/Cargo.toml` — add `svd-parser`, `serde_json` (if not present).
- **Create** `docs/coverage/esp32s3-coverage.json` — committed snapshot (the ratchet baseline).
- **Create** `crates/cli/tests/svd_coverage_ratchet.rs` — gated regression test.

Probe engine (honest core, fully unit-tested) lives in `core`; SVD/bus/CLI plumbing lives in `cli`.

---

## Task 1: Probe engine + classifier (the honest core)

**Files:**
- Create: `crates/core/src/coverage/mod.rs`, `crates/core/src/coverage/probe.rs`
- Modify: `crates/core/src/lib.rs`

- [ ] **Step 1: Declare the module.** In `crates/core/src/lib.rs`, add (near the other `pub mod` lines): `pub mod coverage;`

- [ ] **Step 2: Create `crates/core/src/coverage/mod.rs`:**

```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! SVD-driven register-coverage probing for chip models.
//!
//! The probe engine measures, by OBSERVABLE BEHAVIOR alone, whether a model
//! actually implements each register a chip's SVD declares — so the coverage
//! number cannot be inflated by an author declaring a stub "modelled".

pub mod probe;

pub use probe::{probe_peripheral, Access, ProbeReg, ProbeTarget, RegResult, RegStatus};
```

- [ ] **Step 3: Write the probe engine with failing tests.** Create `crates/core/src/coverage/probe.rs`:

```rust
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
    /// Register-specific behavior observed (retains a write, or reads a value
    /// distinct from the unmapped catch-all).
    Modelled,
    /// A read-write register that behaves exactly like an unmapped offset
    /// (write ignored, read == catch-all) — an accept-and-ignore stub.
    Unmodelled,
    /// Not decidable by probing (write-only with no read-back, read-only reading
    /// the catch-all value, or a generic-storage peripheral).
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

/// Anything we can read/write u32s on at byte offsets. Errors map to `None`/false
/// (treated as catch-all). Implemented for a single `Peripheral` (offset-relative)
/// and for the wired `Bus` (absolute address = base + offset) by the driver.
pub trait ProbeTarget {
    /// Read a u32 at `offset`; `None` if the access errored.
    fn probe_read(&self, offset: u64) -> Option<u32>;
    /// Write a u32 at `offset`; `false` if the access errored.
    fn probe_write(&mut self, offset: u64, value: u32) -> bool;
}

const SENTINEL: u32 = 0xA5A5_A5A5;
const SENTINEL_ALT: u32 = 0x5A5A_5A5A;

struct Baseline {
    /// The most common read value at unmapped offsets (the catch-all read).
    read: u32,
    /// True if writes to unmapped offsets round-trip (generic-storage peripheral).
    write_roundtrips: bool,
}

/// Characterise the peripheral's catch-all by probing offsets NOT used by any
/// register, within `[0, window_size)`, word-aligned.
fn compute_baseline(target: &mut dyn ProbeTarget, regs: &[ProbeReg], window_size: u64) -> Baseline {
    let used: std::collections::HashSet<u64> = regs.iter().map(|r| r.offset & !3).collect();
    // Collect up to 4 unmapped, word-aligned probe offsets, preferring the high
    // end of the window (registers cluster low).
    let mut unmapped: Vec<u64> = Vec::new();
    let mut off = (window_size.saturating_sub(4)) & !3;
    while unmapped.len() < 4 && off >= 4 {
        if !used.contains(&off) {
            unmapped.push(off);
        }
        off -= 4;
    }
    if unmapped.is_empty() {
        // Degenerate (tiny window fully covered): use one offset just past it.
        unmapped.push(window_size & !3);
    }

    // Catch-all read = most common read across the unmapped offsets.
    let reads: Vec<u32> = unmapped
        .iter()
        .map(|&o| target.probe_read(o).unwrap_or(0))
        .collect();
    let read = mode(&reads);

    // Write-roundtrip test (restore afterwards).
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

    // Write-readback (save/restore to limit contamination).
    let sentinel = if base.read == SENTINEL { SENTINEL_ALT } else { SENTINEL };
    let wrote = target.probe_write(reg.offset, sentinel);
    let r1 = target.probe_read(reg.offset).unwrap_or(base.read);
    target.probe_write(reg.offset, r0); // restore
    let retains = wrote && r1 != r0 && r1 != base.read;

    if base.write_roundtrips {
        // Generic-storage peripheral: write-readback is meaningless. Only a read
        // that matches a real non-zero reset value proves register-specific modelling.
        if read_distinct && reg.reset_value != 0 && r0 == reg.reset_value {
            RegStatus::Modelled
        } else {
            RegStatus::Indeterminate
        }
    } else if retains {
        RegStatus::Modelled
    } else if read_distinct {
        // Distinct read behavior (e.g. a real reset value or a status pattern).
        RegStatus::Modelled
    } else {
        // Behaves exactly like an unmapped offset.
        match reg.access {
            Access::ReadWrite => RegStatus::Unmodelled,
            // No observable effect we can attribute; the FSM tests decide.
            Access::WriteOnly | Access::ReadOnly => RegStatus::Indeterminate,
        }
    }
}

/// Probe every register of one peripheral. `window_size` is the peripheral's
/// address-window size (used to find unmapped baseline offsets).
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

    /// A faithful register: stores writes at its offset, reads them back.
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
                Some(0) // catch-all reads 0, ignores writes
            }
        }
        fn probe_write(&mut self, offset: u64, value: u32) -> bool {
            if self.modeled_offsets.contains(&offset) {
                self.regs.insert(offset, value);
            }
            true
        }
    }

    /// Generic-storage stub: round-trips EVERY offset.
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
        m.modeled_offsets.insert(0x00); // only CTRL @ 0x00 is real
        let regs = vec![rw("CTRL", 0x00), rw("DATA", 0x04)];
        let out = probe_peripheral(&mut m, &regs, 0x100);
        assert_eq!(out[0].status, RegStatus::Modelled, "CTRL retains writes");
        assert_eq!(out[1].status, RegStatus::Unmodelled, "DATA is accept-and-ignore");
    }

    #[test]
    fn nonzero_reset_value_read_scores_modelled() {
        // A read-only status reg that returns its documented non-zero reset value
        // (distinct from the catch-all 0) is credited as Modelled.
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
        // A peripheral that round-trips EVERY offset must NOT be credited as
        // modelling its registers (the whole anti-gaming point).
        let mut s = StorageStub::default();
        let regs = vec![rw("CTRL", 0x00), rw("DATA", 0x04)];
        let out = probe_peripheral(&mut s, &regs, 0x100);
        assert!(out.iter().all(|r| r.status == RegStatus::Indeterminate),
            "generic storage must score Indeterminate, never Modelled");
    }

    #[test]
    fn readonly_zero_reset_reading_catchall_is_indeterminate() {
        let mut m = RealModel::default(); // nothing modeled → everything reads 0
        let regs = vec![ProbeReg {
            name: "STATUS".into(), offset: 0x0C, access: Access::ReadOnly, reset_value: 0,
        }];
        let out = probe_peripheral(&mut m, &regs, 0x100);
        assert_eq!(out[0].status, RegStatus::Indeterminate);
    }
}
```

- [ ] **Step 4: Run the tests.** Run: `cargo test -p labwired-core --lib coverage::probe -- --nocapture`. Expected: 4 tests PASS. (If a classification test fails, the bug is in `classify`/`compute_baseline` — fix the engine, not the test.)

- [ ] **Step 5: Commit.**
```bash
git add crates/core/src/lib.rs crates/core/src/coverage/mod.rs crates/core/src/coverage/probe.rs
git -c user.email="14119286+w1ne@users.noreply.github.com" -c user.name="w1ne" commit -m "feat(coverage): behavioral SVD register-coverage probe engine"
```
NO "Claude"/"AI"/"Co-Authored-By".

---

## Task 2: SVD load + driver + matrix (CLI crate)

**Files:**
- Create: `crates/cli/src/coverage.rs`
- Modify: `crates/cli/Cargo.toml`

- [ ] **Step 1: Add deps.** In `crates/cli/Cargo.toml` `[dependencies]`, ensure these exist (add if missing): `svd-parser = "0.14"`, `serde_json = "1"`. (`serde` is likely already present transitively; add `serde = { version = "1", features = ["derive"] }` if the file doesn't already pull it.)

- [ ] **Step 2: Write the driver.** Create `crates/cli/src/coverage.rs`:

```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 SVD register-coverage driver: parse the SVD, build the wired model,
//! probe every peripheral's registers, and emit a coverage matrix.

use std::collections::BTreeMap;
use std::path::PathBuf;

use labwired_core::coverage::{probe_peripheral, Access, ProbeReg, ProbeTarget, RegStatus};
use serde::{Deserialize, Serialize};

/// One SVD peripheral's register set.
struct SvdPeripheral {
    name: String,
    base: u64,
    registers: Vec<ProbeReg>,
}

/// Per-peripheral coverage counts (serialised into the snapshot).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeripheralCoverage {
    pub modelled: usize,
    pub indeterminate: usize,
    pub unmodelled: usize,
    pub total: usize,
    /// Names of the unmodelled registers — the work queue.
    pub unmodelled_regs: Vec<String>,
}

/// The full matrix: peripheral name → coverage. BTreeMap for stable ordering.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoverageMatrix(pub BTreeMap<String, PeripheralCoverage>);

/// Discover the ESP32-S3 SVD: `LABWIRED_ESP32S3_SVD` env override, else the
/// PlatformIO espressif32 platform path.
pub fn discover_svd() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("LABWIRED_ESP32S3_SVD") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let home = std::env::var("HOME").ok()?;
    let pio = PathBuf::from(format!(
        "{home}/.platformio/platforms/espressif32/misc/svd/esp32s3.svd"
    ));
    pio.is_file().then_some(pio)
}

/// Parse the SVD into peripherals with register sets.
fn load_svd(path: &std::path::Path) -> anyhow::Result<Vec<SvdPeripheral>> {
    let xml = std::fs::read_to_string(path)?;
    let device = svd_parser::parse(&xml)?;
    let mut out = Vec::new();
    for p in &device.peripherals {
        let base = p.base_address;
        let mut registers = Vec::new();
        for r in p.registers() {
            let access = match r.properties.access {
                Some(svd_parser::svd::Access::ReadOnly) => Access::ReadOnly,
                Some(svd_parser::svd::Access::WriteOnly)
                | Some(svd_parser::svd::Access::WriteOnce) => Access::WriteOnly,
                _ => Access::ReadWrite,
            };
            registers.push(ProbeReg {
                name: r.name.clone(),
                offset: r.address_offset as u64,
                access,
                reset_value: r.properties.reset_value.unwrap_or(0) as u32,
            });
        }
        if !registers.is_empty() {
            out.push(SvdPeripheral { name: p.name.clone(), base, registers });
        }
    }
    Ok(out)
}

/// A `ProbeTarget` view over the wired bus at a fixed peripheral base.
struct BusTarget<'a> {
    bus: &'a mut labwired_core::bus::SystemBus,
    base: u64,
}
impl ProbeTarget for BusTarget<'_> {
    fn probe_read(&self, offset: u64) -> Option<u32> {
        self.bus.read_u32(self.base + offset).ok()
    }
    fn probe_write(&mut self, offset: u64, value: u32) -> bool {
        self.bus.write_u32(self.base + offset, value).is_ok()
    }
}

/// Build the wired ESP32-S3 bus in fast-boot mode (deterministic, no ROM blob),
/// probe every SVD peripheral that maps onto a bus peripheral by base address.
pub fn build_matrix(svd: &[SvdPeripheral]) -> CoverageMatrix {
    let mut matrix = BTreeMap::new();
    for sp in svd {
        // Fresh bus per peripheral so probing one cannot contaminate another.
        std::env::set_var("LABWIRED_ESP32S3_FASTBOOT", "1");
        let mut bus = labwired_core::bus::SystemBus::new();
        let _ = labwired_core::system::xtensa::configure_xtensa_esp32s3(
            &mut bus,
            &labwired_core::system::xtensa::Esp32s3Opts::default(),
        );
        std::env::remove_var("LABWIRED_ESP32S3_FASTBOOT");

        // Find the bus peripheral whose window contains this SVD base.
        let window = bus
            .peripherals
            .iter()
            .find(|e| sp.base >= e.base && sp.base < e.base + e.size)
            .map(|e| e.size);
        let Some(window_size) = window else {
            continue; // SVD peripheral not wired in the model — skip (not counted)
        };

        let mut target = BusTarget { bus: &mut bus, base: sp.base };
        let results = probe_peripheral(&mut target, &sp.registers, window_size);

        let mut cov = PeripheralCoverage {
            modelled: 0,
            indeterminate: 0,
            unmodelled: 0,
            total: results.len(),
            unmodelled_regs: Vec::new(),
        };
        for r in &results {
            match r.status {
                RegStatus::Modelled => cov.modelled += 1,
                RegStatus::Indeterminate => cov.indeterminate += 1,
                RegStatus::Unmodelled => {
                    cov.unmodelled += 1;
                    cov.unmodelled_regs.push(r.name.clone());
                }
            }
        }
        matrix.insert(sp.name.clone(), cov);
    }
    CoverageMatrix(matrix)
}

/// Human-readable table.
pub fn render_text(m: &CoverageMatrix) -> String {
    let mut s = String::new();
    s.push_str("ESP32-S3 register coverage (behavioral probe)\n");
    s.push_str("peripheral            modelled  indet  unmod  total\n");
    let (mut tm, mut ti, mut tu, mut tt) = (0, 0, 0, 0);
    for (name, c) in &m.0 {
        s.push_str(&format!(
            "{name:<20}  {:>7}  {:>5}  {:>5}  {:>5}\n",
            c.modelled, c.indeterminate, c.unmodelled, c.total
        ));
        tm += c.modelled; ti += c.indeterminate; tu += c.unmodelled; tt += c.total;
    }
    s.push_str(&format!(
        "{:<20}  {tm:>7}  {ti:>5}  {tu:>5}  {tt:>5}\n", "TOTAL"
    ));
    s
}

/// Run the full analysis. Returns None if the SVD can't be found.
pub fn run() -> Option<(CoverageMatrix, String)> {
    let svd_path = discover_svd()?;
    let svd = load_svd(&svd_path).ok()?;
    let matrix = build_matrix(&svd);
    let text = render_text(&matrix);
    Some((matrix, text))
}
```

- [ ] **Step 3: Verify it compiles.** Run: `cargo build -p labwired-cli 2>&1 | grep -iE 'error|warning' || echo CLEAN`. Fix any API mismatches (e.g. `svd_parser::parse` signature, `p.registers()` iterator, `bus::SystemBus` path, `configure_xtensa_esp32s3` import path) to match the real crates — adjust MINIMALLY, keep the algorithm. The `coverage` module must be declared: add `mod coverage;` to `crates/cli/src/main.rs`.

- [ ] **Step 4: Commit.**
```bash
git add crates/cli/src/coverage.rs crates/cli/src/main.rs crates/cli/Cargo.toml
git -c user.email="14119286+w1ne@users.noreply.github.com" -c user.name="w1ne" commit -m "feat(coverage): ESP32-S3 SVD driver — parse, probe wired bus, build matrix"
```

---

## Task 3: `coverage` CLI subcommand + initial snapshot

**Files:**
- Modify: `crates/cli/src/main.rs`
- Create: `docs/coverage/esp32s3-coverage.json`

- [ ] **Step 1: Add the subcommand.** In `crates/cli/src/main.rs`, find the `clap` subcommand enum (e.g. `enum Commands { Run(...), ... }`) and add a `Coverage` variant with an optional `--json <path>` to write the snapshot and a `--svd <path>` override. Wire it to a handler that calls `coverage::run()`. The handler:
```rust
// in the match over subcommands:
Commands::Coverage(args) => {
    if let Some(p) = &args.svd {
        std::env::set_var("LABWIRED_ESP32S3_SVD", p);
    }
    match coverage::run() {
        Some((matrix, text)) => {
            print!("{text}");
            if let Some(out) = &args.json {
                let json = serde_json::to_string_pretty(&matrix).expect("serialize");
                std::fs::write(out, json).expect("write json");
                eprintln!("wrote {}", out.display());
            }
            ExitCode::SUCCESS
        }
        None => {
            eprintln!("error: ESP32-S3 SVD not found; set LABWIRED_ESP32S3_SVD or install the espressif32 PlatformIO platform");
            ExitCode::from(EXIT_CONFIG_ERROR)
        }
    }
}
```
Define the args struct following the existing subcommand-arg pattern in the file (clap `Args`), with `svd: Option<PathBuf>` and `json: Option<PathBuf>`.

- [ ] **Step 2: Generate + commit the snapshot.** Run:
```bash
mkdir -p docs/coverage
cargo run --release -p labwired-cli -- coverage --json docs/coverage/esp32s3-coverage.json
```
Inspect the printed table — it should list real ESP32-S3 peripherals (I2C0, UART0, GPIO, SYSTIMER, TIMG0, SPI2, LEDC, …) with plausible numbers (e.g. I2C0 should have several `modelled`). If a peripheral that is clearly wired shows 0 modelled and 0 indeterminate (all unmodelled), sanity-check the base-address mapping before trusting it; note anything surprising in the report.

- [ ] **Step 3: Commit.**
```bash
git add crates/cli/src/main.rs docs/coverage/esp32s3-coverage.json
git -c user.email="14119286+w1ne@users.noreply.github.com" -c user.name="w1ne" commit -m "feat(coverage): coverage subcommand + initial ESP32-S3 coverage snapshot"
```

---

## Task 4: Ratchet regression test

**Files:**
- Create: `crates/cli/tests/svd_coverage_ratchet.rs`

- [ ] **Step 1: Write the gated ratchet test.** Create `crates/cli/tests/svd_coverage_ratchet.rs`:

```rust
// Regression ratchet: per-peripheral `modelled` coverage must never DROP below
// the committed snapshot. Gated on the ESP32-S3 SVD being discoverable (like the
// firmware e2e tests) — skips cleanly in environments without the toolchain.

use std::collections::BTreeMap;

#[test]
fn esp32s3_coverage_does_not_regress() {
    if labwired_cli::coverage::discover_svd().is_none() {
        eprintln!("SKIP: ESP32-S3 SVD not found (set LABWIRED_ESP32S3_SVD)");
        return;
    }
    let (live, _text) = labwired_cli::coverage::run().expect("coverage run");

    let snapshot_json = include_str!("../../docs/coverage/esp32s3-coverage.json");
    let snapshot: labwired_cli::coverage::CoverageMatrix =
        serde_json::from_str(snapshot_json).expect("parse snapshot");

    let mut regressions: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for (name, snap) in &snapshot.0 {
        let cur = live.0.get(name);
        let cur_modelled = cur.map(|c| c.modelled).unwrap_or(0);
        if cur_modelled < snap.modelled {
            regressions.insert(name.clone(), (snap.modelled, cur_modelled));
        }
    }
    assert!(
        regressions.is_empty(),
        "register coverage regressed (snapshot modelled -> current): {regressions:?}. \
         If intentional, regenerate: cargo run -p labwired-cli -- coverage --json docs/coverage/esp32s3-coverage.json"
    );
}
```

- [ ] **Step 2: Make the CLI lib-accessible.** The test uses `labwired_cli::coverage`. If `crates/cli` is a binary-only crate (no `lib.rs`), add a minimal `crates/cli/src/lib.rs` that exposes `pub mod coverage;` (and whatever else it needs), and have `main.rs` use it — OR, simpler, if the crate already has a lib target, just ensure `pub mod coverage;` is exported. Check `crates/cli/Cargo.toml` for a `[lib]`/`[[bin]]` setup and follow the existing structure. If adding a lib target is disproportionate, instead move `coverage.rs` into `labwired-core` (as `pub mod coverage_esp32s3`) — but prefer keeping the SVD/bus driver in cli; only the pure probe engine belongs in core. Pick the lower-churn option that compiles and report which you chose.

- [ ] **Step 3: Run.** Run: `cargo test -p labwired-cli --test svd_coverage_ratchet -- --nocapture`. Expected: PASS (or SKIP if no SVD — but on this machine the SVD is present, so it must PASS against the just-committed snapshot).

- [ ] **Step 4: Commit.**
```bash
git add crates/cli/tests/svd_coverage_ratchet.rs crates/cli/src/lib.rs crates/cli/Cargo.toml
git -c user.email="14119286+w1ne@users.noreply.github.com" -c user.name="w1ne" commit -m "test(coverage): ratchet ESP32-S3 register coverage against committed snapshot"
```

---

## Task 5: Regression sweep + docs

- [ ] **Step 1: Full workspace test.** Run: `cargo test --release --workspace 2>&1 | grep -E 'test result:|FAILED' | grep -v '0 failed'`. Expected: all suites `ok`, zero `FAILED`.

- [ ] **Step 2: Document the tool.** Append a short "Register coverage" section to `docs/superpowers/specs/2026-06-05-esp32s3-full-chip-model-design.md` §4 noting the tool shipped: how to run (`cargo run -p labwired-cli -- coverage`), what `Modelled`/`Indeterminate`/`Unmodelled` mean, that the probe is behavioral (un-gameable), and that `Indeterminate` registers are resolved by the per-peripheral FSM tests (S4+). Commit:
```bash
git add docs/superpowers/specs/2026-06-05-esp32s3-full-chip-model-design.md
git -c user.email="14119286+w1ne@users.noreply.github.com" -c user.name="w1ne" commit -m "docs(coverage): document the behavioral SVD coverage tool"
```

---

## Self-Review notes

- **Spec coverage:** §3 oracle = SVD load (Task 2). §4 coverage tool + matrix + ratchet = Tasks 1–4. Behavioral-probe method (user decision) = Task 1 engine. CLI surface = Task 3.
- **Type consistency:** `RegStatus::{Modelled,Unmodelled,Indeterminate}`, `Access::{ReadWrite,ReadOnly,WriteOnly}`, `ProbeReg{name,offset,access,reset_value}`, `ProbeTarget::{probe_read,probe_write}`, `CoverageMatrix(BTreeMap<String,PeripheralCoverage>)`, `PeripheralCoverage{modelled,indeterminate,unmodelled,total,unmodelled_regs}` are used consistently across tasks.
- **Honesty guards:** the storage-stub test (Task 1, Step 3) is the key anti-gaming assertion — generic round-trip storage must score `Indeterminate`, never `Modelled`.
- **Risk:** `svd-parser` 0.14 API surface (`parse`, `device.peripherals`, `p.registers()`, `r.properties.{access,reset_value}`, `p.base_address`) — Task 2 Step 3 instructs minimal adjustment to the real API. The `derive_from`/cluster handling: `p.registers()` flattens registers; clusters of repeated registers may need `reg_iter()` — if the parser version exposes a different iterator, use it (the existing `crates/svd-ingestor/src/lib.rs` shows the working pattern — read it for reference).
