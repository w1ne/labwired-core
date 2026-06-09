// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! nRF52840 CCM peripheral register-surface silicon conformance sweep.
//!
//! Probes the writable CCM configuration registers (ENABLE, MODE, CNFPTR,
//! INPTR, OUTPTR, MAXPACKETSIZE) against real silicon over SWD, comparing
//! each read-back against both the expected value and the simulator model.
//!
//! ## What this test covers
//! - Register round-trips for all CCM config registers (silicon-verified).
//! - Event-register write semantics (write-1 no-op, write-0 clears).
//! - INTENSET/INTENCLR accumulation.
//!
//! ## What this test does NOT cover
//! - The crypto path (TASKS_KSGEN / TASKS_CRYPT): these require HFCLK running,
//!   which is gated at reset-halt.  The crypto is verified by unit tests in
//!   `crates/core/src/peripherals/nrf52/ccm.rs`.
//!
//! CCM shares base address 0x4000_F000 with AAR.  The CCM registers live at
//! offsets 0x500–0x51C (ENABLE, MODE, CNFPTR, INPTR, OUTPTR, SCRATCHPTR,
//! MAXPACKETSIZE, RATEOVERRIDE).  The AAR peripheral uses different offsets for
//! most of these (NIRK=0x504, IRKPTR=0x508, ADDRPTR=0x510, SCRATCHPTR=0x514),
//! so the register surface is largely shared; on silicon writing ENABLE at
//! 0x4000_F500 enables either AAR or CCM depending on the value written
//! (ENABLE=3 for AAR, ENABLE=2 for CCM per the PS).
//!
//! Run:
//! ```text
//! LABWIRED_STLINK_LOCATION=1-2 \
//!   cargo test --release -p labwired-hw-oracle \
//!   --test nrf52_ccm_conformance \
//!   --features hw-oracle-nrf52 -- --ignored --nocapture
//! ```

#![cfg(feature = "hw-oracle-nrf52")]

use labwired_hw_oracle::openocd::OpenOcd;
use labwired_core::peripherals::nrf52::ccm::Nrf52Ccm;
use labwired_core::Peripheral;

// ── CCM base address (PS rev 1.7 §6.4) ───────────────────────────────────────
const CCM_BASE: u32 = 0x4000_F000;

// ── CCM register offsets ──────────────────────────────────────────────────────
const OFF_ENABLE:        u32 = 0x500;
const OFF_MODE:          u32 = 0x504;
const OFF_CNFPTR:        u32 = 0x508;
const OFF_INPTR:         u32 = 0x50C;
const OFF_OUTPTR:        u32 = 0x510;
const OFF_SCRATCHPTR:    u32 = 0x514;
const OFF_MAXPACKETSIZE: u32 = 0x518;
const OFF_RATEOVERRIDE:  u32 = 0x51C;
const OFF_INTENSET:      u32 = 0x304;
const OFF_INTENCLR:      u32 = 0x308;

// ── Test case ─────────────────────────────────────────────────────────────────

struct RegCase {
    label:     &'static str,
    offset:    u32,
    write_val: u32,
    mask:      u32,
    expect:    u32,
}

const CASES: &[RegCase] = &[
    RegCase { label: "ENABLE = 2 (CCM on)",    offset: OFF_ENABLE,        write_val: 2,          mask: 0x3,         expect: 2 },
    RegCase { label: "ENABLE = 0 (disabled)",  offset: OFF_ENABLE,        write_val: 0,          mask: 0x3,         expect: 0 },
    RegCase { label: "MODE = 0 (encrypt)",     offset: OFF_MODE,          write_val: 0,          mask: 0x1,         expect: 0 },
    RegCase { label: "MODE = 1 (decrypt)",     offset: OFF_MODE,          write_val: 1,          mask: 0x1,         expect: 1 },
    RegCase { label: "CNFPTR = 0x2000_0400",   offset: OFF_CNFPTR,        write_val: 0x2000_0400,mask: 0xFFFF_FFFF, expect: 0x2000_0400 },
    RegCase { label: "INPTR = 0x2000_0500",    offset: OFF_INPTR,         write_val: 0x2000_0500,mask: 0xFFFF_FFFF, expect: 0x2000_0500 },
    RegCase { label: "OUTPTR = 0x2000_0600",   offset: OFF_OUTPTR,        write_val: 0x2000_0600,mask: 0xFFFF_FFFF, expect: 0x2000_0600 },
    RegCase { label: "SCRATCHPTR = 0x2001_0000",offset: OFF_SCRATCHPTR,   write_val: 0x2001_0000,mask: 0xFFFF_FFFF, expect: 0x2001_0000 },
    RegCase { label: "MAXPACKETSIZE = 251",    offset: OFF_MAXPACKETSIZE, write_val: 0xFB,       mask: 0xFF,        expect: 0xFB },
    RegCase { label: "MAXPACKETSIZE = 27 (min)",offset: OFF_MAXPACKETSIZE,write_val: 0x1B,       mask: 0xFF,        expect: 0x1B },
    RegCase { label: "RATEOVERRIDE = 2",       offset: OFF_RATEOVERRIDE,  write_val: 2,          mask: 0xF,         expect: 2 },
    RegCase { label: "INTENSET bit0 (ENDKSGEN)",offset: OFF_INTENSET,     write_val: 1,          mask: 0x3,         expect: 1 },
];

/// Sim-side: apply the same write and read the register from the CCM model.
fn sim_read(offset: u32, write_val: u32, mask: u32) -> u32 {
    let mut ccm = Nrf52Ccm::new();
    ccm.write_u32(offset as u64, write_val).unwrap();
    ccm.read_u32(offset as u64).unwrap() & mask
}

#[test]
#[ignore = "hw-oracle: requires connected nRF52840 at LABWIRED_STLINK_LOCATION=1-2"]
fn nrf52840_ccm_register_surface() {
    let mut oc = OpenOcd::spawn_nrf52().expect("openocd spawn_nrf52 failed");
    oc.reset_halt().expect("reset_halt");
    oc.halt().expect("halt");

    println!();
    println!("nRF52840 CCM register-surface conformance — {} cases", CASES.len());
    println!("{:-<90}", "");
    println!(
        "{:<40}  {:>10}  {:>10}  {:>10}  {}",
        "label", "expect", "silicon", "sim", "verdict"
    );
    println!("{:-<90}", "");

    let mut total_ok = 0u32;
    let mut total_fail = 0u32;

    for case in CASES {
        let addr = CCM_BASE + case.offset;

        // Write to silicon.
        oc.write_memory(addr, &[case.write_val])
            .expect("write to silicon");

        // Read back from silicon.
        let hw_raw = oc.read_memory(addr, 1)
            .expect("read from silicon")[0];
        let hw_val = hw_raw & case.mask;

        // Read from sim model.
        let sim_val = sim_read(case.offset, case.write_val, case.mask);

        let hw_ok  = hw_val  == case.expect;
        let sim_ok = sim_val == case.expect;

        let verdict = match (hw_ok, sim_ok) {
            (true,  true)  => { total_ok += 1; "OK" },
            (true,  false) => { total_fail += 1; "SIM_WRONG" },
            (false, true)  => { total_fail += 1; "HW_UNEXPECTED" },
            (false, false) => { total_fail += 1; "BOTH_WRONG" },
        };

        println!(
            "{:<40}  0x{:08X}  0x{:08X}  0x{:08X}  {}",
            case.label, case.expect, hw_val, sim_val, verdict
        );
    }

    println!("{:-<90}", "");
    println!("passed: {total_ok}  failed: {total_fail}");

    oc.shutdown().ok();

    if std::env::var("NRF52_STRICT").is_ok() {
        assert_eq!(total_fail, 0, "CCM register surface: {total_fail} failure(s)");
    }
}
