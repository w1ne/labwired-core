// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! RP2040 reset-state MMIO conformance oracle.
//!
//! The RP2040 ships in the browser catalog (`configs/chips/rp2040.yaml`) but,
//! unlike the STM32/ESP32/nRF families, carried no register-level fidelity
//! coverage — only in-tree unit tests on the individual peripheral models. This
//! oracle closes the reset/boot half of that gap: it builds the full RP2040 sim
//! bus exactly as the runtime does (`SystemBus::from_config`) and pins the
//! **cold-reset** value of every wired block against the RP2040 datasheet
//! (RP2040 Datasheet, release 2.1). Every RP2040 peripheral is walk-only /
//! free-running (`uses_scheduler()` is false for all of them), so the correct
//! fidelity instrument is a deterministic reset/exec oracle, not a
//! walk-vs-scheduler differential (there is no scheduler path to diff against).
//!
//! Two classes of assertion:
//!   1. **Silicon reset values** — registers whose datasheet reset value is
//!      load-bearing (SYSINFO identity that the pico-sdk USB errata workaround
//!      reads, the free-running timer at rest, the PL011/PL022 status flags,
//!      SIO core id). A wrong value here is a real fidelity bug.
//!   2. **Mapping conformance** — every wired window must answer a read without
//!      a bus fault. A fault means the block is mis-mapped in the chip yaml and
//!      firmware would hard-fault the moment it touched it.
//!
//! Sim-only (normal CI); there is no `_hw` variant because the project has no
//! attached RP2040 bench target:
//! ```text
//! cargo test -p labwired-hw-oracle --test rp2040_reset_conformance
//! ```

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;
use std::path::PathBuf;

// ── RP2040 register map (RP2040 Datasheet §2.21, §4.6, §4.4, §4.3, §2.3.1) ──────

// SYSINFO (§2.21) — read-only identity block at 0x40000000.
const SYSINFO_CHIP_ID: u32 = 0x4000_0000;
const SYSINFO_PLATFORM: u32 = 0x4000_0004;
/// CHIP_ID for an RP2040 B2 stepping: REVISION=0x2, PART=0x0002,
/// MANUFACTURER=0x927 (Raspberry Pi). The pico-sdk USB errata-E5 fix, run from
/// the USB ISR on bus reset, asserts MANUFACTURER==0x927 — a zero here aborts
/// enumeration inside the ISR.
const SYSINFO_CHIP_ID_RESET: u32 = 0x2000_2927;
/// PLATFORM with the ASIC bit (bit 1) set and FPGA (bit 0) clear: the sim is a
/// real part, so `running_on_fpga()` must read false.
const SYSINFO_PLATFORM_RESET: u32 = 0x0000_0002;

// TIMER (§4.6) — 64-bit free-running microsecond counter at 0x40054000.
const TIMER_BASE: u32 = 0x4005_4000;
const TIMER_ARMED: u32 = TIMER_BASE + 0x20; // alarm armed bits
const TIMER_TIMERAWH: u32 = TIMER_BASE + 0x24; // live high word
const TIMER_TIMERAWL: u32 = TIMER_BASE + 0x28; // live low word
const TIMER_PAUSE: u32 = TIMER_BASE + 0x30; // freeze control
const TIMER_INTR: u32 = TIMER_BASE + 0x34; // raw interrupt
const TIMER_INTE: u32 = TIMER_BASE + 0x38; // interrupt enable
const TIMER_INTS: u32 = TIMER_BASE + 0x40; // masked status

// UART0 PL011 (§4.2) — the console at 0x40034000. UARTFR flag register at +0x18.
const UART0_UARTFR: u32 = 0x4003_4018;
/// PL011 UARTFR at reset: TXFE (bit 7) | RXFE (bit 4) — both FIFOs empty. This
/// is the PrimeCell PL011 (DDI 0183G) documented reset flag state.
const UART0_UARTFR_RESET: u32 = 0x0000_0090;

// SPI0 PL022 SSP (§4.4) — at 0x4003c000. Status register SSPSR at +0x0c.
const SPI0_SSPSR: u32 = 0x4003_c00c;
/// PL022 SSPSR at reset: TFE (bit 0, TX FIFO empty) | TNF (bit 1, TX FIFO not
/// full). No RX data, not busy.
const SPI0_SSPSR_RESET: u32 = 0x0000_0003;

// SIO (§2.3.1) — single-cycle IO at 0xD0000000 (outside the APB/AHB window).
const SIO_CPUID: u32 = 0xD000_0000;
const SIO_GPIO_IN: u32 = 0xD000_0004;
const SIO_GPIO_OUT: u32 = 0xD000_0010;
const SIO_GPIO_OE: u32 = 0xD000_0020;

/// (label, address, expected cold-reset value). Datasheet-anchored; a mismatch
/// is a real model-vs-silicon fidelity bug.
const RESET_VALUES: &[(&str, u32, u32)] = &[
    ("SYSINFO CHIP_ID", SYSINFO_CHIP_ID, SYSINFO_CHIP_ID_RESET),
    ("SYSINFO PLATFORM", SYSINFO_PLATFORM, SYSINFO_PLATFORM_RESET),
    // Free-running timer at rest: the counter and all alarm state read zero
    // before any tick advances it.
    ("TIMER TIMERAWL", TIMER_TIMERAWL, 0x0000_0000),
    ("TIMER TIMERAWH", TIMER_TIMERAWH, 0x0000_0000),
    ("TIMER ARMED", TIMER_ARMED, 0x0000_0000),
    ("TIMER PAUSE", TIMER_PAUSE, 0x0000_0000),
    ("TIMER INTR", TIMER_INTR, 0x0000_0000),
    ("TIMER INTE", TIMER_INTE, 0x0000_0000),
    ("TIMER INTS", TIMER_INTS, 0x0000_0000),
    ("UART0 UARTFR", UART0_UARTFR, UART0_UARTFR_RESET),
    ("SPI0 SSPSR", SPI0_SSPSR, SPI0_SSPSR_RESET),
    // SIO: single core context → CPUID reads core 0; no pin driven at reset.
    ("SIO CPUID", SIO_CPUID, 0x0000_0000),
    ("SIO GPIO_IN", SIO_GPIO_IN, 0x0000_0000),
    ("SIO GPIO_OUT", SIO_GPIO_OUT, 0x0000_0000),
    ("SIO GPIO_OE", SIO_GPIO_OE, 0x0000_0000),
];

/// Every wired window must answer a read without a bus fault (proves the chip
/// yaml maps the block). Value is not asserted here — only that the access is
/// serviced rather than faulting the CPU.
const MAPPED_WINDOWS: &[(&str, u32)] = &[
    ("PIO0", 0x5020_0000),
    ("CLK_RST", 0x4000_8000),
    ("ROSC", 0x4006_0000),
    ("WATCHDOG", 0x4005_8000),
    ("I2C0", 0x4004_4000),
    ("XIP_SSI", 0x1800_0000),
    ("USBCTRL regs", 0x5011_0000),
    ("TBMAN", 0x4006_c000),
];

fn build_sim_bus() -> SystemBus {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip_path = manifest_dir.join("../../configs/chips/rp2040.yaml");
    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load chip {chip_path:?}: {e}"));
    let manifest = SystemManifest {
        cosim_models: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "rp2040-reset-conformance".to_string(),
        chip: chip_path.to_string_lossy().to_string(),
        external_devices: vec![],
        board_io: vec![],
        debug_uart: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
    };
    SystemBus::from_config(&chip, &manifest).unwrap_or_else(|e| panic!("build RP2040 sim bus: {e}"))
}

/// Every wired RP2040 block must return its datasheet cold-reset value.
#[test]
fn rp2040_reset_values_match_datasheet() {
    let sim = build_sim_bus();
    let mut failures = Vec::new();

    for &(label, addr, expect) in RESET_VALUES {
        match sim.read_u32(addr as u64) {
            Ok(got) if got == expect => {}
            Ok(got) => failures.push(format!(
                "  [DIFF] {label} 0x{addr:08X}: sim=0x{got:08X} datasheet=0x{expect:08X}"
            )),
            Err(e) => failures.push(format!("  [FAULT] {label} 0x{addr:08X}: {e:?}")),
        }
    }

    assert!(
        failures.is_empty(),
        "RP2040 reset-state model diverged from datasheet in {} of {} register(s):\n{}",
        failures.len(),
        RESET_VALUES.len(),
        failures.join("\n")
    );
}

/// Every wired RP2040 window must be mapped (a read is serviced, not faulted).
#[test]
fn rp2040_wired_windows_are_mapped() {
    let sim = build_sim_bus();
    let mut failures = Vec::new();

    for &(label, addr) in MAPPED_WINDOWS {
        if let Err(e) = sim.read_u32(addr as u64) {
            failures.push(format!("  [FAULT] {label} 0x{addr:08X}: {e:?}"));
        }
    }

    assert!(
        failures.is_empty(),
        "RP2040 has {} unmapped wired window(s) — firmware would hard-fault on first access:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
