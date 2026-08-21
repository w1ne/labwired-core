// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Standardized per-chip conformance scoreboard + ratchet.
//!
//! ONE uniform battery, run for EVERY chip, so coverage is comparable across the
//! fleet and can never silently regress. This sits *on top of* the existing
//! mechanisms rather than replacing them:
//!
//!   * **Estate** (all chips, always): the chip descriptor loads and every wired
//!     peripheral window is reachable (a read at its base faults nowhere).
//!   * **Registers vs silicon** (chips with a committed capture): the fraction of
//!     a real-silicon reset capture (`reg_oracle.json`) the sim reproduces. The
//!     deep per-register gate stays in `*_reset_conformance` / `register_coverage`;
//!     here we track the headline match% so it can't drop.
//!   * **Behavior** (chips with a golden firmware): whether a running-firmware
//!     gate exists (`firmware_survival` / `*_exec_oracle`), which boots real FW
//!     and asserts its register/IO effects. The named gate is **resolved against
//!     the tree** (`resolve_behavior_gate`) — a chip cannot be promoted on a
//!     string that names no test.
//!
//! The board is written to `docs/coverage/chip-conformance.md`; the ratchet
//! baseline is `docs/coverage/chip-conformance.json`. A chip's estate must stay
//! green, its reg-match% may not fall, and a present behavior gate may not vanish.
//! Re-baseline (after a deliberate, explained change):
//!   UPDATE_CONFORMANCE_BASELINE=1 cargo test -p labwired-core --test chip_conformance -- --nocapture
//!
//! "Are the gates enough?" is now a number per chip on the board — and missing
//! coverage is a visible red cell, not a silent gap.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;
use std::path::PathBuf;

/// One chip's conformance inputs. `reset_oracle` and `behavior_gate` are `None`
/// until that coverage exists — the scoreboard then shows the gap.
struct ChipConf {
    name: &'static str,
    yaml: &'static str,
    /// Committed real-silicon reset capture (schema labwired-hw-oracle/*-regs).
    reset_oracle: Option<&'static str>,
    /// The running-firmware gate that asserts this chip's behavior, as
    /// `"<test target>"` or `"<test target>::<test fn>"`.
    ///
    /// This is NOT a free-form label. Every value here is **resolved** against
    /// the tree by [`resolve_behavior_gate`]: the named `crates/*/tests/
    /// <target>.rs` must exist and, when a function is named, that function must
    /// exist there carrying a `#[…test…]` attribute. An unresolvable string is a
    /// hard failure, not a silently-honoured claim.
    behavior_gate: Option<&'static str>,
}

/// The fleet. Every chip with a descriptor MUST appear here (enforced below), so
/// a new chip can't be added without landing on the board.
const CHIPS: &[ChipConf] = &[
    ChipConf {
        name: "esp32c3",
        yaml: "configs/chips/esp32c3.yaml",
        reset_oracle: Some("scripts/hw-oracle/captures/esp32c3/20260611T161223Z/reg_oracle.json"),
        behavior_gate: Some("firmware_survival::test_esp32c3_demo_survival"),
    },
    ChipConf {
        name: "nrf54l15",
        yaml: "configs/chips/nrf54l15.yaml",
        // No silicon capture: nothing here has been diffed against a real
        // nRF54L15 over SWD. Every register value is MDK/SVD-derived, which is
        // authoritative for the map but is not the same as measured silicon.
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_nrf54l15_zephyr_survival"),
    },
    ChipConf {
        name: "esp32",
        yaml: "configs/chips/esp32.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "esp32s3",
        yaml: "configs/chips/esp32s3.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "esp32s3-zero",
        yaml: "configs/chips/esp32s3-zero.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32f401cdu6",
        yaml: "configs/chips/stm32f401cdu6.yaml",
        reset_oracle: None,
        // Was `Some("onboarding-stm32f401cdu6")`, which resolved to no test
        // anywhere in the tree. The nearest real thing is the *CI lane*
        // `onboarding-stm32f401cdu6` in .github/workflows/core-onboarding-smoke.yml
        // — a matrix job that runs `scripts/onboarding_smoke.sh` on push-to-main
        // and on a schedule, NOT on pull requests (see validation/bus_proof_matrix.json).
        // A lane that does not run on PRs cannot be the thing that holds this
        // chip's level up, and this harness has no way to resolve it, so the
        // claim is withdrawn: promote again only via a real firmware_survival /
        // exec-oracle case that `resolve_behavior_gate` can find.
        behavior_gate: None,
    },
    ChipConf {
        // WeAct F411 Black Pill. Sim-derived from ST's CMSIS header + the modm
        // F411 SVD; there is no bench part, so no reset_oracle.
        //
        // Was `Some("tier1::stm32f411")`. `crates/validation-report/tests/tier1.rs`
        // is a real test target, but it contains no `stm32f411` case at all — it
        // tests the validation-report renderer. The comment here used to claim
        // "asserted by the tier-1 fixture self-tests (clock/gpio/timer/i2c/spi/
        // adc/wdt/rtc PASS + UART)"; nothing in the tree asserted that for this
        // chip. Claim withdrawn until a resolvable running-firmware gate lands.
        name: "stm32f411ceu6",
        yaml: "configs/chips/stm32f411ceu6.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "nrf52832",
        yaml: "configs/chips/nrf52832.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_nrf52832_demo_survival"),
    },
    ChipConf {
        name: "nrf52840",
        yaml: "configs/chips/nrf52840.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_nrf52840_demo_survival"),
    },
    ChipConf {
        name: "nrf5340",
        yaml: "configs/chips/nrf5340.yaml",
        reset_oracle: None,
        // Behaviour gate is the real unmodified Zephyr v3.7 hello_world boot on
        // the application core (Cortex-M33). The ELF-independent twin that
        // replays the boot clock/SCS poll loops is tests/nrf5340_clock_boot.rs.
        behavior_gate: Some("firmware_survival::test_nrf5340_zephyr_survival"),
    },
    ChipConf {
        name: "rp2040",
        yaml: "configs/chips/rp2040.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_rp2040_demo_survival"),
    },
    ChipConf {
        name: "stm32f103",
        yaml: "configs/chips/stm32f103.yaml",
        reset_oracle: None,
        behavior_gate: Some("stm32f1_exec_oracle"),
    },
    ChipConf {
        name: "stm32f401",
        yaml: "configs/chips/stm32f401.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_stm32f401_blinky_survival"),
    },
    ChipConf {
        name: "stm32f405",
        yaml: "configs/chips/stm32f405.yaml",
        reset_oracle: None,
        // Smoke-validated via the feather-f405 example (cli lane), not a
        // firmware_survival case of its own yet.
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32f767",
        yaml: "configs/chips/stm32f767.yaml",
        reset_oracle: None,
        // Smoke-validated via the nucleo-f767zi example (cli lane), not a
        // firmware_survival case of its own yet.
        behavior_gate: None,
    },
    ChipConf {
        name: "rp2350",
        yaml: "configs/chips/rp2350.yaml",
        reset_oracle: None,
        // Smoke-validated via the pico2 example (cli lane), not a
        // firmware_survival case of its own yet.
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32f407",
        yaml: "configs/chips/stm32f407.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_nucleo_f407_smoke_survival"),
    },
    ChipConf {
        name: "stm32g474re",
        yaml: "configs/chips/stm32g474re.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32h563",
        yaml: "configs/chips/stm32h563.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_stm32h563_demo_survival"),
    },
    ChipConf {
        // First Cortex-M7 chip. Sim-derived (RM0468); no silicon capture, so no
        // reset_oracle.
        //
        // Was `Some("tier1::stm32h735")` with the comment "behaviour asserted by
        // the tier-1 fixture self-tests". Same story as stm32f411 above: the
        // `tier1` test target exists but has no stm32h735 case, so nothing ran.
        // This chip is separately known to fail a real hosted compile, which the
        // fictional gate did nothing to surface. Claim withdrawn.
        name: "stm32h735",
        yaml: "configs/chips/stm32h735.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32l073",
        yaml: "configs/chips/stm32l073.yaml",
        reset_oracle: Some("scripts/hw-oracle/captures/stm32l073/reg_oracle.json"),
        behavior_gate: Some("firmware_survival::test_nucleo_l073rz_smoke_survival"),
    },
    ChipConf {
        name: "stm32l476",
        yaml: "configs/chips/stm32l476.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_nucleo_l476rg_demo_survival"),
    },
    ChipConf {
        name: "stm32wb55",
        yaml: "configs/chips/stm32wb55.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32wba52",
        yaml: "configs/chips/stm32wba52.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    // NXP KW41Z (Cortex-M0+ BLE + 802.15.4). Register surface ingested from the
    // public CMSIS-SVD; radio (BTLE_RF/GENFSK/ZLL/XCVR) not yet modelled. The
    // behavior gate boots bare-metal firmware that prints over LPUART0.
    ChipConf {
        name: "mkw41z4",
        yaml: "configs/chips/mkw41z4.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_kw41z_smoke_survival"),
    },
    // Silicon Labs EFR32MG26 (Series-2, Cortex-M33). Register surface from the
    // simplicity_sdk CMSIS headers (no public SVD exists); L1 smoke only —
    // Real silicon capture, taken over SWD from a BRD2709A on 2026-08-21 —
    // the SECOND chip in this table to have one, after esp32c3. CMU, GPIO,
    // TIMER0/1, USART0, IADC0 and I2C0 are modelled from the vendor CMSIS
    // headers (Silicon Labs publishes no SVD for this family), and this is the
    // capture that says the model agrees with the die.
    //
    // ⚠️ The capture state is `reset_halt+preamble`, not pure reset_halt, and
    // that is a property of the silicon rather than a shortcut: a Series-2
    // peripheral that is not clocked does not read as zero over the debug port,
    // it FAULTS, and openocd abandons the rest of its command list. A bare
    // `reset halt` capture returns the CMU window and dies at GPIO. The
    // preamble writes CLKEN0 and nothing else, so every register below is
    // still its reset value.
    ChipConf {
        name: "efr32mg26",
        yaml: "configs/chips/efr32mg26.yaml",
        reset_oracle: Some("scripts/hw-oracle/captures/efr32mg26/20260821T163632Z/reg_oracle.json"),
        behavior_gate: None,
    },
    // Classic Arduino Nano / ATmega328P — sim-smoke twin (PORT/Timer0/USART0).
    // Behavior: PlatformIO nanoatmega328 golden (serial nano-ok + D13 toggle).
    // No silicon SWD capture yet → stays below L2 reg-match.
    ChipConf {
        name: "atmega328p",
        yaml: "configs/chips/atmega328p.yaml",
        reset_oracle: None,
        behavior_gate: Some("avr_nano_golden_survival::arduino_nano_golden_prints_and_blinks"),
    },
];

/// Registers a cold-reset sim model can *never* reproduce from a `reset_halt`
/// silicon capture, with the reason. These are excluded from the match% so the
/// headline measures real cold-reset fidelity, not warm-capture overlap. Any
/// mismatch *outside* these ranges is a genuine model gap and stays counted.
///
/// Inclusive `(start, end, reason)` address ranges, per chip.
fn dynamic_excludes(name: &str) -> &'static [(u64, u64, &'static str)] {
    match name {
        "stm32l073" => &[
            (
                0x40021004,
                0x40021004,
                "RCC_ICSCR: per-die HSI16 factory calibration",
            ),
            (
                0x40021008,
                0x40021008,
                "RCC: clock tree configured before reset_halt (warm)",
            ),
            (
                0x40021030,
                0x40021030,
                "RCC: warm peripheral-enable / clock-ready state",
            ),
            (
                0x4002103c,
                0x4002103c,
                "RCC_CSR: reset-cause flags latched by power-on",
            ),
            (
                0x4000002c,
                0x4000002c,
                "TIM2_ARR: timer clock-gated at reset (APB1 off → reads 0)",
            ),
        ],
        "esp32c3" => &[
            (
                0x60000000,
                0x6000007f,
                "UART0: ROM-driven boot console — warm post-bootloader state",
            ),
            (
                0x60004038,
                0x6000403f,
                "GPIO: strapping / input pin levels (board state)",
            ),
            (
                0x60008018,
                0x60008018,
                "RTC_CNTL: dynamic reset/clock state",
            ),
            (0x60008038, 0x60008038, "RTC_CNTL: dynamic clock state"),
            (0x60008044, 0x60008044, "RTC_CNTL: dynamic state"),
            (0x60008090, 0x60008090, "RTC_CNTL: dynamic timer state"),
            (
                0x600080a8,
                0x600080b3,
                "RTC_CNTL: per-die analog calibration",
            ),
            (
                0x600080bc,
                0x600080cf,
                "RTC_CNTL: XTAL / sensor calibration",
            ),
            (
                0x60009004,
                0x6000903f,
                "IO_MUX: ROM reconfigured pad pull-up/drive per function (warm)",
            ),
            (
                0x60016000,
                0x600160ff,
                "RMT: clock-gated at reset (silicon reads 0)",
            ),
            (0x6001301c, 0x6001301c, "I2C0: dynamic bus/FSM status"),
            (0x600c0040, 0x600c00ff, "SYSTEM: dynamic status"),
            (
                0x600c2040,
                0x600c20ff,
                "INTERRUPT_CORE: dynamic pending/status",
            ),
        ],
        // ⚠️ ONE entry, and it is not the chip.
        //
        // The first pass at this capture excluded ten registers as
        // "undocumented" or "warm". That was the wrong instinct. A register
        // the vendor header calls RESERVED still has a value on the die, and a
        // twin that answers 0 where silicon answers 0xC00000BC is wrong
        // whether or not anybody wrote the answer down. Nine of the ten are
        // now MODELLED at their measured values — CMU +0x40 and SYSCLKCTRL,
        // the five undocumented IADC words at +0x30..0x40, and IADC +0x90 —
        // and the header-documented gaps the pass had lumped in with them
        // (IADC CFG/SCALE/FIFOCFG, TIMER/USART/I2C IPVERSION, USART
        // FRAME/STATUS/IF, TIMER TOP/TOPB) were fixed, not excluded.
        //
        // What is left is the one value the die reported that the DIE did not
        // author.
        "efr32mg26" => &[(
            0x4003c044,
            0x4003c044,
            "GPIO PORTA_DIN: the probe's own footprint. PA1 reads high with its \
             port mode DISABLED, while PB0/PB1 — buttons, pulled up, also \
             DISABLED — read 0 on the same board, so 'disabled reads 0' is the \
             rule this one pin breaks. GPIO_DBGROUTEPEN enables the debug pins \
             out of reset and they override the port mode; a J-Link was \
             clocking SWD throughout the capture. Not asserted as PA1=SWCLK \
             without the UG594 pinout in hand. Either way DIN is a pad read, \
             not a reset value: the model reproduces the mechanism, and no \
             probe is attached to the twin",
        )],
        _ => &[],
    }
}

fn excluded_reason(name: &str, addr: u64) -> Option<&'static str> {
    dynamic_excludes(name)
        .iter()
        .find(|(lo, hi, _)| addr >= *lo && addr <= *hi)
        .map(|(_, _, why)| *why)
}

fn root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

// ---------------------------------------------------------------------------
// behavior_gate resolution
//
// `behavior_gate` used to be a hand-typed string that nothing ever opened. Its
// only consumers were `behavior: c.behavior_gate.is_some()` — which feeds
// `level()` and so the committed ratchet baseline — and a `println!` into the
// scoreboard. Three of the seventeen values named a test that does not exist
// (`onboarding-stm32f401cdu6`, `tier1::stm32f411`, `tier1::stm32h735`), so three
// chips were carrying a behaviour claim, and a frozen ratchet floor, on nothing.
// A typo, a renamed test, or a deleted test would all have kept passing.
//
// The strings are now resolved against the tree, and every failure to resolve is
// a hard error. Deleting `firmware_survival::test_esp32c3_demo_survival` (or just
// renaming it) now turns this test red instead of silently keeping esp32c3 at L2.
// ---------------------------------------------------------------------------

/// Where a resolved gate lives: the test source file, and the test function in
/// it when the gate named one.
#[derive(Debug)]
struct GateTarget {
    /// Repo-relative path of the test target source.
    source: String,
}

/// Resolve a `behavior_gate` string to a real test in this repository.
///
/// Accepted forms:
///   * `"<target>"` — `crates/*/tests/<target>.rs` must exist and declare at
///     least one test function.
///   * `"<target>::<fn>"` — that file must additionally declare `fn <fn>` under
///     a test attribute (`#[test]`, `#[tokio::test]`, `#[thumb_oracle_test]`, …).
///
/// Returns `Err` with a human-readable reason when the gate names nothing.
fn resolve_behavior_gate(gate: &str) -> Result<GateTarget, String> {
    let (target, func) = match gate.split_once("::") {
        Some((t, f)) => (t, Some(f)),
        None => (gate, None),
    };
    if target.is_empty() || func == Some("") {
        return Err(format!(
            "malformed gate `{gate}`: expected `target` or `target::test_fn`"
        ));
    }

    // Locate crates/<pkg>/tests/<target>.rs. Integration-test target names are
    // the file stem, so this is the same mapping cargo uses.
    let mut candidates: Vec<PathBuf> = Vec::new();
    let crates_dir = root("crates");
    let mut pkgs: Vec<PathBuf> = std::fs::read_dir(&crates_dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", crates_dir.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    pkgs.sort();
    for pkg in pkgs {
        let p = pkg.join("tests").join(format!("{target}.rs"));
        if p.is_file() {
            candidates.push(p);
        }
    }
    let Some(path) = candidates.first() else {
        return Err(format!(
            "`{gate}`: no test target `crates/*/tests/{target}.rs` exists in this tree"
        ));
    };
    // Two crates with the same test-target name would make "which test does this
    // gate mean" a coin flip resolved by directory order. Say so instead.
    if candidates.len() > 1 {
        return Err(format!(
            "`{gate}`: ambiguous — {} crates declare a test target `{target}`: {:?}. \
             Qualify the gate or rename one of them.",
            candidates.len(),
            candidates
        ));
    }
    let rel = path
        .strip_prefix(root(""))
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string_lossy().to_string());
    let src = std::fs::read_to_string(path)
        .map_err(|e| format!("`{gate}`: read {}: {e}", path.display()))?;

    match func {
        None => {
            if !src.lines().any(is_test_attr) {
                return Err(format!(
                    "`{gate}`: {rel} exists but declares no test function"
                ));
            }
        }
        Some(f) => {
            if !declares_test_fn(&src, f) {
                return Err(format!(
                    "`{gate}`: {rel} exists but declares no test function `{f}`"
                ));
            }
        }
    }
    Ok(GateTarget { source: rel })
}

/// A line that is a test-ish attribute: `#[test]`, `#[tokio::test]`,
/// `#[thumb_oracle_test]`, `#[rstest]`, …
fn is_test_attr(line: &str) -> bool {
    let t = line.trim();
    t.starts_with("#[") && t.contains("test")
}

/// True when `src` declares `fn name(` (or `async fn name(`) with a test
/// attribute in the attribute/doc-comment block immediately above it.
fn declares_test_fn(src: &str, name: &str) -> bool {
    let lines: Vec<&str> = src.lines().collect();
    let sig = format!("fn {name}(");
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        let is_decl = t.starts_with(&sig)
            || t.starts_with(&format!("pub {sig}"))
            || t.starts_with(&format!("async {sig}"))
            || t.starts_with(&format!("pub async {sig}"));
        if !is_decl {
            continue;
        }
        // Walk back over the contiguous attribute / comment / blank block.
        let mut j = i;
        while j > 0 {
            j -= 1;
            let p = lines[j].trim();
            if is_test_attr(p) {
                return true;
            }
            if p.starts_with("#[") || p.starts_with("//") || p.is_empty() {
                continue;
            }
            break;
        }
    }
    false
}

/// The gate for one chip, resolved. Panics (hard, on the ratchet's own path)
/// when the chip claims a gate that names nothing.
fn behavior_gate_target(c: &ChipConf) -> Option<GateTarget> {
    let gate = c.behavior_gate?;
    match resolve_behavior_gate(gate) {
        Ok(t) => Some(t),
        Err(why) => panic!(
            "{}: behavior_gate does not resolve — {why}\n\
             A behavior_gate is a promotion to L2 in `level()` and a frozen floor \
             in docs/coverage/chip-conformance.json. It must name a test that \
             exists: `<target>` or `<target>::<test_fn>` under crates/*/tests/. \
             Either point it at a real gate or set it to None (which demotes the \
             chip — re-baseline with UPDATE_CONFORMANCE_BASELINE=1).",
            c.name
        ),
    }
}

/// Standalone listing of every unresolvable gate, so a reviewer sees all of them
/// at once instead of the first panic. `measure()` enforces the same rule on the
/// ratchet's own path, so deleting this test does not reopen the hole.
#[test]
fn behavior_gates_name_tests_that_exist() {
    let mut bad = Vec::new();
    let mut resolved = Vec::new();
    for c in CHIPS {
        let Some(gate) = c.behavior_gate else {
            continue;
        };
        match resolve_behavior_gate(gate) {
            Ok(t) => resolved.push(format!("  {:<14} {gate} -> {}", c.name, t.source)),
            Err(why) => bad.push(format!("  {:<14} {why}", c.name)),
        }
    }
    println!(
        "behavior gates resolved ({}):\n{}",
        resolved.len(),
        resolved.join("\n")
    );
    assert!(
        bad.is_empty(),
        "{} chip(s) claim a behavior gate that names no test in this tree:\n{}\n\
         A behavior_gate promotes the chip in `level()`; an unresolvable one is a \
         claim resting on a string. Point it at a real test or set it to None.",
        bad.len(),
        bad.join("\n")
    );
}

/// A positive control for the resolver: if `resolve_behavior_gate` degenerated
/// into "always Ok", this test would go red. Pairs with the negative controls in
/// the test above (which only fires when a bad gate is present in CHIPS).
#[test]
fn behavior_gate_resolver_rejects_what_does_not_exist() {
    // Positive: a gate that really exists resolves.
    let ok = resolve_behavior_gate("firmware_survival::test_esp32c3_demo_survival")
        .expect("the esp32c3 survival gate must resolve");
    assert!(
        ok.source
            .ends_with("crates/core/tests/firmware_survival.rs"),
        "resolved to an unexpected file: {}",
        ok.source
    );
    assert!(resolve_behavior_gate("stm32f1_exec_oracle").is_ok());

    // Negative: no such test target.
    assert!(resolve_behavior_gate("onboarding-stm32f401cdu6").is_err());
    assert!(resolve_behavior_gate("no_such_test_target_at_all").is_err());
    // Negative: the target exists, the function does not — the exact shape of
    // the `tier1::stm32f411` / `tier1::stm32h735` claims this PR withdrew.
    assert!(resolve_behavior_gate("tier1::stm32f411").is_err());
    assert!(resolve_behavior_gate("firmware_survival::no_such_case").is_err());
    // Negative: a real *non-test* function in a test file is not a gate.
    assert!(resolve_behavior_gate("firmware_survival::workspace_root").is_err());
}

fn dummy_manifest(path: &str) -> SystemManifest {
    SystemManifest {
        parts: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "chip-conformance".to_string(),
        chip: path.to_string(),
        external_devices: vec![],
        cosim_models: Vec::new(),
        motor_models: Vec::new(),
        board_io: vec![],
        debug_uart: None,
        wifi_ap: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
        // No override: these harnesses take whatever the chip declares.
        cpu_hz: None,
    }
}

#[derive(Debug, Clone)]
struct Record {
    estate_ok: bool,
    peripherals: usize,
    /// Verifiable (deterministic cold-reset) registers in the capture.
    reg_total: usize,
    /// Verifiable registers the sim reproduces exactly.
    reg_match: usize,
    /// Registers excluded as physically un-reproducible (calibration, gated,
    /// warm-configured, live status) — see `dynamic_excludes`.
    excluded: usize,
    behavior: bool,
}

/// Run the uniform battery for one chip.
fn measure(c: &ChipConf) -> Record {
    let abs = root(c.yaml);
    let abs_str = abs.to_string_lossy().to_string();
    let chip =
        ChipDescriptor::from_file(&abs).unwrap_or_else(|e| panic!("{}: load chip: {e}", c.name));
    let peripherals = chip.peripherals.len();
    let bus = SystemBus::from_config(&chip, &dummy_manifest(&abs_str))
        .unwrap_or_else(|e| panic!("{}: build bus: {e}", c.name));

    // Estate: every wired peripheral's base reads without a bus fault.
    let estate_ok = chip
        .peripherals
        .iter()
        .all(|p| bus.read_u32(p.base_address).is_ok());

    // Registers vs silicon: how much of the deterministic cold-reset state the
    // sim reproduces. Registers a cold model physically can't reproduce from a
    // warm `reset_halt` capture (calibration, gated, console, live status) are
    // excluded with a reason; everything else is verifiable, and a mismatch
    // there is a real model gap.
    let report = std::env::var("CONFORMANCE_REPORT").is_ok();
    let (mut reg_total, mut reg_match, mut excluded) = (0usize, 0usize, 0usize);
    if let Some(oracle) = c.reset_oracle {
        if let Ok(text) = std::fs::read_to_string(root(oracle)) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                // ⚠️ REPLAY THE CAPTURE'S PREAMBLE INTO THE SIM.
                //
                // A capture may have had to write registers before it could
                // read anything — on EFR32 a peripheral that is not clocked
                // FAULTS the debug port rather than reading zero, so its oracle
                // un-gates CLKEN0 first. Diffing that against a sim still in
                // its cold reset state compares an un-gated die with a gated
                // model, and every clock-gated peripheral reads 0 in the sim
                // and non-zero on silicon. That is the gating working, not a
                // model gap, and counting it as one buries the real gaps in
                // noise.
                //
                // So the sim is put into the SAME state the capture was taken
                // in. Nothing else about the comparison changes.
                let mut bus = bus;
                if let Some(pre) = json.get("preamble").and_then(|p| p.as_array()) {
                    for step in pre {
                        let Some(w) = step.get("write32") else {
                            continue;
                        };
                        let addr = w.get("address").and_then(|a| a.as_str()).map(parse_hex);
                        let val = w.get("value").and_then(|v| v.as_str()).map(parse_hex32);
                        if let (Some(a), Some(v)) = (addr, val) {
                            let _ = bus.write_u32(a, v);
                        }
                    }
                }
                let bus = bus;
                if let Some(blocks) = json.get("blocks").and_then(|b| b.as_object()) {
                    for block in blocks.values() {
                        if let Some(words) = block.get("words").and_then(|w| w.as_object()) {
                            for (addr, val) in words {
                                let a = parse_hex(addr);
                                let v = val.as_str().map(parse_hex32).unwrap_or(0);
                                if let Some(why) = excluded_reason(c.name, a) {
                                    excluded += 1;
                                    if report {
                                        if let Ok(got) = bus.read_u32(a) {
                                            if got != v {
                                                eprintln!(
                                                    "  [{}] EXCLUDED 0x{a:08x}: sim=0x{got:08x} silicon=0x{v:08x} ({why})",
                                                    c.name
                                                );
                                            }
                                        }
                                    }
                                    continue;
                                }
                                reg_total += 1;
                                match bus.read_u32(a) {
                                    Ok(got) if got == v => reg_match += 1,
                                    Ok(got) => {
                                        if report {
                                            eprintln!(
                                                "  [{}] REAL-GAP 0x{a:08x}: sim=0x{got:08x} silicon=0x{v:08x}",
                                                c.name
                                            );
                                        }
                                    }
                                    Err(_) => {
                                        if report {
                                            eprintln!(
                                                "  [{}] BUS-FAULT 0x{a:08x}: silicon=0x{v:08x} (sim read faulted)",
                                                c.name
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Behavior: the chip has a running-firmware gate AND that gate names a test
    // that actually exists. `behavior_gate_target` panics on an unresolvable
    // string, so a fictional gate can never reach `level()` — this is on the
    // ratchet's own path, not a side test.
    let behavior = behavior_gate_target(c).is_some();

    Record {
        estate_ok,
        peripherals,
        reg_total,
        reg_match,
        excluded,
        behavior,
    }
}

fn parse_hex(s: &str) -> u64 {
    u64::from_str_radix(s.trim().trim_start_matches("0x"), 16).unwrap_or(0)
}
fn parse_hex32(s: &str) -> u32 {
    u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).unwrap_or(0)
}

/// A chip's conformance level: L0 estate, L1 +silicon-registers, L2 +behavior.
fn level(r: &Record) -> u8 {
    if !r.estate_ok {
        return 0;
    }
    let has_reg = r.reg_total > 0 && r.reg_match * 100 >= r.reg_total * 50;
    match (has_reg, r.behavior) {
        (true, true) => 2,
        (true, false) | (false, true) => 1,
        (false, false) => 0,
    }
}

#[test]
fn chip_conformance_ratchet() {
    // Every chip with a descriptor must be on the board.
    let configured: Vec<String> = std::fs::read_dir(root("configs/chips"))
        .expect("configs/chips")
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with(".yaml"))
        .map(|n| n.trim_end_matches(".yaml").to_string())
        .filter(|n| !n.contains("ci-fixture"))
        .collect();
    for chip in &configured {
        assert!(
            CHIPS.iter().any(|c| c.name == chip),
            "chip '{chip}' has a config but is not in the conformance board — add it to CHIPS"
        );
    }

    let mut rows = Vec::new();
    let mut board = String::from(
        "# Chip Conformance Scoreboard\n\n\
         Generated by `chip_conformance_ratchet`. L0 estate · L1 +registers-vs-silicon · L2 +behavior.\n\n\
         Reg match = verifiable cold-reset registers reproduced. \"Excluded\" = registers a cold \
         model can't reproduce from a warm capture (calibration / clock-gated / boot-console / live \
         status); see `dynamic_excludes`. A mismatch outside the excluded set is a real model gap.\n\n\
         | Chip | Level | Estate | Peripherals | Reg match (verifiable) | Excluded | Behavior gate |\n\
         |------|-------|--------|-------------|------------------------|----------|---------------|\n",
    );
    for c in CHIPS {
        let r = measure(c);
        let lvl = level(&r);
        let reg = if r.reg_total > 0 {
            format!(
                "{}/{} ({}%)",
                r.reg_match,
                r.reg_total,
                r.reg_match * 100 / r.reg_total
            )
        } else {
            "—".to_string()
        };
        let exc = if r.excluded > 0 {
            r.excluded.to_string()
        } else {
            "—".to_string()
        };
        let beh = c.behavior_gate.unwrap_or("—");
        board.push_str(&format!(
            "| {} | **L{}** | {} | {} | {} | {} | {} |\n",
            c.name,
            lvl,
            if r.estate_ok { "✓" } else { "✗" },
            r.peripherals,
            reg,
            exc,
            beh,
        ));
        rows.push((c.name.to_string(), lvl, r));
    }

    // The board is a COMMITTED artifact, so regenerate it only when explicitly
    // asked and otherwise CHECK it — the same contract
    // `scripts/generate_validation_status.py --check` holds.
    //
    // This used to be an unconditional `fs::write(..).ok()`: running the test
    // silently rewrote a tracked file and passed, so the committed board could
    // (and did) drift from reality — nrf52832 read 10 verified registers when
    // the model reproduced 16, stm32f407 read 29 against 31. Drift in the
    // OPTIMISTIC direction at that: the doc UNDER-sold the chips, and nothing
    // failed to say so. A generator that overwrites its own expectation cannot
    // be a gate.
    let board_path = root("docs/coverage/chip-conformance.md");
    if std::env::var("UPDATE_CONFORMANCE_BASELINE").is_ok() {
        std::fs::write(&board_path, &board).expect("write conformance board");
        println!("updated conformance board: {}", board_path.display());
    } else {
        let committed = std::fs::read_to_string(&board_path).unwrap_or_else(|e| {
            panic!(
                "read {}: {e} — regenerate with UPDATE_CONFORMANCE_BASELINE=1",
                board_path.display()
            )
        });
        assert_eq!(
            committed.trim_end(),
            board.trim_end(),
            "docs/coverage/chip-conformance.md is stale — the measured board no \
             longer matches the committed one. Regenerate in this commit with \
             `UPDATE_CONFORMANCE_BASELINE=1 cargo test -p labwired-core --test \
             chip_conformance` and review the diff: a FALLING reg-match count is \
             a model regression (the ratchet below fails on it), a RISING one is \
             coverage the doc has not been told about yet."
        );
    }

    // Ratchet against the committed baseline: estate may not break, level may not
    // drop, reg-match count may not fall.
    let baseline_path = root("docs/coverage/chip-conformance.json");
    let current: serde_json::Value = serde_json::json!(rows
        .iter()
        .map(|(name, lvl, r)| {
            serde_json::json!({"name": name, "level": lvl, "reg_match": r.reg_match, "excluded": r.excluded, "behavior": r.behavior})
        })
        .collect::<Vec<_>>());

    if std::env::var("UPDATE_CONFORMANCE_BASELINE").is_ok() {
        std::fs::write(
            &baseline_path,
            serde_json::to_string_pretty(&current).unwrap(),
        )
        .expect("write baseline");
        println!("updated conformance baseline: {}", baseline_path.display());
        return;
    }

    let baseline: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&baseline_path).unwrap_or_else(|_| {
            panic!(
                "missing {}; create it with UPDATE_CONFORMANCE_BASELINE=1",
                baseline_path.display()
            )
        }),
    )
    .expect("parse baseline");

    let mut failures = Vec::new();
    for (name, lvl, r) in &rows {
        let base = baseline.as_array().and_then(|a| {
            a.iter()
                .find(|b| b.get("name").and_then(|n| n.as_str()) == Some(name))
        });
        let Some(base) = base else { continue };
        let base_lvl = base.get("level").and_then(|l| l.as_u64()).unwrap_or(0) as u8;
        let base_match = base.get("reg_match").and_then(|m| m.as_u64()).unwrap_or(0) as usize;
        if *lvl < base_lvl {
            failures.push(format!("  {name}: level L{lvl} < baseline L{base_lvl}"));
        }
        if r.reg_match < base_match {
            failures.push(format!(
                "  {name}: reg match {} < baseline {base_match}",
                r.reg_match
            ));
        }
        if !r.estate_ok {
            failures.push(format!(
                "  {name}: estate broken (a peripheral window faults)"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "chip conformance regressed ({} issue(s)):\n{}\n(intentional? re-baseline with UPDATE_CONFORMANCE_BASELINE=1)",
        failures.len(),
        failures.join("\n")
    );
}
