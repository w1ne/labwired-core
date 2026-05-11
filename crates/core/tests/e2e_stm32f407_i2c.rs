// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// STM32F407 I²C hardware-oracle replay scaffold.
//
// Conceptually mirrors crates/hw-oracle/ (the Xtensa-side hardware oracle)
// but adapted to STM32 peripheral-register traces: the oracle is a JSON
// timeline of SR1/SR2/DR/CR1/CR2 reads and writes captured from real F407
// silicon during a known firmware run. The simulator replays the same
// firmware-issued register accesses against its modeled I²C peripheral
// and asserts the observed values match silicon — that pins the
// peripheral state machine to silicon ground truth.
//
// This file lands the schema + replay code + a placeholder fixture
// **before** the AHT20 + BMP280 hardware arrives, so the test surface
// exists the moment oracle data can be captured. Populating the fixture
// is the only step gated on hardware.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Cpu, Machine};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;

/// One firmware-issued I²C register access, as captured from silicon
/// (via OpenOCD `mdw` / SWD reads) or replayed against the simulator.
///
/// Captures `expected` for reads (to assert against simulator behavior)
/// and `value` for writes (to drive the simulator into the same state
/// real silicon was in at that step).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TraceEvent {
    /// Firmware wrote `value` to register `offset` (bytes from the I²C
    /// peripheral base). Replays as a `bus.write_u32(base + offset, value)`.
    Write {
        offset: u32,
        value: u32,
        /// Human-readable hint, e.g. "CR1=PE|START".
        note: Option<String>,
    },
    /// Firmware read register `offset`. Replay asserts simulator returned
    /// `expected`. A `mask` may be specified to ignore reserved bits or
    /// volatile-status bits we explicitly don't model byte-for-byte.
    Read {
        offset: u32,
        expected: u32,
        #[serde(default = "default_mask")]
        mask: u32,
        note: Option<String>,
    },
    /// Advance the simulator state machine by ticking the bus N times.
    /// Hardware silicon "advances" via wall-clock; the simulator advances
    /// via explicit ticks. The oracle capture must inject explicit
    /// `tick` markers between operations that depend on state-machine
    /// progress (e.g. after START is set, before SB is expected high).
    Tick {
        count: u32,
        note: Option<String>,
    },
}

fn default_mask() -> u32 {
    0xFFFF_FFFF
}

/// Top-level oracle trace as captured from a single firmware run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OracleTrace {
    pub metadata: TraceMetadata,
    pub events: Vec<TraceEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceMetadata {
    pub chip: String,
    pub scenario: String,
    /// Base address of the I²C peripheral the trace targets, e.g.
    /// `0x40005400` for STM32F407 I²C1.
    pub i2c_base: u32,
    /// Path to the firmware ELF that was running on real silicon when
    /// the trace was captured. Same ELF must drive the simulator side.
    pub firmware_elf: String,
    /// Free-form notes about capture conditions.
    pub captured_at: Option<String>,
    pub openocd_version: Option<String>,
}

impl OracleTrace {
    pub fn load(path: &std::path::Path) -> Result<Self, anyhow::Error> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("read trace {}: {}", path.display(), e))?;
        let trace: Self = serde_json::from_str(&text)
            .map_err(|e| anyhow::anyhow!("parse trace {}: {}", path.display(), e))?;
        Ok(trace)
    }

    /// Replay the trace against a freshly-built simulator. Returns Ok when
    /// every `Read` event's `expected & mask` matches what the simulator
    /// produced.
    pub fn replay(&self, bus: &mut SystemBus) -> Result<(), String> {
        for (step, ev) in self.events.iter().enumerate() {
            match ev {
                TraceEvent::Write { offset, value, .. } => {
                    let addr = self.metadata.i2c_base as u64 + *offset as u64;
                    bus.write_u32(addr, *value).map_err(|e| {
                        format!("step {step}: write to 0x{addr:08X} failed: {e:?}")
                    })?;
                }
                TraceEvent::Read {
                    offset,
                    expected,
                    mask,
                    note,
                } => {
                    let addr = self.metadata.i2c_base as u64 + *offset as u64;
                    let got = bus.read_u32(addr).map_err(|e| {
                        format!("step {step}: read 0x{addr:08X} failed: {e:?}")
                    })?;
                    if (got & mask) != (expected & mask) {
                        return Err(format!(
                            "step {step}: read 0x{addr:08X}: expected 0x{expected:08X} & 0x{mask:08X}, got 0x{got:08X} ({})",
                            note.as_deref().unwrap_or("")
                        ));
                    }
                }
                TraceEvent::Tick { count, .. } => {
                    for _ in 0..*count {
                        let _ = bus.tick_peripherals();
                    }
                }
            }
        }
        Ok(())
    }
}

fn build_f407_bus() -> SystemBus {
    let system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/nucleo-f407-i2c/system.yaml");
    let manifest = SystemManifest::from_file(&system_path).expect("load nucleo-f407-i2c manifest");
    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load f407 chip");
    SystemBus::from_config(&chip, &manifest).expect("build f407 bus")
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/stm32f407")
        .join(name)
}

// ── Scaffolding-only tests (always run) ──────────────────────────────

#[test]
fn trace_schema_round_trips_through_serde() {
    let trace = OracleTrace {
        metadata: TraceMetadata {
            chip: "stm32f407vgt6".to_string(),
            scenario: "schema_roundtrip".to_string(),
            i2c_base: 0x40005400,
            firmware_elf: "irrelevant".to_string(),
            captured_at: None,
            openocd_version: None,
        },
        events: vec![
            TraceEvent::Write {
                offset: 0x00,
                value: 0x0001,
                note: Some("PE".into()),
            },
            TraceEvent::Tick {
                count: 8,
                note: None,
            },
            TraceEvent::Read {
                offset: 0x14,
                expected: 0x0001,
                mask: 0x0001,
                note: Some("SB".into()),
            },
        ],
    };
    let json = serde_json::to_string_pretty(&trace).unwrap();
    let parsed: OracleTrace = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.events.len(), 3);
    assert_eq!(parsed.metadata.i2c_base, 0x40005400);
}

#[test]
fn placeholder_fixture_loads() {
    // Sanity check that the placeholder JSON file parses. Real fixture
    // content is populated when hardware oracle capture lands.
    let path = fixture_path("aht20_chip_id_placeholder.json");
    let trace = OracleTrace::load(&path).expect("placeholder fixture must parse");
    assert_eq!(trace.metadata.chip, "stm32f407vgt6");
    assert_eq!(trace.metadata.i2c_base, 0x40005400);
}

#[test]
fn replay_engine_handles_minimal_write_and_tick() {
    // Build the F407 bus, replay a tiny no-assertion trace (write PE,
    // tick a few times) to prove the replay loop is wired through.
    let mut bus = build_f407_bus();
    let trace = OracleTrace {
        metadata: TraceMetadata {
            chip: "stm32f407vgt6".to_string(),
            scenario: "replay_smoke".to_string(),
            i2c_base: 0x40005400,
            firmware_elf: "irrelevant".to_string(),
            captured_at: None,
            openocd_version: None,
        },
        events: vec![
            TraceEvent::Write {
                offset: 0x00,
                value: 0x0001, // PE
                note: Some("PE enable".into()),
            },
            TraceEvent::Tick {
                count: 4,
                note: None,
            },
        ],
    };
    trace.replay(&mut bus).expect("smoke replay must succeed");
}

// ── Hardware-anchored test (ignored until silicon oracle is captured) ──

/// Replay the AHT20 + BMP280 chip-ID handshake captured from real F407
/// silicon. **Ignored** until hardware lands and the oracle fixture is
/// populated. Run with:
///
/// ```bash
/// cargo test -p labwired-core --test e2e_stm32f407_i2c -- --ignored
/// ```
///
/// Capture procedure for populating the fixture:
/// see `examples/nucleo-f407-i2c/ORACLE_CAPTURE.md`.
#[test]
#[ignore = "needs hardware oracle capture from real F407 silicon"]
fn aht20_bmp280_chip_id_handshake_matches_silicon() {
    let path = fixture_path("aht20_bmp280_chip_id.json");
    let trace = OracleTrace::load(&path).expect("load captured trace");
    let mut bus = build_f407_bus();
    if let Err(msg) = trace.replay(&mut bus) {
        panic!("oracle divergence: {msg}");
    }
}

// ── End-to-end firmware-driven test ───────────────────────────────────

const F407_FIRMWARE_ELF: &str =
    "../../target/thumbv7em-none-eabi/release/nucleo-f407-i2c";

fn ensure_f407_firmware_built() -> PathBuf {
    let elf = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(F407_FIRMWARE_ELF);
    if elf.exists() {
        return elf;
    }
    let example_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/nucleo-f407-i2c");
    let status = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .current_dir(&example_dir)
        .status()
        .expect("invoke cargo build for nucleo-f407-i2c");
    assert!(status.success(), "nucleo-f407-i2c firmware failed to build");
    assert!(
        elf.exists(),
        "firmware ELF not produced at {}",
        elf.display()
    );
    elf
}

/// Load the F407 firmware, run it for enough cycles to exercise both
/// AHT20 and BMP280 I²C transactions, and assert that the bus actually
/// reflected the firmware's activity. This is the simulator-side proof
/// that the runtime-attach machinery is wired all the way through —
/// CPU → I²C peripheral state machine → attached device → back to CPU.
///
/// Specific assertions in priority order:
/// 1. The CPU does not crash or run off into unmapped memory.
/// 2. GPIOA_ODR bit 5 is set at some point — proves firmware reached
///    the success branch where both AHT20 BUSY clears and BMP280
///    chip-ID returns 0x58.
#[test]
fn firmware_drives_aht20_and_bmp280_through_simulator() {
    let elf_path = ensure_f407_firmware_built();
    let system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/nucleo-f407-i2c/system.yaml");
    let manifest = SystemManifest::from_file(&system_path).expect("load f407 manifest");
    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load f407 chip");

    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build f407 bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);

    let image = labwired_loader::load_elf(&elf_path).expect("load f407 elf");
    machine.load_firmware(&image).expect("load firmware");

    // Track whether GPIOA PA5 (LED) was ever set high. The firmware only
    // sets it when both AHT20 BUSY clears and BMP280 chip-ID = 0x58.
    let mut led_was_high = false;
    const GPIOA_ODR: u64 = 0x40020014;
    const I2C1_CR1: u64 = 0x40005400;
    const I2C1_SR1: u64 = 0x40005414;
    const PA5_MASK: u32 = 1 << 5;
    const MAX_CYCLES: u32 = 4_000_000;

    let mut max_odr_seen: u32 = 0;
    let mut last_pc = 0u32;
    let mut stuck_count = 0u32;
    for step in 0..MAX_CYCLES {
        machine
            .step()
            .unwrap_or_else(|e| panic!("simulator crashed at step {step}: {e}"));

        // Sample ODR every step to catch any LED transition.
        let odr = machine.bus.read_u32(GPIOA_ODR).unwrap_or(0);
        if odr > max_odr_seen {
            max_odr_seen = odr;
        }
        if (odr & PA5_MASK) != 0 {
            led_was_high = true;
            break;
        }

        if step % 100_000 == 0 {
            let pc = machine.cpu.get_pc();
            let cr1 = machine.bus.read_u32(I2C1_CR1).unwrap_or(0);
            let sr1 = machine.bus.read_u32(I2C1_SR1).unwrap_or(0);
            eprintln!(
                "step={:>8} pc=0x{:08x} cr1=0x{:04x} sr1=0x{:04x} odr=0x{:04x}",
                step, pc, cr1, sr1, odr
            );
            if pc == last_pc {
                stuck_count += 1;
                if stuck_count >= 3 {
                    eprintln!("PC stuck at 0x{pc:08x} for >300k cycles — bailing");
                    break;
                }
            } else {
                stuck_count = 0;
                last_pc = pc;
            }
        }
    }

    // Diagnostic dump on failure.
    if !led_was_high {
        eprintln!("max ODR seen during run: 0x{:04x}", max_odr_seen);
        eprintln!("  PA5 (LED, success)   set: {}", max_odr_seen & (1 << 5) != 0);
        eprintln!("  PA6 (AHT20 ok diag)  set: {}", max_odr_seen & (1 << 6) != 0);
        eprintln!("  PA7 (BMP280 ok diag) set: {}", max_odr_seen & (1 << 7) != 0);
        // Dump i2c1 peripheral state via downcast.
        let i2c_entry = machine
            .bus
            .peripherals
            .iter()
            .find(|p| p.name == "i2c1")
            .unwrap();
        let snap = i2c_entry.dev.snapshot();
        eprintln!("i2c1 snapshot: {snap}");
    }

    assert!(
        led_was_high,
        "Firmware never set PA5 high — see eprintln trace above."
    );
}
