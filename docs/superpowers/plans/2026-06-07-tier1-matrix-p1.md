# Tier-1 Chip Matrix — P1 (Wedge Slice) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Per-chip × per-peripheral validation matrix with a ratchet gate, CLI exporter, scoreboard, and the ESP32-S3 beachhead fixture (six rubric classes + `mcpwm`/`i2c`/`rmt`), riding the vendored-ROM faithful boot.

**Architecture:** A `tier1` module in `labwired-cli` owns the matrix types, the `TIER1 <class> PASS|FAIL` UART-line parser, chip-YAML `n/a` derivation, and a runner that shells the `labwired` binary (`run --rom-boot`) against committed content-hashed fixture blobs. Integration tests in `crates/cli/tests/` provide the per-PR harness + ratchet (skip cleanly when fixtures are absent, like `svd_coverage_ratchet`). A Python script renders the committed JSON snapshot into a public scoreboard grid. Part B builds and commits the ESP32-S3 fixture firmware (bare-metal Rust, esp-hal) and the initial snapshot.

**Tech Stack:** Rust (labwired-cli/clap/serde), esp-hal 1.1 `no_std` Xtensa firmware, espflash for flash images, Python 3 for the scoreboard.

**Spec:** `docs/superpowers/specs/2026-06-07-tier1-chip-matrix-design.md`. All work happens in the `core` submodule (`~/projects/labwired/core`) on a feature branch off `main`, except this plan file. Commit messages: plain conventional commits, no AI references, author email `14119286+w1ne@users.noreply.github.com`.

---

## File Structure

```
core/
  crates/cli/src/tier1.rs              # NEW: types, parser, n/a derivation, runner, ratchet compare
  crates/cli/src/main.rs               # MODIFY: `tier1-matrix` subcommand
  crates/cli/tests/tier1_matrix.rs     # NEW: per-PR harness (skips w/o fixtures)
  crates/cli/tests/tier1_matrix_ratchet.rs  # NEW: ratchet gate
  docs/coverage/tier1-matrix.json      # NEW: committed snapshot (Part B)
  examples/tier1-fixture/esp32s3/      # NEW: fixture firmware source
  tests/fixtures/tier1/                # NEW: committed ELF + flash bin + MANIFEST.json (Part B)
  scripts/build_tier1_fixtures.sh      # NEW: rebuild + hash blobs (toolchain machines only)
  scripts/generate_tier1_scoreboard.py # NEW: JSON → markdown grid
  docs/coverage_scoreboard.md          # MODIFY: link the tier1 grid
  .github/workflows/core-nightly.yml   # MODIFY: weekly fixture drift check
```

---

# Part A — Infrastructure (no Xtensa toolchain needed)

### Task 1: Matrix types + UART-line parser

**Files:**
- Create: `core/crates/cli/src/tier1.rs`
- Modify: `core/crates/cli/src/lib.rs` (add `pub mod tier1;` next to `pub mod coverage;`)

- [ ] **Step 1: Write the failing tests** — append at the bottom of the new `tier1.rs` (module skeleton + tests first):

```rust
// crates/cli/src/tier1.rs
//! Tier-1 chip × peripheral validation matrix (spec:
//! labwired docs/superpowers/specs/2026-06-07-tier1-chip-matrix-design.md).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One cell's status. `Na` = chip YAML declares no peripheral of this class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CellStatus {
    Pass,
    Partial,
    Blocked,
    Na,
    Unrecorded,
}

/// A cell with its evidence link (CI run that produced it; None until CI stamps it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cell {
    pub status: CellStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_url: Option<String>,
}

/// chip → class → cell. BTreeMaps keep JSON output deterministic (sorted keys).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tier1Matrix(pub BTreeMap<String, BTreeMap<String, Cell>>);

/// The six rubric classes every chip reports.
pub const RUBRIC_CLASSES: &[&str] = &["clock", "gpio", "uart", "timer", "dma", "irq"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pass_fail_lines_and_done() {
        let uart = b"boot noise\nTIER1 clock PASS\nTIER1 gpio PASS\nTIER1 dma FAIL code=gdma-idle\nTIER1 done\ntrailing";
        let parsed = parse_tier1_uart(uart);
        assert!(parsed.done);
        assert_eq!(parsed.classes["clock"], CellStatus::Pass);
        assert_eq!(parsed.classes["gpio"], CellStatus::Pass);
        assert_eq!(parsed.classes["dma"], CellStatus::Blocked);
    }

    #[test]
    fn missing_done_marks_row_partial_for_reported_passes() {
        let uart = b"TIER1 clock PASS\nTIER1 gpio PASS\n"; // hung before done
        let parsed = parse_tier1_uart(uart);
        assert!(!parsed.done);
        let row = parsed.into_row(&["clock", "gpio", "uart"]);
        // reported passes degrade to partial; unreported classes are blocked
        assert_eq!(row["clock"].status, CellStatus::Partial);
        assert_eq!(row["gpio"].status, CellStatus::Partial);
        assert_eq!(row["uart"].status, CellStatus::Blocked);
    }

    #[test]
    fn no_tier1_lines_blocks_uart_and_everything_else() {
        let parsed = parse_tier1_uart(b"garbage \xff\xfe binary noise");
        assert!(!parsed.done);
        assert!(parsed.classes.is_empty());
        let row = parsed.into_row(RUBRIC_CLASSES);
        for class in RUBRIC_CLASSES {
            assert_eq!(row[*class].status, CellStatus::Blocked, "{class}");
        }
    }

    #[test]
    fn garbage_tier1_lines_are_ignored_not_fatal() {
        let uart = b"TIER1 gpio MAYBE\nTIER1\nTIER1 gpio PASS\nTIER1 done\n";
        let parsed = parse_tier1_uart(uart);
        assert_eq!(parsed.classes["gpio"], CellStatus::Pass);
        assert_eq!(parsed.classes.len(), 1);
    }

    #[test]
    fn uart_class_is_implicitly_pass_when_done_arrives() {
        // The fixture never prints "TIER1 uart ..." — receiving the protocol IS the proof.
        let parsed = parse_tier1_uart(b"TIER1 clock PASS\nTIER1 done\n");
        let row = parsed.into_row(&["clock", "uart"]);
        assert_eq!(row["uart"].status, CellStatus::Pass);
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd ~/projects/labwired/core && cargo test -p labwired-cli --lib tier1 2>&1 | tail -5`
Expected: compile error — `parse_tier1_uart` not found.

- [ ] **Step 3: Implement parser** — insert above the tests:

```rust
/// Parsed `TIER1` protocol from a UART capture.
#[derive(Debug, Default)]
pub struct ParsedTier1 {
    /// class → status from explicit `TIER1 <class> PASS|FAIL` lines.
    pub classes: BTreeMap<String, CellStatus>,
    /// `TIER1 done` seen — the fixture completed its sequence.
    pub done: bool,
}

/// Parse `TIER1 <class> PASS|FAIL[ code=..]` lines + `TIER1 done` out of a raw
/// UART byte capture. Non-UTF8 and unrelated lines are skipped; malformed
/// `TIER1` lines are ignored (never fatal — boot noise is expected).
pub fn parse_tier1_uart(uart: &[u8]) -> ParsedTier1 {
    let mut out = ParsedTier1::default();
    for line in String::from_utf8_lossy(uart).lines() {
        let mut it = line.trim().split_whitespace();
        if it.next() != Some("TIER1") {
            continue;
        }
        match (it.next(), it.next()) {
            (Some("done"), _) => out.done = true,
            (Some(class), Some("PASS")) => {
                out.classes.insert(class.to_string(), CellStatus::Pass);
            }
            (Some(class), Some("FAIL")) => {
                out.classes.insert(class.to_string(), CellStatus::Blocked);
            }
            _ => {} // malformed TIER1 line — ignore
        }
    }
    out
}

impl ParsedTier1 {
    /// Resolve a full row over `classes`. Rules (spec §2 conventions):
    /// - `uart` is implicitly Pass once any protocol arrived AND done was seen
    ///   (receiving the lines is the proof), Blocked otherwise.
    /// - missing `done` degrades every reported Pass to Partial (hung mid-sequence);
    /// - classes never reported are Blocked.
    pub fn into_row(&self, classes: &[&str]) -> BTreeMap<String, Cell> {
        let mut row = BTreeMap::new();
        for &class in classes {
            let status = if class == "uart" {
                if self.done && !self.classes.is_empty() {
                    CellStatus::Pass
                } else {
                    CellStatus::Blocked
                }
            } else {
                match self.classes.get(class) {
                    Some(CellStatus::Pass) if !self.done => CellStatus::Partial,
                    Some(s) => *s,
                    None => CellStatus::Blocked,
                }
            };
            row.insert(class.to_string(), Cell { status, run_url: None });
        }
        row
    }
}
```

And in `crates/cli/src/lib.rs`, next to the existing `pub mod coverage;`:

```rust
pub mod tier1;
```

- [ ] **Step 4: Run tests to verify pass**

Run: `cargo test -p labwired-cli --lib tier1 2>&1 | tail -3`
Expected: `test result: ok. 5 passed`

- [ ] **Step 5: Commit**

```bash
git add crates/cli/src/tier1.rs crates/cli/src/lib.rs
git commit -m "feat(tier1): matrix types + TIER1 UART protocol parser"
```

### Task 2: `n/a` derivation from chip YAML

**Files:**
- Modify: `core/crates/cli/src/tier1.rs`

- [ ] **Step 1: Write failing tests** (append to the `tests` module):

```rust
    #[test]
    fn derives_na_from_chip_yaml_peripheral_ids() {
        // Minimal chip yaml shape — only `peripherals[].id` matters here.
        let yaml = r#"
name: "fakechip"
arch: "xtensa"
peripherals:
  - { id: "uart0", type: "uart", base_address: 0x60000000 }
  - { id: "gpio", type: "declarative", base_address: 0x60004000 }
  - { id: "timg0", type: "declarative", base_address: 0x6001F000 }
  - { id: "interrupt_core0", type: "declarative", base_address: 0x600C2000 }
"#;
        let declared = declared_classes_from_yaml(yaml).unwrap();
        assert!(declared.contains("uart"));
        assert!(declared.contains("gpio"));
        assert!(declared.contains("timer"));
        assert!(declared.contains("irq"));
        assert!(!declared.contains("dma")); // not declared → n/a, not blocked
        assert!(!declared.contains("mcpwm"));
    }

    #[test]
    fn na_overrides_blocked_in_row_resolution() {
        let parsed = parse_tier1_uart(b"TIER1 clock PASS\nTIER1 done\n");
        let mut row = parsed.into_row(RUBRIC_CLASSES);
        let declared: std::collections::BTreeSet<String> =
            ["clock", "uart"].iter().map(|s| s.to_string()).collect();
        apply_na(&mut row, &declared);
        assert_eq!(row["clock"].status, CellStatus::Pass);
        assert_eq!(row["dma"].status, CellStatus::Na); // undeclared
        assert_eq!(row["gpio"].status, CellStatus::Na); // undeclared
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p labwired-cli --lib tier1 2>&1 | tail -3`
Expected: compile error — `declared_classes_from_yaml` / `apply_na` not found.

- [ ] **Step 3: Implement** (insert above the tests; `serde_yaml` is already a labwired-cli dependency — verify with `grep serde_yaml crates/cli/Cargo.toml`, and if absent use `serde_yml` matching whatever `crates/config` uses):

```rust
/// peripheral-id substring → tier1 class. First match wins; order matters
/// (e.g. "gdma" must map to dma before "dma" generic).
const CLASS_MARKERS: &[(&str, &str)] = &[
    ("uart", "uart"),
    ("usb_serial", "uart"), // S3 console can be USB-Serial-JTAG
    ("gpio", "gpio"),
    ("timg", "timer"),
    ("systimer", "timer"),
    ("tim", "timer"),
    ("gdma", "dma"),
    ("dma", "dma"),
    ("intmatrix", "irq"),
    ("interrupt", "irq"),
    ("nvic", "irq"),
    ("rcc", "clock"),
    ("clk", "clock"),
    ("rtc_cntl", "clock"),
    ("system", "clock"),
    ("mcpwm", "mcpwm"),
    ("i2c", "i2c"),
    ("rmt", "rmt"),
];

#[derive(Deserialize)]
struct ChipYamlPeripheral {
    id: String,
}

#[derive(Deserialize)]
struct ChipYamlDoc {
    #[serde(default)]
    peripherals: Vec<ChipYamlPeripheral>,
}

/// Which tier1 classes a chip YAML declares, by peripheral-id heuristics.
pub fn declared_classes_from_yaml(yaml: &str) -> Result<std::collections::BTreeSet<String>, String> {
    let doc: ChipYamlDoc = serde_yaml::from_str(yaml).map_err(|e| e.to_string())?;
    let mut classes = std::collections::BTreeSet::new();
    for p in &doc.peripherals {
        let id = p.id.to_lowercase();
        for (marker, class) in CLASS_MARKERS {
            if id.contains(marker) {
                classes.insert(class.to_string());
                break;
            }
        }
    }
    Ok(classes)
}

/// Cells whose class is not declared by the chip become `Na`.
pub fn apply_na(row: &mut BTreeMap<String, Cell>, declared: &std::collections::BTreeSet<String>) {
    for (class, cell) in row.iter_mut() {
        if !declared.contains(class) {
            cell.status = CellStatus::Na;
            cell.run_url = None;
        }
    }
}
```

- [ ] **Step 4: Run tests** — `cargo test -p labwired-cli --lib tier1 2>&1 | tail -3` → `7 passed`.

- [ ] **Step 5: Commit** — `git add -u && git commit -m "feat(tier1): derive n/a cells from chip YAML peripheral declarations"`

### Task 3: Ratchet comparison

**Files:**
- Modify: `core/crates/cli/src/tier1.rs`

- [ ] **Step 1: Failing tests:**

```rust
    fn cell(s: CellStatus) -> Cell { Cell { status: s, run_url: None } }

    #[test]
    fn ratchet_flags_pass_regressions_only() {
        let mut snap = Tier1Matrix::default();
        snap.0.entry("esp32s3".into()).or_default().insert("gpio".into(), cell(CellStatus::Pass));
        snap.0.entry("esp32s3".into()).or_default().insert("dma".into(), cell(CellStatus::Blocked));

        let mut live = Tier1Matrix::default();
        live.0.entry("esp32s3".into()).or_default().insert("gpio".into(), cell(CellStatus::Blocked)); // regression!
        live.0.entry("esp32s3".into()).or_default().insert("dma".into(), cell(CellStatus::Pass)); // improvement — fine

        let regressions = ratchet_regressions(&snap, &live);
        assert_eq!(regressions, vec!["esp32s3/gpio: pass -> blocked".to_string()]);
    }

    #[test]
    fn ratchet_ignores_unrecorded_and_missing_chips() {
        let mut snap = Tier1Matrix::default();
        snap.0.entry("esp32s3".into()).or_default().insert("gpio".into(), cell(CellStatus::Unrecorded));
        let live = Tier1Matrix::default(); // chip absent from live run
        assert!(ratchet_regressions(&snap, &live).is_empty());
    }

    #[test]
    fn snapshot_roundtrip_is_deterministic() {
        let mut m = Tier1Matrix::default();
        m.0.entry("b".into()).or_default().insert("z".into(), cell(CellStatus::Pass));
        m.0.entry("a".into()).or_default().insert("gpio".into(), cell(CellStatus::Na));
        let j1 = serde_json::to_string_pretty(&m).unwrap();
        let j2 = serde_json::to_string_pretty(&serde_json::from_str::<Tier1Matrix>(&j1).unwrap()).unwrap();
        assert_eq!(j1, j2);
    }
```

- [ ] **Step 2: Verify failure** — `cargo test -p labwired-cli --lib tier1 2>&1 | tail -3` → `ratchet_regressions` not found.

- [ ] **Step 3: Implement:**

```rust
/// Cells recorded `Pass` in the snapshot must still pass live. Everything else
/// (partial/blocked/na/unrecorded, chips missing from the live run) moves freely.
pub fn ratchet_regressions(snapshot: &Tier1Matrix, live: &Tier1Matrix) -> Vec<String> {
    let mut out = Vec::new();
    for (chip, row) in &snapshot.0 {
        for (class, snap_cell) in row {
            if snap_cell.status != CellStatus::Pass {
                continue;
            }
            let live_status = live
                .0
                .get(chip)
                .and_then(|r| r.get(class))
                .map(|c| c.status);
            match live_status {
                Some(CellStatus::Pass) | None => {} // None = chip not exercised in this run
                Some(s) => out.push(format!(
                    "{chip}/{class}: pass -> {}",
                    serde_json::to_string(&s).unwrap().trim_matches('"')
                )),
            }
        }
    }
    out
}
```

- [ ] **Step 4: Run** — `cargo test -p labwired-cli --lib tier1` → `10 passed`.

- [ ] **Step 5: Commit** — `git add -u && git commit -m "feat(tier1): ratchet comparison — recorded pass can never silently regress"`

### Task 4: Fixture manifest + runner (shells the `labwired` binary)

**Files:**
- Modify: `core/crates/cli/src/tier1.rs`

The runner executes `labwired run --chip <chip.yaml> --firmware <elf> --rom-boot --max-steps N` with `LABWIRED_ESP32S3_FLASH=<flash.bin>`, captures stdout (UART echoes there — see `main.rs` run path, `attach_uart_tx_sink(.., !no_uart_stdout)`), and parses the `TIER1` protocol. Targets are declared in a table; fixtures resolve under `<workspace>/tests/fixtures/tier1/`.

- [ ] **Step 1: Failing tests** (pure logic only — binary execution is covered by the integration test in Task 6):

```rust
    #[test]
    fn target_table_paths_resolve_relative_to_workspace_root() {
        let t = &TIER1_TARGETS[0];
        assert_eq!(t.chip, "esp32s3");
        assert!(t.chip_yaml.ends_with("configs/chips/esp32s3.yaml"));
        assert!(t.elf.ends_with("tests/fixtures/tier1/esp32s3.elf"));
        assert!(t.flash_bin.as_deref().unwrap().ends_with("tests/fixtures/tier1/esp32s3-flash.bin"));
    }

    #[test]
    fn manifest_verification_rejects_corrupt_blob() {
        let dir = std::env::temp_dir().join("tier1-manifest-test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("esp32s3.elf"), b"not the real elf").unwrap();
        let manifest = r#"{ "esp32s3.elf": { "sha256": "0000000000000000000000000000000000000000000000000000000000000000" } }"#;
        std::fs::write(dir.join("MANIFEST.json"), manifest).unwrap();
        let err = verify_fixture_manifest(&dir).unwrap_err();
        assert!(err.contains("esp32s3.elf"), "{err}");
    }
```

- [ ] **Step 2: Verify failure** — compile error: `TIER1_TARGETS` / `verify_fixture_manifest` not found.

- [ ] **Step 3: Implement** (labwired-cli already depends on `sha2` — verify via `grep sha2 crates/cli/Cargo.toml`; if absent add `sha2 = "0.10"` to `[dependencies]`):

```rust
use std::path::{Path, PathBuf};

/// One matrix target. Paths are workspace-root-relative; `resolve` makes them absolute.
pub struct Tier1Target {
    pub chip: &'static str,
    pub chip_yaml: &'static str,
    pub elf: &'static str,
    /// Flash image for `--rom-boot` (None = fast-boot ELF entry).
    pub flash_bin: Option<&'static str>,
    pub rom_boot: bool,
    pub max_steps: u64,
    /// Beachhead classes beyond RUBRIC_CLASSES (spec wedge-alignment §4).
    pub extra_classes: &'static [&'static str],
}

pub const TIER1_TARGETS: &[Tier1Target] = &[
    Tier1Target {
        chip: "esp32s3",
        chip_yaml: "configs/chips/esp32s3.yaml",
        elf: "tests/fixtures/tier1/esp32s3.elf",
        flash_bin: Some("tests/fixtures/tier1/esp32s3-flash.bin"),
        rom_boot: true,
        max_steps: 40_000_000, // real ROM + bootloader + app + self-tests
        extra_classes: &["mcpwm", "i2c", "rmt"],
    },
    Tier1Target {
        chip: "esp32s3-zero",
        chip_yaml: "configs/chips/esp32s3-zero.yaml",
        elf: "tests/fixtures/tier1/esp32s3.elf", // same silicon, same fixture
        flash_bin: Some("tests/fixtures/tier1/esp32s3-flash.bin"),
        rom_boot: true,
        max_steps: 40_000_000,
        extra_classes: &["mcpwm", "i2c", "rmt"],
    },
];

/// Workspace root = two parents up from the cli crate (crates/cli → core).
pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

#[derive(Deserialize)]
struct ManifestEntry {
    sha256: String,
}

/// Verify every blob listed in `<dir>/MANIFEST.json` against its sha256.
/// Returns Err naming the first mismatching file.
pub fn verify_fixture_manifest(dir: &Path) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    let manifest_path = dir.join("MANIFEST.json");
    let manifest: BTreeMap<String, ManifestEntry> = serde_json::from_str(
        &std::fs::read_to_string(&manifest_path).map_err(|e| format!("{}: {e}", manifest_path.display()))?,
    )
    .map_err(|e| e.to_string())?;
    for (file, entry) in &manifest {
        let bytes = std::fs::read(dir.join(file)).map_err(|e| format!("{file}: {e}"))?;
        let got = hex::encode(Sha256::digest(&bytes));
        if got != entry.sha256 {
            return Err(format!("{file}: sha256 mismatch (manifest {}, got {got})", entry.sha256));
        }
    }
    Ok(())
}

/// Run one target through the `labwired` binary and parse its TIER1 row.
/// `labwired_bin` lets integration tests pass `env!("CARGO_BIN_EXE_labwired")`.
pub fn run_target(target: &Tier1Target, labwired_bin: &Path) -> Result<BTreeMap<String, Cell>, String> {
    let root = workspace_root();
    let mut cmd = std::process::Command::new(labwired_bin);
    cmd.arg("run")
        .arg("--chip")
        .arg(root.join(target.chip_yaml))
        .arg("--firmware")
        .arg(root.join(target.elf))
        .arg("--max-steps")
        .arg(target.max_steps.to_string());
    if target.rom_boot {
        cmd.arg("--rom-boot");
        let flash = target.flash_bin.ok_or("rom_boot target needs flash_bin")?;
        cmd.env("LABWIRED_ESP32S3_FLASH", root.join(flash));
    }
    let out = cmd.output().map_err(|e| format!("spawn labwired: {e}"))?;
    // UART echoes on stdout; the sim may exit nonzero on step-limit — that's
    // fine, the protocol lines are the verdict.
    let parsed = parse_tier1_uart(&out.stdout);
    let classes: Vec<&str> = RUBRIC_CLASSES
        .iter()
        .chain(target.extra_classes.iter())
        .copied()
        .collect();
    let mut row = parsed.into_row(&classes);
    let chip_yaml = std::fs::read_to_string(root.join(target.chip_yaml)).map_err(|e| e.to_string())?;
    apply_na(&mut row, &declared_classes_from_yaml(&chip_yaml)?);
    Ok(row)
}

/// Run every target whose fixture blobs exist. Returns the live matrix and the
/// list of skipped chips (missing fixtures — Part B not landed yet, or a fresh clone).
pub fn run_all(labwired_bin: &Path) -> Result<(Tier1Matrix, Vec<String>), String> {
    let root = workspace_root();
    let fixture_dir = root.join("tests/fixtures/tier1");
    if fixture_dir.join("MANIFEST.json").exists() {
        verify_fixture_manifest(&fixture_dir)?;
    }
    let mut matrix = Tier1Matrix::default();
    let mut skipped = Vec::new();
    for target in TIER1_TARGETS {
        if !root.join(target.elf).exists() {
            skipped.push(target.chip.to_string());
            continue;
        }
        let row = run_target(target, labwired_bin)?;
        matrix.0.insert(target.chip.to_string(), row);
    }
    Ok((matrix, skipped))
}
```

If `hex` is not already a labwired-cli dependency (`grep '^hex' crates/cli/Cargo.toml`), add `hex = "0.4"`.

- [ ] **Step 4: Run** — `cargo test -p labwired-cli --lib tier1` → `12 passed`.

- [ ] **Step 5: Commit** — `git add -u crates/cli && git commit -m "feat(tier1): target table, manifest verification, binary-driving runner"`

### Task 5: `tier1-matrix` CLI subcommand

**Files:**
- Modify: `core/crates/cli/src/main.rs` (mirror the `Coverage` wiring: enum variant near `Coverage(CoverageArgs)` ~line 106, dispatch near `Some(Commands::Coverage(args)) => run_coverage(args)`, handler near `run_coverage` ~line 1080)

- [ ] **Step 1: Add args struct + variant + dispatch:**

```rust
/// Run the Tier-1 chip × peripheral validation matrix and export it.
#[derive(Parser, Debug)]
pub struct Tier1MatrixArgs {
    /// Write the matrix as JSON (the committed snapshot path is
    /// docs/coverage/tier1-matrix.json).
    #[arg(long = "json-out")]
    pub json_out: Option<PathBuf>,

    /// Evidence link stamped into every non-unrecorded cell (CI passes its run URL).
    #[arg(long = "run-url")]
    pub run_url: Option<String>,
}
```

In `enum Commands`: `Tier1Matrix(Tier1MatrixArgs),` — and in the match: `Some(Commands::Tier1Matrix(args)) => run_tier1_matrix(args),`

Handler (place next to `run_coverage`):

```rust
fn run_tier1_matrix(args: Tier1MatrixArgs) -> ExitCode {
    let self_bin = std::env::current_exe().expect("current exe");
    match labwired_cli::tier1::run_all(&self_bin) {
        Ok((mut matrix, skipped)) => {
            for chip in &skipped {
                eprintln!("SKIP: {chip} (fixture not present)");
            }
            if let Some(url) = &args.run_url {
                use labwired_cli::tier1::CellStatus;
                for row in matrix.0.values_mut() {
                    for cell in row.values_mut() {
                        if cell.status != CellStatus::Unrecorded && cell.status != CellStatus::Na {
                            cell.run_url = Some(url.clone());
                        }
                    }
                }
            }
            // Text grid for humans.
            for (chip, row) in &matrix.0 {
                let cells: Vec<String> = row
                    .iter()
                    .map(|(class, cell)| format!("{class}={:?}", cell.status))
                    .collect();
                println!("{chip}: {}", cells.join(" "));
            }
            if let Some(out) = &args.json_out {
                let json = serde_json::to_string_pretty(&matrix).expect("serialize tier1 matrix");
                std::fs::write(out, json.as_bytes()).expect("write tier1 json");
                eprintln!("wrote {}", out.display());
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("tier1-matrix failed: {e}");
            ExitCode::FAILURE
        }
    }
}
```

- [ ] **Step 2: Build + smoke** — `cargo build -p labwired-cli && cargo run -p labwired-cli -- tier1-matrix` → prints `SKIP: esp32s3 (fixture not present)` (twice) and exits 0 (no fixtures yet).

- [ ] **Step 3: Commit** — `git add -u && git commit -m "feat(cli): tier1-matrix subcommand with --json-out and --run-url evidence stamping"`

### Task 6: Per-PR harness integration test

**Files:**
- Create: `core/crates/cli/tests/tier1_matrix.rs`

- [ ] **Step 1: Write the test:**

```rust
// Per-PR Tier-1 matrix harness. Runs every target whose committed fixture
// exists; skips cleanly (like svd_coverage_ratchet) on fresh clones or before
// Part B lands the blobs.
use labwired_cli::tier1;

#[test]
fn tier1_matrix_runs_all_available_fixtures() {
    let bin = std::path::Path::new(env!("CARGO_BIN_EXE_labwired"));
    let (matrix, skipped) = tier1::run_all(bin).expect("tier1 run_all");
    for chip in &skipped {
        eprintln!("SKIP: {chip} (fixture not present)");
    }
    // Every exercised chip must produce a full row (rubric + extra classes).
    for (chip, row) in &matrix.0 {
        let target = tier1::TIER1_TARGETS
            .iter()
            .find(|t| t.chip == chip.as_str())
            .expect("target for chip");
        let expected = tier1::RUBRIC_CLASSES.len() + target.extra_classes.len();
        assert_eq!(row.len(), expected, "{chip}: row incomplete: {row:?}");
    }
}
```

- [ ] **Step 2: Run** — `cargo test -p labwired-cli --test tier1_matrix 2>&1 | tail -3` → passes with SKIP lines (no fixtures yet).

- [ ] **Step 3: Commit** — `git add crates/cli/tests/tier1_matrix.rs && git commit -m "test(tier1): per-PR matrix harness, skips without fixtures"`

### Task 7: Ratchet gate integration test

**Files:**
- Create: `core/crates/cli/tests/tier1_matrix_ratchet.rs`

- [ ] **Step 1: Write the test** (mirrors `svd_coverage_ratchet.rs` structure):

```rust
// Regression ratchet: recorded `pass` cells in docs/coverage/tier1-matrix.json
// may never silently regress. Skips before the snapshot exists (pre-Part B).
use labwired_cli::tier1;

#[test]
fn tier1_matrix_does_not_regress() {
    let root = tier1::workspace_root();
    let snapshot_path = root.join("docs/coverage/tier1-matrix.json");
    if !snapshot_path.exists() {
        eprintln!("SKIP: no tier1 snapshot at {}", snapshot_path.display());
        return;
    }
    let snapshot: tier1::Tier1Matrix =
        serde_json::from_str(&std::fs::read_to_string(&snapshot_path).expect("read snapshot"))
            .expect("parse snapshot");

    let bin = std::path::Path::new(env!("CARGO_BIN_EXE_labwired"));
    let (live, skipped) = tier1::run_all(bin).expect("tier1 run_all");
    for chip in &skipped {
        eprintln!("SKIP: {chip} (fixture not present)");
    }

    let regressions = tier1::ratchet_regressions(&snapshot, &live);
    assert!(
        regressions.is_empty(),
        "tier1 matrix regressed: {regressions:?}. If intentional, edit the snapshot \
         explicitly; to record improvements regenerate: \
         cargo run -p labwired-cli -- tier1-matrix --json-out docs/coverage/tier1-matrix.json"
    );
}
```

- [ ] **Step 2: Run** — `cargo test -p labwired-cli --test tier1_matrix_ratchet 2>&1 | tail -3` → passes with SKIP (no snapshot yet).

- [ ] **Step 3: Full gate check** — `cargo fmt --all && cargo clippy -p labwired-cli --all-targets -- -D warnings && cargo test -p labwired-cli 2>&1 | tail -3` → all green.

- [ ] **Step 4: Commit** — `git add crates/cli/tests/tier1_matrix_ratchet.rs && git commit -m "test(tier1): ratchet gate against committed snapshot"`

### Task 8: Scoreboard renderer

**Files:**
- Create: `core/scripts/generate_tier1_scoreboard.py`
- Modify: `core/docs/coverage_scoreboard.md` (add a link line)

- [ ] **Step 1: Write the script:**

```python
#!/usr/bin/env python3
"""Render docs/coverage/tier1-matrix.json as a chip × peripheral markdown grid.

Proof-artifact bar (spec wedge-alignment §2): a cell renders its real status
ONLY if it carries a run_url; cells without evidence render as unrecorded.
"""
import argparse
import json
from pathlib import Path

ICONS = {"pass": "✅", "partial": "🟡", "blocked": "⛔", "na": "—", "unrecorded": "·"}
RUBRIC = ["clock", "gpio", "uart", "timer", "dma", "irq"]


def render(matrix: dict) -> str:
    # Column set = rubric order, then any extra classes seen (e.g. S3 beachhead).
    extras = sorted({c for row in matrix.values() for c in row if c not in RUBRIC})
    classes = RUBRIC + extras
    lines = [
        "# Tier-1 validation matrix",
        "",
        "Every cell links the CI run that produced it; no link → `·` unrecorded.",
        "",
        "| chip | " + " | ".join(classes) + " |",
        "|---|" + "---|" * len(classes),
    ]
    for chip in sorted(matrix):
        row = matrix[chip]
        cells = []
        for cls in classes:
            cell = row.get(cls)
            if cell is None:
                cells.append("·")
                continue
            status, url = cell.get("status", "unrecorded"), cell.get("run_url")
            if status not in ("na", "unrecorded") and not url:
                status = "unrecorded"  # no evidence, no claim
            icon = ICONS.get(status, "·")
            cells.append(f"[{icon}]({url})" if url else icon)
        lines.append(f"| {chip} | " + " | ".join(cells) + " |")
    return "\n".join(lines) + "\n"


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--matrix", default="docs/coverage/tier1-matrix.json")
    ap.add_argument("--out", default="docs/coverage/tier1-scoreboard.md")
    args = ap.parse_args()
    matrix = json.loads(Path(args.matrix).read_text())
    Path(args.out).write_text(render(matrix))
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()
```

- [ ] **Step 2: Test with a sample** (no committed snapshot yet):

```bash
cd ~/projects/labwired/core
cat > /tmp/tier1-sample.json <<'EOF'
{ "esp32s3": {
    "gpio":  { "status": "pass", "run_url": "https://github.com/w1ne/labwired-core/actions/runs/1" },
    "mcpwm": { "status": "pass" },
    "dma":   { "status": "blocked", "run_url": "https://github.com/w1ne/labwired-core/actions/runs/1" } } }
EOF
python3 scripts/generate_tier1_scoreboard.py --matrix /tmp/tier1-sample.json --out /tmp/tier1-scoreboard.md
cat /tmp/tier1-scoreboard.md
```

Expected: `gpio` renders `[✅](…runs/1)`, `mcpwm` renders `·` (pass but **no run_url** → unrecorded), `dma` renders `[⛔](…runs/1)`.

- [ ] **Step 3: Link from the scoreboard doc** — append to `docs/coverage_scoreboard.md`:

```markdown

## Tier-1 validation matrix

Per-chip, per-peripheral real-firmware validation: [tier1-scoreboard](coverage/tier1-scoreboard.md).
```

- [ ] **Step 4: Commit** — `git add scripts/generate_tier1_scoreboard.py docs/coverage_scoreboard.md && git commit -m "feat(tier1): scoreboard renderer with run_url proof-artifact bar"`

**Part A checkpoint:** open a PR (`feat/tier1-matrix-infra`), let `core-integrity` run (everything skips gracefully), merge. Part B can be a second PR.

---

# Part B — ESP32-S3 fixture + snapshot (needs espressif toolchain on the dev machine)

### Task 9: Fixture firmware source

**Files:**
- Create: `core/examples/tier1-fixture/esp32s3/Cargo.toml`
- Create: `core/examples/tier1-fixture/esp32s3/src/main.rs`
- Create: `core/examples/tier1-fixture/esp32s3/.cargo/config.toml`
- Create: `core/examples/tier1-fixture/esp32s3/rust-toolchain.toml`

Copy the scaffolding pattern from `core/examples/esp32s3-hello-world/` (same esp-hal version, same `.cargo/config.toml` target/runner and `rust-toolchain.toml` channel = `esp`). The fixture is deliberately raw-register (no HAL drivers beyond clock/UART init) so it validates the *model*, not the HAL.

- [ ] **Step 1: Cargo.toml** (match the esp-hal version `grep esp-hal examples/esp32s3-hello-world/Cargo.toml` reports):

```toml
[package]
name = "tier1-fixture-esp32s3"
version = "0.1.0"
edition = "2021"

[dependencies]
esp-hal = { version = "1.1", features = ["esp32s3", "unstable"] }
esp-backtrace = { version = "0.17", features = ["esp32s3", "panic-handler"] }
esp-println = { version = "0.15", features = ["esp32s3", "uart"] }

[profile.release]
opt-level = "s"
lto = "fat"
```

(If `examples/esp32s3-hello-world` pins different esp-backtrace/esp-println versions, use those.)

- [ ] **Step 2: src/main.rs** — the complete fixture:

```rust
//! Tier-1 validation fixture for ESP32-S3.
//!
//! Prints `TIER1 <class> PASS|FAIL` per peripheral class over UART0, then
//! `TIER1 done`. Raw register pokes (TRM/SVD addresses) — the point is to
//! validate the chip MODEL, not esp-hal. Runs after the genuine ROM +
//! bootloader under `labwired run --rom-boot`.
#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::main;
use esp_println::println;

const GPIO_BASE: u32 = 0x6000_4000;
const GPIO_OUT_W1TS: u32 = GPIO_BASE + 0x08;
const GPIO_OUT_W1TC: u32 = GPIO_BASE + 0x0C;
const GPIO_OUT: u32 = GPIO_BASE + 0x04;
const GPIO_ENABLE_W1TS: u32 = GPIO_BASE + 0x24;

const TIMG0_BASE: u32 = 0x6001_F000;
const TIMG0_T0CONFIG: u32 = TIMG0_BASE + 0x00;
const TIMG0_T0LO: u32 = TIMG0_BASE + 0x04;
const TIMG0_T0UPDATE: u32 = TIMG0_BASE + 0x0C;

const SYSTIMER_BASE: u32 = 0x6002_3000;
const SYSTIMER_CONF: u32 = SYSTIMER_BASE + 0x00;
const SYSTIMER_UNIT0_OP: u32 = SYSTIMER_BASE + 0x04;
const SYSTIMER_UNIT0_VALUE_LO: u32 = SYSTIMER_BASE + 0x44;

const MCPWM0_BASE: u32 = 0x6001_E000;
const MCPWM0_CLK_CFG: u32 = MCPWM0_BASE + 0x00;
const MCPWM0_TIMER0_CFG0: u32 = MCPWM0_BASE + 0x04;
const MCPWM0_TIMER0_CFG1: u32 = MCPWM0_BASE + 0x08;
const MCPWM0_INT_RAW: u32 = MCPWM0_BASE + 0x110;
const MCPWM0_INT_CLR: u32 = MCPWM0_BASE + 0x118;
const MCPWM_TIMER0_TEZ_INT_RAW_BIT: u32 = 1 << 3;

const RMT_BASE: u32 = 0x6001_6000;
const RMT_CH0DATA_MEM: u32 = 0x6001_6800; // channel 0 RAM
const RMT_CH0CONF0: u32 = RMT_BASE + 0x20;
const RMT_CH0_TX_LIM: u32 = RMT_BASE + 0xA0;
const RMT_INT_RAW: u32 = RMT_BASE + 0x70;
const RMT_INT_CLR: u32 = RMT_BASE + 0x78;
const RMT_SYS_CONF: u32 = RMT_BASE + 0x68;
const RMT_CH0_TX_END_BIT: u32 = 1 << 0;

const I2C0_BASE: u32 = 0x6001_3000;
const I2C0_CTR: u32 = I2C0_BASE + 0x04;
const I2C0_INT_RAW: u32 = I2C0_BASE + 0x20;
const I2C0_INT_CLR: u32 = I2C0_BASE + 0x24;
const I2C0_CMD0: u32 = I2C0_BASE + 0x58;
const I2C0_FIFO: u32 = I2C0_BASE + 0x1C;

const INTMATRIX_TIMG0_T0_MAP: u32 = 0x600C_2000 + 4 * 50; // source 50 = TG0_T0_LEVEL

#[inline(always)]
fn rr(addr: u32) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}
#[inline(always)]
fn wr(addr: u32, v: u32) {
    unsafe { core::ptr::write_volatile(addr as *mut u32, v) }
}

fn report(class: &str, ok: bool, code: &str) {
    if ok {
        println!("TIER1 {} PASS", class);
    } else {
        println!("TIER1 {} FAIL code={}", class, code);
    }
}

/// Spin a bounded number of polls; the sim is deterministic so this can't flake.
fn poll_until(mut f: impl FnMut() -> bool, tries: u32) -> bool {
    for _ in 0..tries {
        if f() {
            return true;
        }
    }
    false
}

fn check_clock() -> bool {
    // SYSTIMER advances: enable, snapshot, busy-wait, snapshot again.
    wr(SYSTIMER_CONF, rr(SYSTIMER_CONF) | (1 << 30)); // clk_en
    wr(SYSTIMER_UNIT0_OP, 1 << 30); // update
    let a = rr(SYSTIMER_UNIT0_VALUE_LO);
    for _ in 0..2_000 {
        core::hint::spin_loop();
    }
    wr(SYSTIMER_UNIT0_OP, 1 << 30);
    let b = rr(SYSTIMER_UNIT0_VALUE_LO);
    b != a
}

fn check_gpio() -> bool {
    // Drive GPIO5 high then low; read back via OUT register.
    wr(GPIO_ENABLE_W1TS, 1 << 5);
    wr(GPIO_OUT_W1TS, 1 << 5);
    let high = rr(GPIO_OUT) & (1 << 5) != 0;
    wr(GPIO_OUT_W1TC, 1 << 5);
    let low = rr(GPIO_OUT) & (1 << 5) == 0;
    high && low
}

fn check_timer() -> bool {
    // TIMG0 T0: enable + increase, latch twice, counter must advance.
    wr(TIMG0_T0CONFIG, (1 << 31) | (1 << 30) | (2 << 13)); // EN | INCREASE | divider=2
    wr(TIMG0_T0UPDATE, 1);
    let a = rr(TIMG0_T0LO);
    for _ in 0..2_000 {
        core::hint::spin_loop();
    }
    wr(TIMG0_T0UPDATE, 1);
    let b = rr(TIMG0_T0LO);
    b != a
}

fn check_irq() -> bool {
    // Route TG0_T0 to a CPU interrupt line via the interrupt matrix and verify
    // the mapping register round-trips (delivery itself is exercised by the
    // intmatrix_alarm integration test; here we prove the matrix is wired).
    wr(INTMATRIX_TIMG0_T0_MAP, 10);
    rr(INTMATRIX_TIMG0_T0_MAP) & 0x1F == 10
}

fn check_dma() -> bool {
    // GDMA mem-to-mem is not modeled yet (register-file only) — attempt a
    // minimal channel-enable round-trip so the day it grows behavior this
    // starts passing; until then FAIL is the honest, recorded answer.
    const GDMA_BASE: u32 = 0x6003_F000;
    const GDMA_MISC_CONF: u32 = GDMA_BASE + 0x44;
    wr(GDMA_MISC_CONF, 1); // ahbm_rst_inter
    rr(GDMA_MISC_CONF) == 1 && {
        // require an actual transfer before calling it PASS:
        false
    }
}

fn check_mcpwm() -> bool {
    // Timer0 free-runs: prescale, period, continuous mode → TEZ raw interrupt.
    wr(MCPWM0_CLK_CFG, 3); // prescale
    wr(MCPWM0_TIMER0_CFG0, (100 << 8) | 3); // period=100, prescale=3
    wr(MCPWM0_INT_CLR, MCPWM_TIMER0_TEZ_INT_RAW_BIT);
    wr(MCPWM0_TIMER0_CFG1, 2); // mode=increase, start free-running
    poll_until(|| rr(MCPWM0_INT_RAW) & MCPWM_TIMER0_TEZ_INT_RAW_BIT != 0, 50_000)
}

fn check_rmt() -> bool {
    // One end-marker entry in channel-0 RAM, conf, TX start → ch0_tx_end raw.
    wr(RMT_SYS_CONF, rr(RMT_SYS_CONF) | (1 << 0)); // clk en (apb_fifo_mask side effects fine)
    wr(RMT_CH0DATA_MEM, 0x0000_0000); // end marker
    wr(RMT_CH0_TX_LIM, 0);
    wr(RMT_INT_CLR, RMT_CH0_TX_END_BIT);
    wr(RMT_CH0CONF0, rr(RMT_CH0CONF0) | (1 << 0)); // tx_start
    poll_until(|| rr(RMT_INT_RAW) & RMT_CH0_TX_END_BIT != 0, 10_000)
}

fn check_i2c() -> bool {
    // Command-list engine: a START+STOP sequence to an empty bus must complete
    // (trans_complete int) — NACK is fine, a dead engine is not.
    const TRANS_COMPLETE: u32 = 1 << 7;
    wr(I2C0_INT_CLR, 0xFFFF_FFFF);
    wr(I2C0_FIFO, 0xAA); // address byte into TX FIFO
    wr(I2C0_CMD0, 6 << 11); // OP=RSTART then rely on engine defaults; CMD1 STOP
    wr(I2C0_BASE + 0x5C, (3 << 11) | (1 << 8)); // CMD1: STOP
    wr(I2C0_CTR, rr(I2C0_CTR) | (1 << 5)); // trans_start
    poll_until(|| rr(I2C0_INT_RAW) & TRANS_COMPLETE != 0, 50_000)
}

#[main]
fn main() -> ! {
    let _p = esp_hal::init(esp_hal::Config::default());
    report("clock", check_clock(), "systimer-stuck");
    report("gpio", check_gpio(), "out-readback");
    report("timer", check_timer(), "timg0-stuck");
    report("irq", check_irq(), "intmatrix-map");
    report("dma", check_dma(), "gdma-no-m2m-model");
    report("mcpwm", check_mcpwm(), "tez-int-missing");
    report("rmt", check_rmt(), "tx-end-missing");
    report("i2c", check_i2c(), "trans-complete-missing");
    println!("TIER1 done");
    loop {
        core::hint::spin_loop();
    }
}
```

**Register-address caveat for the implementer:** the I2C command encoding and RMT/MCPWM bit positions above are from the ESP32-S3 TRM as modeled in `crates/core/src/peripherals/esp32s3/{i2c,rmt,mcpwm}.rs` — before building, cross-check each constant against those model files (they document offsets/bits in comments) and against `examples/esp32s3-i2c-tmp102` (working I2C command-list usage). Where the model and this plan disagree, the model file is right; fix the constant, not the model.

- [ ] **Step 3: Build it** (toolchain machine):

```bash
cd ~/projects/labwired/core/examples/tier1-fixture/esp32s3
source ~/export-esp.sh
cargo build --release
ls target/xtensa-esp32s3-none-elf/release/tier1-fixture-esp32s3
```

- [ ] **Step 4: Iterate against the sim until the protocol is honest.** Run:

```bash
cd ~/projects/labwired/core
espflash save-image --chip esp32s3 \
  examples/tier1-fixture/esp32s3/target/xtensa-esp32s3-none-elf/release/tier1-fixture-esp32s3 \
  /tmp/tier1-s3-flash.bin
LABWIRED_ESP32S3_FLASH=/tmp/tier1-s3-flash.bin \
  cargo run -p labwired-cli --release -- run --rom-boot \
  --chip configs/chips/esp32s3.yaml \
  --firmware examples/tier1-fixture/esp32s3/target/xtensa-esp32s3-none-elf/release/tier1-fixture-esp32s3 \
  --max-steps 40000000 | grep TIER1
```

Expected end state: `TIER1` lines for all 8 classes + `done`; `dma` FAILs honestly (`gdma-no-m2m-model`); every other class PASSes. If `mcpwm`/`rmt`/`i2c` FAIL, debug the *fixture constants* first (Step 2 caveat), then the model — and if a genuine model gap is found, record FAIL and file an issue; do NOT weaken the check (no gate tampering).

- [ ] **Step 5: Commit source** — `git add examples/tier1-fixture && git commit -m "feat(tier1): ESP32-S3 fixture firmware — rubric + mcpwm/i2c/rmt beachhead checks"`

### Task 10: Build script + committed blobs

**Files:**
- Create: `core/scripts/build_tier1_fixtures.sh`
- Create: `core/tests/fixtures/tier1/{esp32s3.elf,esp32s3-flash.bin,MANIFEST.json}`

- [ ] **Step 1: The build script:**

```bash
#!/usr/bin/env bash
# Rebuild the committed Tier-1 fixture blobs from source and refresh MANIFEST.json.
# Needs the espressif Rust toolchain (`source ~/export-esp.sh`) + espflash.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/tests/fixtures/tier1"
mkdir -p "$OUT"

build_s3() {
  local src="$ROOT/examples/tier1-fixture/esp32s3"
  (cd "$src" && cargo build --release)
  local elf="$src/target/xtensa-esp32s3-none-elf/release/tier1-fixture-esp32s3"
  cp "$elf" "$OUT/esp32s3.elf"
  espflash save-image --chip esp32s3 "$elf" "$OUT/esp32s3-flash.bin"
}

build_s3

# Refresh the manifest: file -> { sha256, source_rev }.
(cd "$OUT" && python3 - <<'EOF'
import hashlib, json, pathlib, subprocess
rev = subprocess.run(["git", "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
manifest = {}
for f in sorted(pathlib.Path(".").iterdir()):
    if f.suffix in (".elf", ".bin"):
        manifest[f.name] = {
            "sha256": hashlib.sha256(f.read_bytes()).hexdigest(),
            "source_rev": rev,
        }
pathlib.Path("MANIFEST.json").write_text(json.dumps(manifest, indent=2) + "\n")
print("MANIFEST.json refreshed")
EOF
)
```

`chmod +x scripts/build_tier1_fixtures.sh`

- [ ] **Step 2: Run it** — `source ~/export-esp.sh && ./scripts/build_tier1_fixtures.sh` → blobs + manifest in `tests/fixtures/tier1/`.

- [ ] **Step 3: Verify the harness picks them up:**

```bash
cargo test -p labwired-cli --test tier1_matrix 2>&1 | tail -5
```

Expected: no SKIP lines for esp32s3/esp32s3-zero; test passes with full rows.

- [ ] **Step 4: Corruption check** — flip one byte of `esp32s3.elf` (`printf '\x00' | dd of=tests/fixtures/tier1/esp32s3.elf bs=1 seek=100 conv=notrunc`), run the harness, expect a sha256-mismatch failure; `git checkout tests/fixtures/tier1/esp32s3.elf` to restore.

- [ ] **Step 5: Commit** — `git add scripts/build_tier1_fixtures.sh tests/fixtures/tier1 && git commit -m "feat(tier1): committed content-hashed ESP32-S3 fixture blobs + rebuild script"`

### Task 11: Initial snapshot + full-gate run

**Files:**
- Create: `core/docs/coverage/tier1-matrix.json`

- [ ] **Step 1: Generate** — `cargo run -p labwired-cli --release -- tier1-matrix --json-out docs/coverage/tier1-matrix.json` (local generation carries no `--run-url`; the scoreboard will render cells unrecorded until CI stamps them — expected per the proof-artifact bar).

- [ ] **Step 2: Render the scoreboard** — `python3 scripts/generate_tier1_scoreboard.py` → `docs/coverage/tier1-scoreboard.md`; eyeball the grid.

- [ ] **Step 3: Ratchet now bites** — `cargo test -p labwired-cli --test tier1_matrix_ratchet 2>&1 | tail -3` → passes (live == snapshot).

- [ ] **Step 4: Full gates** — `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -2 && cargo test --workspace 2>&1 | grep -ac "test result: FAILED" || true` → 0 failures.

- [ ] **Step 5: Commit** — `git add docs/coverage && git commit -m "chore(tier1): initial matrix snapshot + scoreboard (esp32s3 beachhead row)"`

### Task 12: CI evidence stamping + weekly drift check

**Files:**
- Modify: `core/.github/workflows/core-nightly.yml`
- Modify: `core/.github/workflows/core-validate-hw-targets.yml` *(stamping only — full catalog verdict integration is P2)*

- [ ] **Step 1: Nightly job** — append to `core-nightly.yml` `jobs:`:

```yaml
  tier1-matrix:
    name: Tier-1 matrix (evidence stamp + drift check)
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.95.0
      - uses: Swatinem/rust-cache@v2
        with:
          shared-key: core-nightly
          workspaces: |
            . -> target
      - name: Run matrix with evidence URL
        run: |
          cargo run -p labwired-cli --release -- tier1-matrix \
            --json-out docs/coverage/tier1-matrix.json \
            --run-url "${{ github.server_url }}/${{ github.repository }}/actions/runs/${{ github.run_id }}"
          python3 scripts/generate_tier1_scoreboard.py
      - name: Commit refreshed snapshot if changed
        run: |
          git config user.name "labwired-ci"
          git config user.email "14119286+w1ne@users.noreply.github.com"
          git add docs/coverage/tier1-matrix.json docs/coverage/tier1-scoreboard.md
          git diff --cached --quiet || git commit -m "chore(tier1): nightly matrix refresh with run evidence"
          git push
```

(Mirror the auth pattern the existing catalog-autoupdate job in `core-validate-hw-targets.yml` uses — if it pushes via a PAT/secret or a different commit identity, copy that exactly instead of the block above.)

- [ ] **Step 2: Weekly drift check** — in the same workflow, a second job gated to Sundays:

```yaml
  tier1-fixture-drift:
    name: Tier-1 fixture source↔binary drift
    runs-on: ubuntu-latest
    if: github.event.schedule == '' || startsWith(github.event.schedule, '0 2') # manual or the nightly cron; guard body picks Sunday
    steps:
      - uses: actions/checkout@v4
      - name: Only on Sundays (cron has no weekly slot in this file)
        run: |
          [ "$(date -u +%u)" = "7" ] || { echo "not Sunday — skip"; exit 0; }
      - name: Install espressif toolchain
        run: |
          cargo install espup espflash --locked
          espup install
      - name: Rebuild and compare
        run: |
          source ~/export-esp.sh
          ./scripts/build_tier1_fixtures.sh
          git diff --exit-code tests/fixtures/tier1/ \
            || { echo "::error::fixture blobs drifted from source"; exit 1; }
```

- [ ] **Step 3: Validate workflow syntax** — `gh workflow list` after push; or `actionlint .github/workflows/core-nightly.yml` if installed.

- [ ] **Step 4: Commit** — `git add .github/workflows && git commit -m "ci(tier1): nightly evidence-stamped refresh + weekly fixture drift check"`

- [ ] **Step 5: PR + merge** — open `feat/tier1-matrix-esp32s3` PR against core main, wait for `core-integrity` (now actually running the matrix), merge, then bump the core submodule in the parent labwired repo.

---

# Part C — Public web scoreboard (parent repo, no toolchain needed)

### Task 13: Validation-matrix page on the playground site

**Files:**
- Create: `packages/playground/src/ValidationMatrix.tsx`
- Create: `packages/playground/src/ValidationMatrix.test.tsx`
- Modify: the playground router/nav where `/ci` is wired (find with `grep -rn "'/ci'\|\"/ci\"" packages/playground/src | head`) to add a `/validation` route + nav link

This is the **public trace**: a styled chip × peripheral grid fetching the raw
snapshot from core main, so the page is as fresh as the last nightly run with
zero deploys. It is the outreach link for the driver-bringup-CI beachhead.

- [ ] **Step 1: Write the failing test:**

```tsx
// packages/playground/src/ValidationMatrix.test.tsx
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import { ValidationMatrix, MATRIX_URL } from './ValidationMatrix';

const SAMPLE = {
  esp32s3: {
    gpio: { status: 'pass', run_url: 'https://github.com/w1ne/labwired-core/actions/runs/1' },
    mcpwm: { status: 'pass' }, // no evidence -> must render unrecorded
    dma: { status: 'blocked', run_url: 'https://github.com/w1ne/labwired-core/actions/runs/1' },
  },
};

describe('ValidationMatrix', () => {
  beforeEach(() => {
    vi.stubGlobal('fetch', vi.fn(() =>
      Promise.resolve(new Response(JSON.stringify(SAMPLE), { status: 200 })),
    ));
  });

  it('fetches the core-main snapshot and renders evidence-linked cells', async () => {
    render(<ValidationMatrix />);
    await waitFor(() => expect(screen.getByText('esp32s3')).toBeTruthy());
    expect(fetch).toHaveBeenCalledWith(MATRIX_URL);
    // gpio: pass + run_url -> a link to the CI run
    const gpioLink = screen.getByRole('link', { name: /gpio: pass/i });
    expect(gpioLink.getAttribute('href')).toContain('/actions/runs/1');
  });

  it('downgrades evidence-less cells to unrecorded (proof-artifact bar)', async () => {
    render(<ValidationMatrix />);
    await waitFor(() => expect(screen.getByText('esp32s3')).toBeTruthy());
    // mcpwm has status pass but NO run_url -> rendered unrecorded, no link
    expect(screen.getByLabelText('mcpwm: unrecorded')).toBeTruthy();
    expect(screen.queryByRole('link', { name: /mcpwm: pass/i })).toBeNull();
  });

  it('shows a graceful empty state when the fetch fails', async () => {
    (fetch as ReturnType<typeof vi.fn>).mockImplementationOnce(() => Promise.reject(new Error('offline')));
    render(<ValidationMatrix />);
    await waitFor(() => expect(screen.getByText(/validation data unavailable/i)).toBeTruthy());
  });
});
```

- [ ] **Step 2: Run to verify failure**

Run: `cd ~/projects/labwired/packages/playground && npx vitest run src/ValidationMatrix.test.tsx 2>&1 | tail -5`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement the component** (match the playground's existing styling idiom — check how `/ci` page components are styled, e.g. CSS modules vs inline; the version below uses plain classNames the implementer should map onto the site's existing card/table styles):

```tsx
// packages/playground/src/ValidationMatrix.tsx
//
// Public Tier-1 validation matrix — the "public trace". One source of truth:
// docs/coverage/tier1-matrix.json on core main (nightly-refreshed with CI
// run_url evidence). Proof-artifact bar: a cell renders its status ONLY if it
// carries a run_url; status without evidence renders as unrecorded.
import { useEffect, useState } from 'react';

export const MATRIX_URL =
  'https://raw.githubusercontent.com/w1ne/labwired-core/main/docs/coverage/tier1-matrix.json';

const RUBRIC = ['clock', 'gpio', 'uart', 'timer', 'dma', 'irq'];

type Cell = { status: string; run_url?: string };
type Matrix = Record<string, Record<string, Cell>>;

const ICON: Record<string, string> = {
  pass: '✅',
  partial: '🟡',
  blocked: '⛔',
  na: '—',
  unrecorded: '·',
};

function effectiveStatus(cell: Cell | undefined): { status: string; url?: string } {
  if (!cell) return { status: 'unrecorded' };
  if (cell.status === 'na' || cell.status === 'unrecorded') return { status: cell.status };
  if (!cell.run_url) return { status: 'unrecorded' }; // no evidence, no claim
  return { status: cell.status, url: cell.run_url };
}

export function ValidationMatrix() {
  const [matrix, setMatrix] = useState<Matrix | null>(null);
  const [error, setError] = useState(false);

  useEffect(() => {
    fetch(MATRIX_URL)
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then(setMatrix)
      .catch(() => setError(true));
  }, []);

  if (error) return <p className="validation-empty">Validation data unavailable.</p>;
  if (!matrix) return <p className="validation-empty">Loading validation matrix…</p>;

  const chips = Object.keys(matrix).sort();
  const extras = [...new Set(chips.flatMap((c) => Object.keys(matrix[c])))].filter(
    (k) => !RUBRIC.includes(k),
  ).sort();
  const classes = [...RUBRIC, ...extras];

  return (
    <section className="validation-matrix">
      <h2>Validated in CI, on real firmware</h2>
      <p>
        Every green cell links the CI run that proved it — peripheral-by-peripheral,
        chip-by-chip, refreshed nightly. No link, no claim.
      </p>
      <table>
        <thead>
          <tr>
            <th>chip</th>
            {classes.map((c) => (
              <th key={c}>{c}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {chips.map((chip) => (
            <tr key={chip}>
              <td>{chip}</td>
              {classes.map((cls) => {
                const { status, url } = effectiveStatus(matrix[chip][cls]);
                const label = `${cls}: ${status}`;
                return (
                  <td key={cls} aria-label={label}>
                    {url ? (
                      <a href={url} aria-label={label} target="_blank" rel="noreferrer">
                        {ICON[status] ?? '·'}
                      </a>
                    ) : (
                      <span aria-label={label}>{ICON[status] ?? '·'}</span>
                    )}
                  </td>
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}
```

- [ ] **Step 4: Run tests** — `npx vitest run src/ValidationMatrix.test.tsx` → 3 passed.

- [ ] **Step 5: Wire the route + nav.** Find the `/ci` route registration (`grep -rn "'/ci'" packages/playground/src`), add a `/validation` route rendering `<ValidationMatrix />`, and a nav link labeled "Validation" next to the CI nav item, following the exact pattern of the existing route/nav entries.

- [ ] **Step 6: Full playground suite** — `npm test 2>&1 | tail -3` → all green.

- [ ] **Step 7: Visual check, then hand off for user preview.** `npm run dev`, open `/validation` with chrome-devtools, screenshot the grid. **Do NOT deploy** — per standing rule, web deploys ship only after the user previews the rendered page; give the user the local URL.

- [ ] **Step 8: Commit** — `git add packages/playground/src/ValidationMatrix.tsx packages/playground/src/ValidationMatrix.test.tsx <router/nav files> && git commit -m "feat(playground): public Tier-1 validation matrix page — nightly-fresh, evidence-linked"`

---

## Self-review notes (already applied)

- Spec coverage: matrix model (T1–T3), fixture protocol + binary policy (T9–T10), harness (T4, T6), ratchet (T3, T7), CLI exporter (T5), run_url proof bar (T5, T8, T12), scoreboard (T8), CI wiring (T12), beachhead extra classes (T4 target table, T9 fixture). Catalog-verdict integration deferred to P2 per spec phasing.
- The fixture's register constants are flagged as must-verify-against-model rather than trusted; the model files are the source of truth.
- `dma` is expected to record FAIL honestly (GDMA m2m unmodeled) — the ratchet protects nothing there until the model grows the behavior.
