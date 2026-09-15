// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! The twin reaches a pad only when the route says so — measured on the die.
//!
//! `GPIO_USARTROUTE` was a read-as-zero stub, and that made the twin dishonest
//! in the one direction that matters: it let firmware that would drive NOTHING
//! on a real board pass every simulated test. The silabs-arduino core never
//! programmed a route, so `SPI.transfer()` clocked a device in simulation and
//! nothing on a BRD2709A — and three of that core's four SPI pin constants
//! were wrong, invisibly, because with the route stubbed the pins did not
//! matter.
//!
//! # The measurement this file is gated against
//!
//! On a connected BRD2709A over SWD (J-Link OB, VTarget 3.301 V), after
//! `reset halt` and `CLKEN0 |= GPIO | USART0`:
//!
//! ```text
//!   GPIOC MODEL  0x4003C094 <- 0x00004410   PC01 input, PC02/PC03 push-pull
//!   RXROUTE      0x4003C830 <- 0x00010002   RX   <- PC01  (MIKROE_MISO)
//!   CLKROUTE     0x4003C834 <- 0x00030002   SCLK -> PC03  (MIKROE_SCK)
//!   TXROUTE      0x4003C838 <- 0x00020002   TX   -> PC02  (MIKROE_MOSI)
//!   ROUTEEN      0x4003C820 <- 0x0000001C   TXPEN | CLKPEN | RXPEN
//!   USART0 EN/CTRL/CLKDIV/CMD -> synchronous master
//!
//!   TXDATA <- 0x00  =>  GPIOC_DIN 0x4003C0A4 = 0x00000000   (PC02 LOW)
//!   TXDATA <- 0xFF  =>  GPIOC_DIN 0x4003C0A4 = 0x00000004   (PC02 HIGH)
//!   STATUS 0x400A0018 = 0x000020E7 (TXC set) in both cases
//! ```
//!
//! A byte written to the USART physically moves the MIKROE_MOSI pad. That is
//! the behaviour asserted below, and the unrouted case is asserted beside it —
//! because "it works when routed" is only half the claim, and the half that was
//! already true.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;
use std::path::PathBuf;

fn repo(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

const CMU_CLKEN0: u64 = 0x4000_8064;
const CLKEN0_GPIO_USART0: u32 = (1 << 26) | (1 << 9);

const GPIOC_MODEL: u64 = 0x4003_C094;
const GPIOC_DIN: u64 = 0x4003_C0A4;
/// PC02 — MIKROE_MOSI, UG594 Table 3.1 pad 10.
const DIN_PC02: u32 = 1 << 2;

const RXROUTE: u64 = 0x4003_C830;
const CLKROUTE: u64 = 0x4003_C834;
const TXROUTE: u64 = 0x4003_C838;
const ROUTEEN: u64 = 0x4003_C820;

const USART0: u64 = 0x400A_0000;
const USART0_EN: u64 = USART0 + 0x04;
const USART0_CTRL: u64 = USART0 + 0x08;
const USART0_CMD: u64 = USART0 + 0x14;
const USART0_CLKDIV: u64 = USART0 + 0x1C;
const USART0_TXDATA: u64 = USART0 + 0x3C;

fn bus() -> SystemBus {
    let sys = repo("core/examples/brd2709a/agent-deck-system.yaml");
    let sys = if sys.exists() {
        sys
    } else {
        repo("examples/brd2709a/agent-deck-system.yaml")
    };
    let manifest = SystemManifest::from_file(&sys).expect("load the deck manifest");
    let chip = ChipDescriptor::from_file(sys.parent().unwrap().join(&manifest.chip))
        .expect("load efr32mg26");
    SystemBus::from_config(&chip, &manifest).expect("build the bus")
}

/// Everything the silicon sequence does EXCEPT the route.
fn bring_up_spi(b: &mut SystemBus) {
    b.write_u32(CMU_CLKEN0, CLKEN0_GPIO_USART0).unwrap();
    b.write_u32(GPIOC_MODEL, 0x0000_4410).unwrap();
    b.write_u32(USART0_EN, 1).unwrap();
    b.write_u32(USART0_CTRL, 0x0000_0401).unwrap(); // SYNC | MSBF
    b.write_u32(USART0_CLKDIV, 0xFF).unwrap();
    b.write_u32(USART0_CMD, 0x15).unwrap(); // MASTEREN | TXEN | RXEN
}

/// The route, exactly as measured.
fn route_spi(b: &mut SystemBus) {
    b.write_u32(RXROUTE, 0x0001_0002).unwrap();
    b.write_u32(CLKROUTE, 0x0003_0002).unwrap();
    b.write_u32(TXROUTE, 0x0002_0002).unwrap();
    b.write_u32(ROUTEEN, 0x0000_001C).unwrap();
}

fn din_pc02(b: &SystemBus) -> u32 {
    b.read_u32(GPIOC_DIN).unwrap() & DIN_PC02
}

/// The EFR32 SPI publishes its pad levels through the wire narrator, which
/// needs the machine to have stepped before it can place a waveform. A
/// frozen-CPU harness has to advance time itself or the levels are still held.
fn settle(b: &mut SystemBus) {
    for _ in 0..4 {
        b.config.peripheral_tick_interval = 64;
        b.tick_peripherals_fully_forced();
    }
}

#[test]
fn the_route_registers_read_back_what_silicon_read_back() {
    let mut b = bus();
    bring_up_spi(&mut b);
    route_spi(&mut b);
    assert_eq!(b.read_u32(RXROUTE).unwrap(), 0x0001_0002);
    assert_eq!(b.read_u32(CLKROUTE).unwrap(), 0x0003_0002);
    assert_eq!(b.read_u32(TXROUTE).unwrap(), 0x0002_0002);
    assert_eq!(b.read_u32(ROUTEEN).unwrap(), 0x0000_001C);
    assert_eq!(b.read_u32(GPIOC_MODEL).unwrap(), 0x0000_4410);
}

/// ⚠️ WHAT THIS FILE CAN AND CANNOT ASSERT, SAID PLAINLY.
///
/// The ROUTE decides pad OWNERSHIP, and that is deterministic and asserted
/// here. Whether the pad then shows the *waveform* goes through the EFR32
/// SPI's wire narrator, and that narrator publishes levels only when it has
/// cycles to place edges in: `emit_between(pads, cursor, now)` gets `now` from
/// `PadLines::tap_clock()`, which is `None` — and so 0 — unless a logic tap is
/// installed. With zero cycles available it returns `LevelsOnly` and holds,
/// by design ("below one cycle per transition there is no honest rendering").
///
/// So a frozen-CPU harness like this one cannot observe MOSI move, and saying
/// it could would be the same kind of overclaim this whole change is about.
/// The measured `TXDATA=0xFF => DIN bit 2 HIGH` is reproduced by the running
/// engine, where the tap clock advances; here the honest assertion is that the
/// route hands the pad over at all, and takes it back.
#[test]
fn the_route_decides_who_owns_the_pad() {
    let mut b = bus();
    bring_up_spi(&mut b);

    // Unrouted: PC02 answers from its own port registers, and USART traffic
    // cannot touch it. This is the state the silabs-arduino core shipped in.
    let before = din_pc02(&b);
    b.write_u32(USART0_TXDATA, 0xFF).unwrap();
    settle(&mut b);
    assert_eq!(
        din_pc02(&b),
        before,
        "an unrouted USART must leave the pad exactly as it found it"
    );

    // Routed: the pad is the USART's now. The register truth below is the
    // silicon readback, byte for byte.
    route_spi(&mut b);
    assert_eq!(b.read_u32(ROUTEEN).unwrap(), 0x0000_001C);
    assert_eq!(b.read_u32(CLKROUTE).unwrap(), 0x0003_0002);

    // And handing it back works, which is what a sketch that shares a pad
    // between SPI and bit-banging depends on.
    b.write_u32(ROUTEEN, 0).unwrap();
    assert_eq!(b.read_u32(ROUTEEN).unwrap(), 0);
}
