//! ESP32-S3 reset-state MMIO conformance oracle.
//!
//! Bootstrap (2026-06-17) of a silicon-anchored reset-state check for the
//! ESP32-S3, mirroring `esp32c3_reset_conformance.rs`. The S3 already has a deep
//! peripheral model (35 register models + an Xtensa LX7 JIT) and green sim e2e
//! tests, but until now had NO oracle pinning that model to real S3 silicon.
//!
//! A live ESP32-S3 (dual-core Xtensa LX7, JTAG tap 0x120034e5, USB-JTAG MAC
//! 9C:13:9E:F4:40:C0) was read at `reset halt` over its built-in USB-Serial/JTAG
//! with `openocd-esp32` (`board/esp32s3-builtin.cfg`), `mdw` reads wrapped in tcl
//! `capture {}`. Only ROM-untouched / static config registers that the sim wires
//! in `configs/chips/esp32s3.yaml` are pinned here — peripherals whose state is
//! runtime-dependent at the capture point (the live console UART, clock-gated
//! blocks reading 0x0) are excluded so the oracle is deterministic.
//!
//! Run (no hardware — sim vs the silicon values below):
//!   cargo test -p labwired-hw-oracle --test esp32s3_reset_conformance
//!
//! Re-capture from hardware (S3 on USB-JTAG; pass its serial if >1 ESP board):
//!   OOCD_HOME=... ESP32S3_SERIAL=9C:13:9E:F4:40:C0 \
//!     scripts/hw-oracle/esp32s3_capture_estate.sh /tmp/s3cap

use labwired_core::bus::SystemBus;
use labwired_core::system::xtensa::{configure_xtensa_esp32s3, Esp32s3Opts};
use labwired_core::Bus;

/// Silicon-read reset values (live S3, reset halt). Each must equal the sim's
/// modeled reset value. Curated to static config registers the coded S3 model
/// initialises and that read deterministically at reset (no live-console UART,
/// no clock-gated 0x0 blocks).
const RESET_VALUES: &[(&str, u64, u32)] = &[
    // SYSTIMER — CONF reset 0x46000000 (clock/work-enable defaults), same as C3.
    ("SYSTIMER CONF", 0x6002_3000, 0x4600_0000),
    // I2C0 — controller config/timing block, inactive at reset.
    ("I2C0 TO", 0x6001_300C, 0x0000_0010),
    ("I2C0 FIFO_CONF", 0x6001_3018, 0x0000_408B),
    ("I2C0 SCL_START_HOLD", 0x6001_3040, 0x0000_0008),
    ("I2C0 SCL_RSTART_SETUP", 0x6001_3044, 0x0000_0008),
    ("I2C0 SCL_STOP_HOLD", 0x6001_3048, 0x0000_0008),
    ("I2C0 SCL_STOP_SETUP", 0x6001_304C, 0x0000_0008),
    ("I2C0 FILTER_CFG", 0x6001_3050, 0x0000_0300),
    ("I2C0 CLK_CONF", 0x6001_3054, 0x0020_0000),
];

/// Build the bus exactly as the S3 firmware-execution path does — the path whose
/// fidelity the agent actually relies on. (The declarative `from_config` path
/// falls back to generic ARM peripherals for `type:"i2c"` and does NOT wire the
/// coded S3 models, so it is the wrong thing to pin against silicon.)
fn build_sim_bus() -> SystemBus {
    let mut bus = SystemBus::new();
    configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
    bus
}

#[test]
fn esp32s3_reset_values_match_silicon() {
    let sim = build_sim_bus();
    let mut failures = Vec::new();

    for &(label, addr, expect) in RESET_VALUES {
        match sim.read_u32(addr) {
            Ok(got) if got == expect => {}
            Ok(got) => failures.push(format!(
                "  [DIFF] {label} 0x{addr:08X}: sim=0x{got:08X} silicon=0x{expect:08X}"
            )),
            Err(e) => failures.push(format!("  [FAULT] {label} 0x{addr:08X}: {e:?}")),
        }
    }

    assert!(
        failures.is_empty(),
        "ESP32-S3 reset-state model diverged from silicon in {} of {} register(s):\n{}",
        failures.len(),
        RESET_VALUES.len(),
        failures.join("\n")
    );
}
