// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Per-chip **bus visibility** scoreboard + ratchet.
//!
//! # The gap this closes
//!
//! The in-engine logic analyzer samples pads. A bus is therefore only
//! measurable on a chip if that bus's controller wire is BOUND to a pad — which
//! is what the `wire_*_pads` functions in `crates/core/src/bus/attach.rs` do.
//! A chip whose family has no such binding boots, runs firmware, passes every
//! conformance gate, and shows a flat line on every probe: the SPI traffic is
//! real, the pad simply reads the GPIO output latch instead of the wire.
//!
//! Before this board nothing recorded that. Nine wiring functions covered some
//! families and not others, no artifact said which, and a new chip could be
//! onboarded with zero bus visibility without a single gate noticing.
//!
//! # Derived, never hand-maintained
//!
//! There is no list of "chips that have I²C" here to fall out of date. For every
//! chip in `configs/chips/`, this builds the real bus through
//! `SystemBus::from_config` — the same path `chip_conformance.rs` uses — and
//! asks it which signal names are actually bound to pads
//! ([`SystemBus::bound_pad_functions`]). The datasheet name of each binding
//! (`"I2C1_SCL"`, `"SPI0_SCK"`, `"USART3_TX"`, `"I2CEXT0_SDA"`) is classified
//! into a bus kind. Delete a `wire_*_pads` call and the cell goes empty here.
//!
//! ⚠️ An unrecognised function name is a hard FAILURE, never a silent drop.
//! That is the single most important property of this gate: if a rename made
//! `classify` return `None` and we skipped it, the matrix would quietly empty
//! itself and the ratchet — which only fires on a LOSS it can see — would be
//! comparing nothing to nothing. Adding a new signal-name stem is a deliberate
//! edit to [`STEMS`], made by whoever introduced the name.
//!
//! # Artifacts
//!
//! * `docs/coverage/bus-visibility.md` — the human board, CHECKED not rewritten.
//! * `docs/coverage/bus-visibility.json` — the ratchet baseline.
//!
//! A chip may GAIN a bus freely; losing one fails. Re-baseline (after a
//! deliberate, explained change):
//!
//! ```text
//! UPDATE_BUS_VISIBILITY_BASELINE=1 cargo test -p labwired-core --test bus_visibility
//! ```

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use std::path::PathBuf;

/// The bus kinds a probe can be asked to decode. Extending this is a deliberate
/// act: a kind with no [`STEMS`] entry can never appear on the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum BusKind {
    I2c,
    Spi,
    Uart,
}

impl BusKind {
    /// Column order on the board, and the order buses are recorded in the
    /// baseline — stable so the artifacts do not churn.
    const ALL: &'static [BusKind] = &[BusKind::I2c, BusKind::Spi, BusKind::Uart];

    fn label(self) -> &'static str {
        match self {
            BusKind::I2c => "I2C",
            BusKind::Spi => "SPI",
            BusKind::Uart => "UART",
        }
    }
}

/// Signal-name stems the engine actually binds, and the bus each denotes.
///
/// A bound function name is `<STEM><instance>_<signal>`: `I2C1_SCL`,
/// `SPI0_CSn`, `USART3_TX`, `I2CEXT0_SDA` (the ESP32 output-matrix spelling).
/// The stem is matched EXACTLY after the instance digits are stripped, so a
/// near-miss like `I2CFOO0_SCL` is rejected rather than swallowed by a loose
/// `starts_with("I2C")`.
///
/// ⚠️ Longest stem first is load-bearing for readability only — `classify`
/// strips digits and compares the whole stem, so `I2CEXT` cannot be shadowed by
/// `I2C`. If you add a new peripheral naming convention (`TWIM0_SCL`,
/// `LPUART1_TX`, …) the test will fail naming it, and adding the row here is
/// the fix. That failure is the contract, not an inconvenience.
const STEMS: &[(&str, BusKind)] = &[
    ("I2CEXT", BusKind::I2c),
    ("I2C", BusKind::I2c),
    // Nordic spells its instances after the IP block, not after the protocol:
    // the I²C master is TWIM, the SPI master SPIM, the serial port UARTE. The
    // names come from the nRF52840 PS instance tables (§6.31.7 p790, §6.25.6
    // p727, §6.34.9 p836) and are what a user reading the trace sees in the
    // datasheet, so they are kept rather than translated. `classify` compares
    // whole stems, so `UARTE` cannot be shadowed by `UART`.
    ("TWIM", BusKind::I2c),
    ("SPIM", BusKind::Spi),
    ("SPI", BusKind::Spi),
    ("USART", BusKind::Uart),
    ("UARTE", BusKind::Uart),
    ("UART", BusKind::Uart),
];

/// `"USART3_TX"` → `Some(BusKind::Uart)`; anything unrecognised → `None`, which
/// the caller turns into a test failure quoting the string.
///
/// Deliberately total and dumb: split at the first `_`, strip the trailing
/// instance digits from the head, and look the remainder up in [`STEMS`]. No
/// substring search, no fallback, no default arm.
fn classify(func: &str) -> Option<BusKind> {
    let head = func.split('_').next()?;
    let stem = head.trim_end_matches(|c: char| c.is_ascii_digit());
    STEMS
        .iter()
        .find(|(s, _)| *s == stem)
        .map(|&(_, kind)| kind)
}

fn root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// Minimal manifest naming only the chip — same shape as
/// `chip_conformance.rs`'s, so both boards measure the same construction path
/// (no external devices, no board_io, nothing a system yaml would add).
fn dummy_manifest(path: &str) -> SystemManifest {
    SystemManifest {
        parts: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "bus-visibility".to_string(),
        chip: path.to_string(),
        external_devices: vec![],
        cosim_models: Vec::new(),
        motor_models: Vec::new(),
        board_io: vec![],
        debug_uart: None,
        wifi_ap: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
    }
}

/// Every chip descriptor on the board, sorted. `ci-fixture-*` is excluded for
/// the same reason `chip_conformance.rs` excludes it: those yamls are
/// deliberately partial arch fixtures, not products, and they would put
/// permanent empty rows on a board that is supposed to read as a fleet gap
/// list.
fn fleet() -> Vec<String> {
    let mut chips: Vec<String> = std::fs::read_dir(root("configs/chips"))
        .expect("configs/chips")
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with(".yaml"))
        .map(|n| n.trim_end_matches(".yaml").to_string())
        .filter(|n| !n.contains("ci-fixture"))
        .collect();
    chips.sort();
    chips
}

/// Build one chip's bus and read back which bus kinds reach a pad.
///
/// Returns the bound function names too, so a failure can quote them: "esp32c3
/// lost SPI" is only actionable next to the list that no longer contains
/// `SPI2_SCK`.
fn measure(chip_name: &str) -> (Vec<BusKind>, Vec<&'static str>) {
    let abs = root(&format!("configs/chips/{chip_name}.yaml"));
    let abs_str = abs.to_string_lossy().to_string();
    let chip =
        ChipDescriptor::from_file(&abs).unwrap_or_else(|e| panic!("{chip_name}: load chip: {e}"));
    let bus = SystemBus::from_config(&chip, &dummy_manifest(&abs_str))
        .unwrap_or_else(|e| panic!("{chip_name}: build bus: {e}"));

    let funcs = bus.bound_pad_functions();
    let mut unclassified: Vec<&str> = Vec::new();
    let mut kinds: Vec<BusKind> = Vec::new();
    for f in &funcs {
        match classify(f) {
            Some(kind) => {
                if !kinds.contains(&kind) {
                    kinds.push(kind);
                }
            }
            // NOT a `continue`. See the module header: swallowing this is how
            // the whole matrix silently empties.
            None => unclassified.push(f),
        }
    }
    assert!(
        unclassified.is_empty(),
        "{chip_name}: {} bound pad function name(s) the bus classifier does not \
         recognise: {:?}\nEvery name a `wire_*_pads` function binds must map to \
         a bus kind, or this scoreboard silently under-reports. Either fix the \
         name to `<STEM><instance>_<SIGNAL>` or add its stem to `STEMS` in \
         crates/core/tests/bus_visibility.rs.",
        unclassified.len(),
        unclassified
    );
    kinds.sort();
    (kinds, funcs)
}

#[test]
fn bus_visibility_ratchet() {
    let chips = fleet();
    assert!(
        !chips.is_empty(),
        "no chip descriptors found under configs/chips — the derivation is broken, \
         not the fleet"
    );

    let mut rows: Vec<(String, Vec<BusKind>, Vec<&'static str>)> = Vec::new();
    for chip in &chips {
        let (kinds, funcs) = measure(chip);
        rows.push((chip.clone(), kinds, funcs));
    }

    // ⚠️ NOT-VACUOUS GUARD. A board of all-dashes is what a broken derivation
    // looks like — a renamed accessor, a `bound_pad_functions` that lost its
    // downcast arms, a `from_config` that stopped calling the wiring pass — and
    // it is indistinguishable, cell by cell, from "this fleet genuinely has no
    // bus visibility". The ratchet below cannot catch it either on a fresh
    // baseline: nothing lost, because nothing was ever there. So refuse to
    // treat an empty matrix as green at all.
    let with_any = rows.iter().filter(|(_, k, _)| !k.is_empty()).count();
    assert!(
        with_any > 0,
        "bus-visibility derivation produced NO bus on ANY of the {} chips. That is \
         a broken derivation, not a fleet-wide gap — check \
         `SystemBus::bound_pad_functions` and the `wire_*_pads` calls in \
         `SystemBus::from_config`.",
        rows.len()
    );

    let mut board = String::from(
        "# Bus Visibility Scoreboard\n\n\
         Generated by `bus_visibility_ratchet` (`crates/core/tests/bus_visibility.rs`).\n\n\
         A ✓ means the engine BINDS that bus's controller wire to at least one pad on \
         this chip, so the in-engine logic analyzer can show its waveform. A — means \
         the traffic still happens, but every probe reads the GPIO output latch instead \
         of the wire: the bus is invisible.\n\n\
         Derived from live `SystemBus::from_config` builds — never hand-edited. Bind a \
         bus by adding a `wire_*_pads` function in `crates/core/src/bus/attach.rs` \
         (reference: `wire_rp2040_uart_pads`) and calling it from `from_config`.\n\n\
         ⚠️ SCOPE: this measures the `from_config` path ONLY. The Xtensa chips are \
         also built programmatically by `configure_xtensa_esp32*`, which registers \
         its peripheral bank in Rust and bypasses the chip yaml, so a — here can \
         still mean \"routed on the builder a real lab uses\". esp32s3 is exactly \
         that today: `configure_xtensa_esp32s3` binds its I²C pads, while this row \
         reads — because `esp32s3.yaml` declares `gpio` as `type: \"declarative\"` \
         rather than the `esp32s3_gpio` factory type, so `from_config` finds no \
         model to route. Closing that gap is a yaml change with its own register \
         risk, tracked separately — do not close it by editing this board.\n\n\
         | Chip | I2C | SPI | UART |\n\
         |------|-----|-----|------|\n",
    );
    for (name, kinds, _) in &rows {
        board.push_str(&format!("| {name} |"));
        for kind in BusKind::ALL {
            board.push_str(if kinds.contains(kind) {
                " ✓ |"
            } else {
                " — |"
            });
        }
        board.push('\n');
    }

    // The board is a COMMITTED artifact, so regenerate it only when explicitly
    // asked and otherwise CHECK it — the same contract `chip_conformance.rs`
    // holds, for the same reason: a generator that overwrites its own
    // expectation is not a gate.
    let board_path = root("docs/coverage/bus-visibility.md");
    let baseline_path = root("docs/coverage/bus-visibility.json");
    let current = serde_json::json!(rows
        .iter()
        .map(|(name, kinds, _)| serde_json::json!({
            "name": name,
            "buses": kinds.iter().map(|k| k.label()).collect::<Vec<_>>(),
        }))
        .collect::<Vec<_>>());

    if std::env::var("UPDATE_BUS_VISIBILITY_BASELINE").is_ok() {
        std::fs::write(&board_path, &board).expect("write bus-visibility board");
        std::fs::write(
            &baseline_path,
            format!("{}\n", serde_json::to_string_pretty(&current).unwrap()),
        )
        .expect("write bus-visibility baseline");
        println!("updated bus-visibility board:    {}", board_path.display());
        println!(
            "updated bus-visibility baseline: {}",
            baseline_path.display()
        );
        return;
    }

    // Ratchet: a chip may GAIN a bus freely; losing one it previously had is a
    // regression, and a chip that vanishes from the board takes every bus it
    // had with it, silently, so that counts as a loss too.
    let baseline: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&baseline_path).unwrap_or_else(|_| {
            panic!(
                "missing {}; create it with UPDATE_BUS_VISIBILITY_BASELINE=1",
                baseline_path.display()
            )
        }),
    )
    .expect("parse bus-visibility baseline");

    let mut failures = Vec::new();
    for base in baseline.as_array().expect("baseline is an array") {
        let name = base
            .get("name")
            .and_then(|n| n.as_str())
            .expect("baseline row has a name");
        let Some((_, kinds, funcs)) = rows.iter().find(|(n, _, _)| n == name) else {
            failures.push(format!(
                "  {name}: on the baseline but no longer on the board (chip renamed or \
                 removed?) — every bus it had is now unmeasurable"
            ));
            continue;
        };
        let had: Vec<&str> = base
            .get("buses")
            .and_then(|b| b.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        for want in had {
            if !kinds.iter().any(|k| k.label() == want) {
                failures.push(format!(
                    "  {name}: LOST {want} visibility (bound pad functions now: {funcs:?})"
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "bus visibility regressed ({} issue(s)):\n{}\n\
         A bus that stops binding to a pad becomes invisible to the logic analyzer \
         while still running — nothing else fails. Restore the `wire_*_pads` binding, \
         or, if the loss is intentional and explained, re-baseline with \
         UPDATE_BUS_VISIBILITY_BASELINE=1.",
        failures.len(),
        failures.join("\n")
    );

    // Doc staleness LAST, deliberately unlike `chip_conformance.rs`, which
    // checks its board first. The two failures have very different value: the
    // ratchet says "rp2040 LOST UART visibility", the staleness check says only
    // "this markdown differs". A lost bus makes BOTH fire, and whichever runs
    // first is the one the developer reads — so the specific one goes first.
    let committed = std::fs::read_to_string(&board_path).unwrap_or_else(|e| {
        panic!(
            "read {}: {e} — regenerate with UPDATE_BUS_VISIBILITY_BASELINE=1",
            board_path.display()
        )
    });
    assert_eq!(
        committed.trim_end(),
        board.trim_end(),
        "docs/coverage/bus-visibility.md is stale — the measured matrix no longer \
         matches the committed one. The ratchet above already passed, so nothing \
         was LOST; regenerate in this commit with \
         `UPDATE_BUS_VISIBILITY_BASELINE=1 cargo test -p labwired-core --test \
         bus_visibility` and review the diff — a new ✓ is coverage the doc has \
         not been told about yet."
    );
}

/// The classifier's own contract, asserted without building anything — so a
/// change to it is caught even on a machine where no chip builds.
#[test]
fn classifier_recognises_the_engine_naming_convention_and_nothing_else() {
    assert_eq!(classify("I2C1_SCL"), Some(BusKind::I2c));
    assert_eq!(classify("I2C0_SDA"), Some(BusKind::I2c));
    assert_eq!(
        classify("I2CEXT0_SDA"),
        Some(BusKind::I2c),
        "ESP32 matrix spelling"
    );
    assert_eq!(classify("SPI0_SCK"), Some(BusKind::Spi));
    assert_eq!(classify("SPI1_CSn"), Some(BusKind::Spi));
    assert_eq!(classify("UART0_TX"), Some(BusKind::Uart));
    assert_eq!(classify("USART3_TX"), Some(BusKind::Uart));

    // Nordic IP-block spellings. `UARTE` and `SPIM` are the ones that would be
    // quietly absorbed by a `starts_with` classifier — `UARTE0_TX` starts with
    // `UART` and `SPIM2_SCK` starts with `SPI` — so they are asserted
    // explicitly, and the near-miss below proves it is the whole stem that
    // matches, not a prefix.
    assert_eq!(classify("TWIM0_SCL"), Some(BusKind::I2c));
    assert_eq!(classify("TWIM1_SDA"), Some(BusKind::I2c));
    assert_eq!(classify("SPIM2_SCK"), Some(BusKind::Spi));
    assert_eq!(classify("SPIM0_MOSI"), Some(BusKind::Spi));
    assert_eq!(classify("UARTE0_TX"), Some(BusKind::Uart));
    assert_eq!(classify("UARTEX0_TX"), None, "near-miss on the Nordic stem");
    assert_eq!(classify("TWIS0_SCL"), None, "the SLAVE IP is not wired");

    // ⚠️ The whole point: a near-miss must be REJECTED, not absorbed. A loose
    // `starts_with` would classify all three of these and hide a rename.
    assert_eq!(classify("I2CFOO0_SCL"), None, "near-miss stem");
    assert_eq!(classify("i2c1_scl"), None, "case matters");
    assert_eq!(classify("SCL"), None, "no stem at all");
    assert_eq!(classify("TIM2_CH1"), None, "not a bus");
}
