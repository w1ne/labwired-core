// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! nRF52840 SPIS0 + TWIS0 register-surface sim-vs-silicon conformance.
//!
//! SPIS0 and TWIS0 share base addresses with the master-mode peripherals
//! that are already declared in the production nrf52840.yaml:
//!
//!   SPIS0 — 0x40003000  (same physical block as SPIM0 / SPI0)
//!   TWIS0 — 0x40003000  (TWIS0 is actually at the same base as SPIM0!)
//!   TWIS1 — 0x40004000  (same physical block as TWIM1 / TWI1)
//!
//! Per PS §6.26 and §6.29 the ENABLE register selects the mode:
//!   ENABLE = 0  → peripheral disabled (neutral; master or slave selectable)
//!   ENABLE = 7  → SPIM (master) mode
//!   ENABLE = 2  → SPIS (slave) mode
//!   ENABLE = 6  → TWIM (master) mode
//!   ENABLE = 9  → TWIS (slave) mode
//!
//! Because base addresses collide, the sim-side is built from standalone
//! model instances rather than the production chip YAML.  The hardware side
//! writes ENABLE into the shared silicon block to select slave mode before
//! probing, then restores ENABLE=0 at teardown.
//!
//! Run (pin the nRF probe when multiple ST-Links are attached):
//! ```text
//! LABWIRED_STLINK_LOCATION=1-2 NRF52_STRICT=1 \
//!   cargo test --release -p labwired-hw-oracle \
//!   --test nrf52_spis_twis_conformance \
//!   --features hw-oracle-nrf52 -- --ignored --nocapture
//! ```

#![cfg(feature = "hw-oracle-nrf52")]

use labwired_core::peripherals::nrf52::spis::Nrf52Spis;
use labwired_core::peripherals::nrf52::twis::Nrf52Twis;
use labwired_core::Peripheral;
use labwired_hw_oracle::openocd::OpenOcd;
use std::sync::Mutex;

// ── Base addresses (nRF52840 PS rev 1.7 §6.1.4) ──────────────────────────────

/// SPIS0 / SPIM0 / SPI0 / TWIS0 / TWIM0 / TWI0 all share 0x40003000.
const SPIS0_BASE: u32 = 0x4000_3000;
/// TWIS1 / TWIM1 / TWI1 share 0x40004000.
const TWIS1_BASE: u32 = 0x4000_4000;

// ── Case definition (mirrors nrf52_timer_rtc_conformance scaffold) ────────────

struct Case {
    label: &'static str,
    /// Base address of the silicon peripheral to use.
    base: u32,
    /// (offset, value) pairs written to BOTH sim and hw before the main write.
    prep: &'static [(u32, u32)],
    /// The (offset, value) write under test.
    write: (u32, u32),
    /// Address to read back (absolute = base + offset — we pass absolute addrs).
    read_addr: u32,
    mask: u32,
    expect: u32,
}

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Match,
    BothDisagreeWithExpect { both: u32 },
    Diverge { sim: u32, hw: u32 },
    SimError(String),
}

static HW_LOCK: Mutex<()> = Mutex::new(());

// ── SPIS0 test cases ──────────────────────────────────────────────────────────
// Enable SPIS by writing ENABLE=2 to the shared block before the test.
// Each case uses SPIS0_BASE as the peripheral base.
// We structure cases as absolute addresses (base + offset).

const SPIS0_ENABLE: u32 = SPIS0_BASE + 0x500;
const SPIS0_PSEL_SCK: u32 = SPIS0_BASE + 0x508;
const SPIS0_PSEL_MISO: u32 = SPIS0_BASE + 0x50C;
const SPIS0_PSEL_MOSI: u32 = SPIS0_BASE + 0x510;
const SPIS0_PSEL_CSN: u32 = SPIS0_BASE + 0x514;
const SPIS0_RXD_PTR: u32 = SPIS0_BASE + 0x534;
const SPIS0_RXD_MAXCNT: u32 = SPIS0_BASE + 0x538;
const SPIS0_TXD_PTR: u32 = SPIS0_BASE + 0x544;
const SPIS0_TXD_MAXCNT: u32 = SPIS0_BASE + 0x548;
const SPIS0_CONFIG: u32 = SPIS0_BASE + 0x554;
const SPIS0_DEF: u32 = SPIS0_BASE + 0x55C;
const SPIS0_ORC: u32 = SPIS0_BASE + 0x5C0;
const SPIS0_INTENSET: u32 = SPIS0_BASE + 0x304;
const SPIS0_INTENCLR: u32 = SPIS0_BASE + 0x308;
const SPIS0_SHORTS: u32 = SPIS0_BASE + 0x200;
const SPIS0_EVENTS_END: u32 = SPIS0_BASE + 0x104;
const SPIS0_EVENTS_ENDRX: u32 = SPIS0_BASE + 0x110;
const SPIS0_EVENTS_ACQUIRED: u32 = SPIS0_BASE + 0x128;
const SPIS0_SEMSTAT: u32 = SPIS0_BASE + 0x400;
#[allow(dead_code)]
const SPIS0_STATUS: u32 = SPIS0_BASE + 0x440;

const TWIS1_ENABLE: u32 = TWIS1_BASE + 0x500;
const TWIS1_PSEL_SCL: u32 = TWIS1_BASE + 0x508;
const TWIS1_PSEL_SDA: u32 = TWIS1_BASE + 0x50C;
const TWIS1_RXD_PTR: u32 = TWIS1_BASE + 0x534;
const TWIS1_RXD_MAXCNT: u32 = TWIS1_BASE + 0x538;
const TWIS1_TXD_PTR: u32 = TWIS1_BASE + 0x544;
const TWIS1_TXD_MAXCNT: u32 = TWIS1_BASE + 0x548;
const TWIS1_ADDRESS0: u32 = TWIS1_BASE + 0x588;
const TWIS1_ADDRESS1: u32 = TWIS1_BASE + 0x58C;
const TWIS1_CONFIG: u32 = TWIS1_BASE + 0x594;
const TWIS1_ORC: u32 = TWIS1_BASE + 0x5C0;
const TWIS1_INTENSET: u32 = TWIS1_BASE + 0x304;
const TWIS1_INTENCLR: u32 = TWIS1_BASE + 0x308;
const TWIS1_EVENTS_STOPPED: u32 = TWIS1_BASE + 0x104;
const TWIS1_EVENTS_ERROR: u32 = TWIS1_BASE + 0x124;
const TWIS1_EVENTS_RXSTARTED: u32 = TWIS1_BASE + 0x14C;
const TWIS1_EVENTS_TXSTARTED: u32 = TWIS1_BASE + 0x150;
const TWIS1_EVENTS_WRITE: u32 = TWIS1_BASE + 0x164;
const TWIS1_EVENTS_READ: u32 = TWIS1_BASE + 0x168;
const TWIS1_ERRORSRC: u32 = TWIS1_BASE + 0x4D0;

// Prep: set ENABLE=2 (SPIS mode) on the shared block.
const SPIS_ENABLE_PREP: &[(u32, u32)] = &[(SPIS0_ENABLE, 2)];
// Prep: set ENABLE=9 (TWIS mode) on the shared block.
const TWIS_ENABLE_PREP: &[(u32, u32)] = &[(TWIS1_ENABLE, 9)];

// INTEN valid mask for SPIS: END(bit1) ENDRX(bit4) ACQUIRED(bit10).
const SPIS_INTEN_MASK: u32 = (1 << 1) | (1 << 4) | (1 << 10);
// INTEN valid mask for TWIS: STOPPED(1) ERROR(9) RXSTARTED(19) TXSTARTED(20) WRITE(25) READ(26).
const TWIS_INTEN_MASK: u32 = (1 << 1) | (1 << 9) | (1 << 19) | (1 << 20) | (1 << 25) | (1 << 26);

const CASES: &[Case] = &[
    // ── SPIS0: ENABLE selector ────────────────────────────────────────────────
    Case {
        label: "SPIS0 ENABLE=2 (SPIS mode)",
        base: SPIS0_BASE,
        prep: &[],
        write: (SPIS0_ENABLE, 2),
        read_addr: SPIS0_ENABLE,
        mask: 0xF,
        expect: 2,
    },
    // ── SPIS0: PSEL registers (R/W full-width; reset = 0xFFFFFFFF) ───────────
    Case {
        label: "SPIS0 PSEL.SCK write/readback",
        base: SPIS0_BASE,
        prep: SPIS_ENABLE_PREP,
        write: (SPIS0_PSEL_SCK, 0x0000_001A),
        read_addr: SPIS0_PSEL_SCK,
        mask: 0xFFFF_FFFF,
        expect: 0x0000_001A,
    },
    Case {
        label: "SPIS0 PSEL.MISO write/readback",
        base: SPIS0_BASE,
        prep: SPIS_ENABLE_PREP,
        write: (SPIS0_PSEL_MISO, 0x0000_001B),
        read_addr: SPIS0_PSEL_MISO,
        mask: 0xFFFF_FFFF,
        expect: 0x0000_001B,
    },
    Case {
        label: "SPIS0 PSEL.MOSI write/readback",
        base: SPIS0_BASE,
        prep: SPIS_ENABLE_PREP,
        write: (SPIS0_PSEL_MOSI, 0x0000_001C),
        read_addr: SPIS0_PSEL_MOSI,
        mask: 0xFFFF_FFFF,
        expect: 0x0000_001C,
    },
    Case {
        label: "SPIS0 PSEL.CSN write/readback",
        base: SPIS0_BASE,
        prep: SPIS_ENABLE_PREP,
        write: (SPIS0_PSEL_CSN, 0x0000_001D),
        read_addr: SPIS0_PSEL_CSN,
        mask: 0xFFFF_FFFF,
        expect: 0x0000_001D,
    },
    // ── SPIS0: RXD/TXD pointers and max-counts ───────────────────────────────
    Case {
        label: "SPIS0 RXD.PTR write/readback",
        base: SPIS0_BASE,
        prep: SPIS_ENABLE_PREP,
        write: (SPIS0_RXD_PTR, 0x2000_1000),
        read_addr: SPIS0_RXD_PTR,
        mask: 0xFFFF_FFFF,
        expect: 0x2000_1000,
    },
    Case {
        label: "SPIS0 RXD.MAXCNT masks to 8 bits",
        base: SPIS0_BASE,
        prep: SPIS_ENABLE_PREP,
        write: (SPIS0_RXD_MAXCNT, 0xFF),
        read_addr: SPIS0_RXD_MAXCNT,
        mask: 0xFF,
        expect: 0xFF,
    },
    Case {
        label: "SPIS0 TXD.PTR write/readback",
        base: SPIS0_BASE,
        prep: SPIS_ENABLE_PREP,
        write: (SPIS0_TXD_PTR, 0x2000_2000),
        read_addr: SPIS0_TXD_PTR,
        mask: 0xFFFF_FFFF,
        expect: 0x2000_2000,
    },
    Case {
        label: "SPIS0 TXD.MAXCNT masks to 8 bits",
        base: SPIS0_BASE,
        prep: SPIS_ENABLE_PREP,
        write: (SPIS0_TXD_MAXCNT, 0x7F),
        read_addr: SPIS0_TXD_MAXCNT,
        mask: 0xFF,
        expect: 0x7F,
    },
    // ── SPIS0: CONFIG (CPHA/CPOL/ORDER — 3 bits) ─────────────────────────────
    Case {
        label: "SPIS0 CONFIG round-trip (3 bits)",
        base: SPIS0_BASE,
        prep: SPIS_ENABLE_PREP,
        write: (SPIS0_CONFIG, 0x7),
        read_addr: SPIS0_CONFIG,
        mask: 0x7,
        expect: 0x7,
    },
    // ── SPIS0: DEF / ORC (8-bit) ─────────────────────────────────────────────
    Case {
        label: "SPIS0 DEF round-trip (8 bits)",
        base: SPIS0_BASE,
        prep: SPIS_ENABLE_PREP,
        write: (SPIS0_DEF, 0xA5),
        read_addr: SPIS0_DEF,
        mask: 0xFF,
        expect: 0xA5,
    },
    Case {
        label: "SPIS0 ORC round-trip (8 bits)",
        base: SPIS0_BASE,
        prep: SPIS_ENABLE_PREP,
        write: (SPIS0_ORC, 0x5A),
        read_addr: SPIS0_ORC,
        mask: 0xFF,
        expect: 0x5A,
    },
    // ── SPIS0: INTENSET/INTENCLR ─────────────────────────────────────────────
    Case {
        label: "SPIS0 INTENSET set valid bits",
        base: SPIS0_BASE,
        // Clear first to start from known state.
        prep: &[(SPIS0_INTENCLR, 0xFFFF_FFFF), (SPIS0_ENABLE, 2)],
        write: (SPIS0_INTENSET, SPIS_INTEN_MASK),
        read_addr: SPIS0_INTENSET,
        mask: SPIS_INTEN_MASK,
        expect: SPIS_INTEN_MASK,
    },
    Case {
        label: "SPIS0 INTENCLR clears all",
        base: SPIS0_BASE,
        prep: &[(SPIS0_INTENSET, SPIS_INTEN_MASK), (SPIS0_ENABLE, 2)],
        write: (SPIS0_INTENCLR, SPIS_INTEN_MASK),
        read_addr: SPIS0_INTENSET,
        mask: SPIS_INTEN_MASK,
        expect: 0,
    },
    // ── SPIS0: EVENTS — SW write-1 ignored (HW-set only) ─────────────────────
    Case {
        label: "SPIS0 EVENTS_END SW-write-1 ignored",
        base: SPIS0_BASE,
        prep: SPIS_ENABLE_PREP,
        write: (SPIS0_EVENTS_END, 1),
        read_addr: SPIS0_EVENTS_END,
        mask: 1,
        expect: 0,
    },
    Case {
        label: "SPIS0 EVENTS_ENDRX SW-write-1 ignored",
        base: SPIS0_BASE,
        prep: SPIS_ENABLE_PREP,
        write: (SPIS0_EVENTS_ENDRX, 1),
        read_addr: SPIS0_EVENTS_ENDRX,
        mask: 1,
        expect: 0,
    },
    Case {
        label: "SPIS0 EVENTS_ACQUIRED SW-write-1 ignored",
        base: SPIS0_BASE,
        prep: SPIS_ENABLE_PREP,
        write: (SPIS0_EVENTS_ACQUIRED, 1),
        read_addr: SPIS0_EVENTS_ACQUIRED,
        mask: 1,
        expect: 0,
    },
    // ── SPIS0: SEMSTAT is RO (write silently discarded) ──────────────────────
    // Silicon reset value is 0x1 (CPU holds semaphore). The write of 0x3
    // must be ignored; readback must still equal the reset value 0x1.
    Case {
        label: "SPIS0 SEMSTAT read-only (write ignored, reset=0x1)",
        base: SPIS0_BASE,
        prep: SPIS_ENABLE_PREP,
        write: (SPIS0_SEMSTAT, 0x3),
        read_addr: SPIS0_SEMSTAT,
        mask: 0x3,
        expect: 0x1,
    },
    // ── SPIS0: SHORTS ─────────────────────────────────────────────────────────
    Case {
        label: "SPIS0 SHORTS END_ACQUIRE bit2",
        base: SPIS0_BASE,
        prep: SPIS_ENABLE_PREP,
        write: (SPIS0_SHORTS, 0xFFFF_FFFF),
        read_addr: SPIS0_SHORTS,
        mask: 0x0000_0004,
        expect: 0x0000_0004,
    },

    // ── TWIS1: ENABLE selector ────────────────────────────────────────────────
    Case {
        label: "TWIS1 ENABLE=9 (TWIS mode)",
        base: TWIS1_BASE,
        prep: &[],
        write: (TWIS1_ENABLE, 9),
        read_addr: TWIS1_ENABLE,
        mask: 0xF,
        expect: 9,
    },
    // ── TWIS1: PSEL registers ─────────────────────────────────────────────────
    Case {
        label: "TWIS1 PSEL.SCL write/readback",
        base: TWIS1_BASE,
        prep: TWIS_ENABLE_PREP,
        write: (TWIS1_PSEL_SCL, 0x0000_000B),
        read_addr: TWIS1_PSEL_SCL,
        mask: 0xFFFF_FFFF,
        expect: 0x0000_000B,
    },
    Case {
        label: "TWIS1 PSEL.SDA write/readback",
        base: TWIS1_BASE,
        prep: TWIS_ENABLE_PREP,
        write: (TWIS1_PSEL_SDA, 0x0000_000C),
        read_addr: TWIS1_PSEL_SDA,
        mask: 0xFFFF_FFFF,
        expect: 0x0000_000C,
    },
    // ── TWIS1: RXD/TXD ───────────────────────────────────────────────────────
    Case {
        label: "TWIS1 RXD.PTR write/readback",
        base: TWIS1_BASE,
        prep: TWIS_ENABLE_PREP,
        write: (TWIS1_RXD_PTR, 0x2000_0400),
        read_addr: TWIS1_RXD_PTR,
        mask: 0xFFFF_FFFF,
        expect: 0x2000_0400,
    },
    Case {
        label: "TWIS1 RXD.MAXCNT masks to 8 bits",
        base: TWIS1_BASE,
        prep: TWIS_ENABLE_PREP,
        write: (TWIS1_RXD_MAXCNT, 0x80),
        read_addr: TWIS1_RXD_MAXCNT,
        mask: 0xFF,
        expect: 0x80,
    },
    Case {
        label: "TWIS1 TXD.PTR write/readback",
        base: TWIS1_BASE,
        prep: TWIS_ENABLE_PREP,
        write: (TWIS1_TXD_PTR, 0x2000_0800),
        read_addr: TWIS1_TXD_PTR,
        mask: 0xFFFF_FFFF,
        expect: 0x2000_0800,
    },
    Case {
        label: "TWIS1 TXD.MAXCNT masks to 8 bits",
        base: TWIS1_BASE,
        prep: TWIS_ENABLE_PREP,
        write: (TWIS1_TXD_MAXCNT, 0x40),
        read_addr: TWIS1_TXD_MAXCNT,
        mask: 0xFF,
        expect: 0x40,
    },
    // ── TWIS1: ADDRESS[0]/[1] (7-bit) ────────────────────────────────────────
    Case {
        label: "TWIS1 ADDRESS[0] masks to 7 bits",
        base: TWIS1_BASE,
        prep: TWIS_ENABLE_PREP,
        write: (TWIS1_ADDRESS0, 0x68),
        read_addr: TWIS1_ADDRESS0,
        mask: 0x7F,
        expect: 0x68,
    },
    Case {
        label: "TWIS1 ADDRESS[1] masks to 7 bits",
        base: TWIS1_BASE,
        prep: TWIS_ENABLE_PREP,
        write: (TWIS1_ADDRESS1, 0x76),
        read_addr: TWIS1_ADDRESS1,
        mask: 0x7F,
        expect: 0x76,
    },
    // ── TWIS1: CONFIG (2-bit) ─────────────────────────────────────────────────
    Case {
        label: "TWIS1 CONFIG round-trip (2 bits)",
        base: TWIS1_BASE,
        prep: TWIS_ENABLE_PREP,
        write: (TWIS1_CONFIG, 0x3),
        read_addr: TWIS1_CONFIG,
        mask: 0x3,
        expect: 0x3,
    },
    // ── TWIS1: ORC (8-bit) ───────────────────────────────────────────────────
    Case {
        label: "TWIS1 ORC round-trip (8 bits)",
        base: TWIS1_BASE,
        prep: TWIS_ENABLE_PREP,
        write: (TWIS1_ORC, 0xFF),
        read_addr: TWIS1_ORC,
        mask: 0xFF,
        expect: 0xFF,
    },
    // ── TWIS1: INTENSET/INTENCLR ──────────────────────────────────────────────
    Case {
        label: "TWIS1 INTENSET set valid bits",
        base: TWIS1_BASE,
        prep: &[(TWIS1_INTENCLR, 0xFFFF_FFFF), (TWIS1_ENABLE, 9)],
        write: (TWIS1_INTENSET, TWIS_INTEN_MASK),
        read_addr: TWIS1_INTENSET,
        mask: TWIS_INTEN_MASK,
        expect: TWIS_INTEN_MASK,
    },
    Case {
        label: "TWIS1 INTENCLR clears all",
        base: TWIS1_BASE,
        prep: &[(TWIS1_INTENSET, TWIS_INTEN_MASK), (TWIS1_ENABLE, 9)],
        write: (TWIS1_INTENCLR, TWIS_INTEN_MASK),
        read_addr: TWIS1_INTENSET,
        mask: TWIS_INTEN_MASK,
        expect: 0,
    },
    // ── TWIS1: EVENTS — SW write-1 ignored ───────────────────────────────────
    Case {
        label: "TWIS1 EVENTS_STOPPED SW-write-1 ignored",
        base: TWIS1_BASE,
        prep: TWIS_ENABLE_PREP,
        write: (TWIS1_EVENTS_STOPPED, 1),
        read_addr: TWIS1_EVENTS_STOPPED,
        mask: 1,
        expect: 0,
    },
    Case {
        label: "TWIS1 EVENTS_ERROR SW-write-1 ignored",
        base: TWIS1_BASE,
        prep: TWIS_ENABLE_PREP,
        write: (TWIS1_EVENTS_ERROR, 1),
        read_addr: TWIS1_EVENTS_ERROR,
        mask: 1,
        expect: 0,
    },
    Case {
        label: "TWIS1 EVENTS_RXSTARTED SW-write-1 ignored",
        base: TWIS1_BASE,
        prep: TWIS_ENABLE_PREP,
        write: (TWIS1_EVENTS_RXSTARTED, 1),
        read_addr: TWIS1_EVENTS_RXSTARTED,
        mask: 1,
        expect: 0,
    },
    Case {
        label: "TWIS1 EVENTS_TXSTARTED SW-write-1 ignored",
        base: TWIS1_BASE,
        prep: TWIS_ENABLE_PREP,
        write: (TWIS1_EVENTS_TXSTARTED, 1),
        read_addr: TWIS1_EVENTS_TXSTARTED,
        mask: 1,
        expect: 0,
    },
    Case {
        label: "TWIS1 EVENTS_WRITE SW-write-1 ignored",
        base: TWIS1_BASE,
        prep: TWIS_ENABLE_PREP,
        write: (TWIS1_EVENTS_WRITE, 1),
        read_addr: TWIS1_EVENTS_WRITE,
        mask: 1,
        expect: 0,
    },
    Case {
        label: "TWIS1 EVENTS_READ SW-write-1 ignored",
        base: TWIS1_BASE,
        prep: TWIS_ENABLE_PREP,
        write: (TWIS1_EVENTS_READ, 1),
        read_addr: TWIS1_EVENTS_READ,
        mask: 1,
        expect: 0,
    },
    // ── TWIS1: ERRORSRC W1C — seed via direct model write, then check clear ──
    // Note: on silicon ERRORSRC is read-only unless the HW sets it; in the
    // reset-halted sweep it will always read 0.  We verify that writing 1
    // is idempotent (doesn't SET the register, as W1C write is a clear).
    Case {
        label: "TWIS1 ERRORSRC write-1 idempotent (HW never set it)",
        base: TWIS1_BASE,
        prep: TWIS_ENABLE_PREP,
        write: (TWIS1_ERRORSRC, 0x7),
        read_addr: TWIS1_ERRORSRC,
        mask: 0x7,
        expect: 0,
    },
];

// ── Sim-side: standalone models with manually managed address offsets ─────────
//
// Because the SPIS/TWIS models are not declared in the production chip YAML
// (their base addresses conflict with existing spi0 and twi1 master entries),
// we build them directly and route read/write_u32 calls by subtracting the
// base address to obtain the register offset.

fn sim_read(
    spis: &Nrf52Spis,
    twis: &Nrf52Twis,
    base: u32,
    addr: u32,
) -> Result<u32, String> {
    let offset = (addr - base) as u64;
    if base == SPIS0_BASE {
        spis.read_u32(offset).map_err(|e| format!("{e:?}"))
    } else {
        twis.read_u32(offset).map_err(|e| format!("{e:?}"))
    }
}

fn sim_write(
    spis: &mut Nrf52Spis,
    twis: &mut Nrf52Twis,
    base: u32,
    addr: u32,
    val: u32,
) {
    let offset = (addr - base) as u64;
    if base == SPIS0_BASE {
        spis.write_u32(offset, val)
            .unwrap_or_else(|e| panic!("sim write 0x{addr:08X}=0x{val:08X}: {e:?}"));
    } else {
        twis.write_u32(offset, val)
            .unwrap_or_else(|e| panic!("sim write 0x{addr:08X}=0x{val:08X}: {e:?}"));
    }
}

fn run_case(
    spis: &mut Nrf52Spis,
    twis: &mut Nrf52Twis,
    oc: &mut OpenOcd,
    case: &Case,
) -> Outcome {
    // Prep writes (both sim + hw).
    for &(addr, val) in case.prep {
        sim_write(spis, twis, case.base, addr, val);
        oc.write_memory(addr, &[val])
            .unwrap_or_else(|e| panic!("hw prep write 0x{addr:08X}=0x{val:08X}: {e}"));
    }
    // Main write.
    sim_write(spis, twis, case.base, case.write.0, case.write.1);
    oc.write_memory(case.write.0, &[case.write.1])
        .unwrap_or_else(|e| panic!("hw write 0x{:08X}=0x{:08X}: {e}", case.write.0, case.write.1));
    // Read back.
    let sim_val = match sim_read(spis, twis, case.base, case.read_addr) {
        Ok(v) => v,
        Err(e) => return Outcome::SimError(e),
    };
    let hw_val = oc
        .read_memory(case.read_addr, 1)
        .unwrap_or_else(|e| panic!("hw read 0x{:08X}: {e}", case.read_addr))[0];

    let sim_m = sim_val & case.mask;
    let hw_m = hw_val & case.mask;

    if sim_m == hw_m {
        if sim_m == case.expect {
            Outcome::Match
        } else {
            Outcome::BothDisagreeWithExpect { both: sim_m }
        }
    } else {
        Outcome::Diverge { sim: sim_m, hw: hw_m }
    }
}

#[test]
#[ignore]
fn nrf52840_spis_twis_conformance() {
    let _guard = HW_LOCK.lock().unwrap();
    let mut spis = Nrf52Spis::new();
    let mut twis = Nrf52Twis::new();
    let mut oc = OpenOcd::spawn_nrf52().expect("openocd spawn_nrf52 failed");
    oc.reset_halt().expect("reset halt failed");
    oc.halt().expect("halt failed");

    println!();
    println!(
        "nRF52840 SPIS0+TWIS1 register-surface conformance — {} cases",
        CASES.len()
    );
    println!(
        "  SPIS0 base: 0x{SPIS0_BASE:08X} (shared with SPIM0; ENABLE=2 selects SPIS)",
    );
    println!(
        "  TWIS1 base: 0x{TWIS1_BASE:08X} (shared with TWIM1; ENABLE=9 selects TWIS)",
    );
    println!(
        "  Note: SPIS0/TWIS0 both live at 0x40003000; this sweep uses TWIS1 at 0x40004000",
    );
    println!("{:-<92}", "");

    let mut by_periph: std::collections::BTreeMap<&str, (u32, u32, u32, u32)> =
        std::collections::BTreeMap::new();

    for case in CASES {
        let periph = if case.base == SPIS0_BASE { "SPIS0" } else { "TWIS1" };
        let b = by_periph.entry(periph).or_insert((0, 0, 0, 0));
        match run_case(&mut spis, &mut twis, &mut oc, case) {
            Outcome::Match => {
                b.0 += 1;
                println!("[OK ]  {}", case.label);
            }
            Outcome::Diverge { sim, hw } => {
                b.1 += 1;
                println!(
                    "[DIFF] {}  sim=0x{:08X} hw=0x{:08X} (mask=0x{:08X})",
                    case.label, sim, hw, case.mask
                );
            }
            Outcome::BothDisagreeWithExpect { both } => {
                b.2 += 1;
                println!(
                    "[BOTH] {}  both=0x{:08X} expected=0x{:08X}",
                    case.label, both, case.expect
                );
            }
            Outcome::SimError(m) => {
                b.3 += 1;
                println!("[SIM!] {}  {}", case.label, m);
            }
        }
    }

    // ── Restore ENABLE=0 on both peripherals so the shared blocks revert to
    //    disabled/neutral mode — avoids leaving the board in slave mode.
    oc.write_memory(SPIS0_ENABLE, &[0])
        .unwrap_or_else(|e| panic!("restore SPIS0 ENABLE=0: {e}"));
    oc.write_memory(TWIS1_ENABLE, &[0])
        .unwrap_or_else(|e| panic!("restore TWIS1 ENABLE=0: {e}"));
    // Mirror the ENABLE=0 restore in the sim models too.
    spis.write_u32(0x500, 0).ok();
    twis.write_u32(0x500, 0).ok();

    println!("{:-<92}", "");
    let mut total_div = 0u32;
    for (p, (m, d, bo, se)) in &by_periph {
        total_div += *d;
        println!("{p}: match={m} diverge={d} both_disagree={bo} sim_err={se}");
    }
    println!("Summary: total_diverge={total_div}");
    oc.shutdown().ok();

    if std::env::var("NRF52_STRICT").is_ok() {
        assert_eq!(total_div, 0, "SPIS/TWIS diff: {total_div} register(s) diverged");
    }
}
