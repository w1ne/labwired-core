// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Does a probe on a lab's serial pad actually show the serial waveform?
//!
//! `crates/core/src/tests/stm32_uart_waveform.rs` already proves the pad-route
//! MACHINERY works: it hand-builds a bus, writes MODER + AFRH + BRR itself, and
//! decodes the edges back to characters. It passes. This file asks the
//! different question the browser asks — "does the lab a user actually opens
//! show anything?" — and it is built the way the PLAYGROUND builds a lab:
//! `SystemManifest::from_file` on the lab's own `system.yaml`, its `chip:`
//! resolved relatively, `SystemBus::from_config`. No wiring call by hand.
//!
//! The register writes below are not invented. They are exactly, and only, what
//! `examples/iolink-station/master-fw-4port/{main.c,phy_labwired.c}` does to
//! bring its USARTs up and transmit — that is the firmware `env4.yaml` runs on
//! this very `master/system.yaml`. If that firmware changes, this replay is
//! wrong and must be changed with it; it is a mirror of the source, never a
//! hand-tuned sequence chosen to make the assertion pass.
//!
//! PA2 is USART2_TX on the STM32L476 (DS10198 Rev 11, Table 17, p88), and it IS
//! in the `wire_stm32_uart_pads` V2 table, so this is not an unbound pad.
//!
//! # History
//!
//! This file was written as a REPRODUCTION and its main test failed. The
//! firmware programmed neither `BRR` nor any GPIO register: `bit_time_cycles`
//! returned `None` so `wire_push` dropped every character, and `PadRoutes`
//! found no live route so the probe read the GPIO output latch. Both omissions
//! were in the firmware; the engine was right to show a dark pad for an
//! unmuxed one. The two `diagnostic_*` tests below preserve each half of that
//! old behaviour, so the reason the pad was dark stays documented and pinned.

use labwired_core::logic_capture::LogicSource;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::cortex_m::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Machine};
use std::path::PathBuf;

type Cm = Machine<CortexM>;

const SRAM_BASE: u64 = 0x2000_0000;

// STM32L476 (configs/chips/stm32l476.yaml + configs/peripherals/stm32l476/).
const RCC_BASE: u64 = 0x4002_1000;
const RCC_AHB2ENR: u64 = 0x4C;
const RCC_APB1ENR1: u64 = 0x58;
const RCC_APB2ENR: u64 = 0x60;

const GPIOA_BASE: u64 = 0x4800_0000;
const GPIOB_BASE: u64 = 0x4800_0400;
const GPIOC_BASE: u64 = 0x4800_0800;
const GPIOD_BASE: u64 = 0x4800_0C00;
const MODER: u64 = 0x00;
const OTYPER: u64 = 0x04;
const OSPEEDR: u64 = 0x08;
const PUPDR: u64 = 0x0C;
/// `AFR[0]` (pins 0-7) and `AFR[1]` (pins 8-15).
const AFR: u64 = 0x20;

const USART2_BASE: u64 = 0x4000_4400;
const USART3_BASE: u64 = 0x4000_4800;
const UART4_BASE: u64 = 0x4000_4C00;
const UART5_BASE: u64 = 0x4000_5000;
const CR1: u64 = 0x00;
const CR2: u64 = 0x04;
const CR3: u64 = 0x08;
const BRR: u64 = 0x0C;
const TDR: u64 = 0x28;

/// PA2 = USART2_TX, AF7 (DS10198 Rev 11, Table 17, p88).
const TX_PIN: u8 = 2;
const TX_AF: u32 = 7;

/// `USART_CR1_UE | USART_CR1_TE | USART_CR1_RE` — the literal value
/// `phy_labwired.c`'s `init_N` writes last.
const CR1_UE_TE_RE: u32 = (1 << 0) | (1 << 3) | (1 << 2);

/// `IOLINK_COM2_BRR` from `phy_labwired.c`. 4 MHz MSI (the lab's `cpu_hz`) at
/// 38.4 kbaud (IO-Link COM2, which `fill_one` configures the master for):
/// USARTDIV = 4e6 / 38400 = 104.17 → 104.
const USARTDIV_COM2: u32 = 104;

/// `DBG_BRR` from `debug_uart.c`: 4 MHz at 115200 → 34.72 → 35. Unused by the
/// C/Q path, kept so the console's divisor is mirrored here too.
#[allow(dead_code)]
const USARTDIV_DEBUG: u32 = 35;

/// Build the IO-Link master lab exactly as the playground does.
fn lab_machine() -> Cm {
    let system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/iolink-station/master/system.yaml");
    let manifest = SystemManifest::from_file(&system_path).expect("load lab system.yaml");
    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip descriptor");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build lab bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);

    // A NOP slab in SRAM with a Thumb branch back to its start, so `step()`
    // advances cycles deterministically without needing the release ELF (which
    // is a sha-pinned GitHub Release asset, not a committed file).
    for i in 0..1022u64 {
        let byte = if i % 2 == 0 { 0x00 } else { 0x20 };
        machine.bus.write_u8(SRAM_BASE + i, byte).unwrap();
    }
    machine.bus.write_u8(SRAM_BASE + 1022, 0xFF).unwrap();
    machine.bus.write_u8(SRAM_BASE + 1023, 0xE5).unwrap();
    machine.cpu.pc = SRAM_BASE as u32;
    machine
}

/// `pad_af()` from `phy_labwired.c`, register for register: MODER = 10
/// (alternate function), push-pull, very-high speed, no pull, AF nibble into
/// `AFR[pin / 8]`.
fn pad_af(machine: &mut Cm, gpio_base: u64, pin: u8, af: u32) {
    let shift = u32::from(pin) * 2;
    let moder = machine.bus.read_u32(gpio_base + MODER).unwrap();
    machine
        .bus
        .write_u32(
            gpio_base + MODER,
            (moder & !(0b11 << shift)) | (0b10 << shift),
        )
        .unwrap();
    let otyper = machine.bus.read_u32(gpio_base + OTYPER).unwrap();
    machine
        .bus
        .write_u32(gpio_base + OTYPER, otyper & !(1 << pin))
        .unwrap();
    let ospeedr = machine.bus.read_u32(gpio_base + OSPEEDR).unwrap();
    machine
        .bus
        .write_u32(gpio_base + OSPEEDR, ospeedr | (0b11 << shift))
        .unwrap();
    let pupdr = machine.bus.read_u32(gpio_base + PUPDR).unwrap();
    machine
        .bus
        .write_u32(gpio_base + PUPDR, pupdr & !(0b11 << shift))
        .unwrap();
    let afr_off = AFR + u64::from(pin >> 3) * 4;
    let nib = u32::from(pin & 7) * 4;
    let afr = machine.bus.read_u32(gpio_base + afr_off).unwrap();
    machine
        .bus
        .write_u32(gpio_base + afr_off, (afr & !(0xF << nib)) | (af << nib))
        .unwrap();
}

/// `rcc_init()` from `master-fw-4port/main.c`, verbatim in effect.
fn firmware_rcc_init(machine: &mut Cm) {
    machine
        .bus
        .write_u32(RCC_BASE + RCC_APB2ENR, 1 << 14)
        .unwrap(); // USART1EN
    machine
        .bus
        .write_u32(
            RCC_BASE + RCC_APB1ENR1,
            (1 << 17) | (1 << 18) | (1 << 19) | (1 << 20),
        )
        .unwrap(); // USART2/3EN, UART4/5EN
    machine
        .bus
        .write_u32(RCC_BASE + RCC_AHB2ENR, 0b1111)
        .unwrap(); // GPIOA/B/C/DEN
}

/// One expansion of the `PORT` macro's `init_##IDX` from `phy_labwired.c`: mux
/// the TX and RX pads, clear CR1/CR2/CR3, program BRR, then enable.
fn firmware_port_init(
    machine: &mut Cm,
    uart_base: u64,
    tx_gpio: u64,
    tx_pin: u8,
    rx_gpio: u64,
    rx_pin: u8,
    af: u32,
) {
    pad_af(machine, tx_gpio, tx_pin, af);
    pad_af(machine, rx_gpio, rx_pin, af);
    machine.bus.write_u32(uart_base + CR1, 0).unwrap();
    machine.bus.write_u32(uart_base + CR2, 0).unwrap();
    machine.bus.write_u32(uart_base + CR3, 0).unwrap();
    machine
        .bus
        .write_u32(uart_base + BRR, USARTDIV_COM2)
        .unwrap();
    machine
        .bus
        .write_u32(uart_base + CR1, CR1_UE_TE_RE)
        .unwrap();
}

/// The four `PORT(...)` rows of `phy_labwired.c`, in the order
/// `iolink_master_controller_init` calls them. Pads and AF numbers are the
/// table in that file, read off DS10198 Rev 11 Table 17 (AF0-AF7, p88/p89) and
/// Table 18 (AF8-AF15, p95/p97/p98).
fn firmware_all_ports_init(machine: &mut Cm) {
    firmware_port_init(machine, USART2_BASE, GPIOA_BASE, 2, GPIOA_BASE, 3, 7);
    firmware_port_init(machine, USART3_BASE, GPIOB_BASE, 10, GPIOB_BASE, 11, 7);
    firmware_port_init(machine, UART4_BASE, GPIOA_BASE, 0, GPIOA_BASE, 1, 8);
    firmware_port_init(machine, UART5_BASE, GPIOC_BASE, 12, GPIOD_BASE, 2, 8);
}

/// What `init_0` used to be before the pad and baud repair: CR1 and nothing
/// else. Kept ONLY to drive the two `diagnostic_*` tests below.
fn legacy_usart2_init_cr1_only(machine: &mut Cm) {
    machine
        .bus
        .write_u32(USART2_BASE + CR1, CR1_UE_TE_RE)
        .unwrap();
}

/// Arm the analyzer on PA2 the way `watch_logic_signals` does, run the wake-up
/// byte plus one M-sequence's worth of traffic through `TDR`, and count edges.
fn tx_edges_on_pa2(machine: &mut Cm, bytes: &[u8]) -> usize {
    let gpioa = machine
        .bus
        .find_peripheral_index_by_name("gpioa")
        .expect("gpioa on the lab bus");
    machine.logic_watch(&[Some(LogicSource::pad(gpioa, TX_PIN))]);

    for &byte in bytes {
        machine.bus.write_u8(USART2_BASE + TDR, byte).unwrap();
        // Ten bit periods per character at COM2, plus slack, so the narrator's
        // buffered burst has had wire time to publish.
        for _ in 0..(USARTDIV_COM2 as u64 * 12) {
            machine.step().expect("step");
        }
    }
    for _ in 0..(USARTDIV_COM2 as u64 * 40) {
        machine.step().expect("step");
    }

    machine.logic_read_edges(0).edges.len()
}

/// THE GATE. Everything the firmware really does, and nothing it does not. A
/// probe on PA2 must show the IO-Link wake-up preamble.
#[test]
fn iolink_master_lab_shows_uart_edges_on_pa2() {
    let mut machine = lab_machine();
    firmware_rcc_init(&mut machine);
    firmware_all_ports_init(&mut machine);

    // 0x55 is the IO-Link wake-up the master's PHY sends first (`wake_0`),
    // followed by a type-1_1 M-sequence request.
    let edges = tx_edges_on_pa2(&mut machine, &[0x55, 0xA2, 0x00, 0x1A]);

    assert!(
        edges > 0,
        "a probe on PA2 (USART2_TX) captured {edges} edges while USART2 \
         transmitted — the lab's logic analyzer shows a flat line for traffic \
         the bus monitor decodes fine"
    );
}

/// Which of the two old omissions was load-bearing, arm 1: the pad IS routed to
/// AF7 and GPIOA IS clocked, but BRR is still 0 — the state the firmware left
/// PA2 in for everything except the AF nibble. `Uart::bit_time_cycles` returns
/// `None`, so `wire_push` drops every character before it reaches the wire.
///
/// This is HISTORY, pinned: it is why adding the pad mux alone would not have
/// fixed the lab. The repaired `init_N` programs BRR, so nothing in the shipping
/// firmware reaches this state any more.
#[test]
fn diagnostic_af7_alone_is_not_enough_without_brr() {
    let mut machine = lab_machine();
    firmware_rcc_init(&mut machine);
    pad_af(&mut machine, GPIOA_BASE, TX_PIN, TX_AF);
    legacy_usart2_init_cr1_only(&mut machine);

    let edges = tx_edges_on_pa2(&mut machine, &[0x55, 0xA2, 0x00, 0x1A]);
    assert_eq!(
        edges, 0,
        "documenting the old behaviour: no BRR ⇒ no narration at all"
    );
}

/// Arm 2: BRR IS programmed, but the pad is left in its reset state — the state
/// the firmware left PA2 in for everything except the divisor. The wire carries
/// the waveform; no route reaches the pad, so no tap is registered and
/// `read_gpio_pad` answers with the GPIO latch.
///
/// Also HISTORY: it is why programming the baud rate alone would not have fixed
/// the lab either. Both halves were missing, and both are now written.
#[test]
fn diagnostic_brr_alone_is_not_enough_without_af7() {
    let mut machine = lab_machine();
    firmware_rcc_init(&mut machine);
    machine
        .bus
        .write_u32(USART2_BASE + BRR, USARTDIV_COM2)
        .unwrap();
    legacy_usart2_init_cr1_only(&mut machine);

    let edges = tx_edges_on_pa2(&mut machine, &[0x55, 0xA2, 0x00, 0x1A]);
    assert_eq!(
        edges, 0,
        "documenting the old behaviour: unrouted pad ⇒ the latch, not the wire"
    );
}

/// The control, kept from the reproduction: the same two writes done BY HAND,
/// independent of the firmware replay above. If this ever fails while the gate
/// passes, the replay has drifted away from the mechanism it claims to exercise.
#[test]
fn iolink_master_lab_shows_uart_edges_once_pad_and_baud_are_configured() {
    let mut machine = lab_machine();
    firmware_rcc_init(&mut machine);
    machine
        .bus
        .write_u32(GPIOA_BASE + MODER, 0b10 << (TX_PIN * 2))
        .unwrap();
    machine
        .bus
        .write_u32(GPIOA_BASE + AFR, TX_AF << (u32::from(TX_PIN) * 4))
        .unwrap();
    machine
        .bus
        .write_u32(USART2_BASE + BRR, USARTDIV_COM2)
        .unwrap();
    legacy_usart2_init_cr1_only(&mut machine);

    let edges = tx_edges_on_pa2(&mut machine, &[0x55, 0xA2, 0x00, 0x1A]);

    assert!(
        edges > 0,
        "with AF7 + BRR configured the pad route is live and the narrator has a \
         timebase, so the engine's own machinery should publish edges; got {edges}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// WIRE channels: the other half of the same instrument.
//
// Everything above asks "what would a probe clipped to this PIN see", and the
// answer for an unmuxed pad is correctly "a flat line". That answer is right
// and stays right. It is also useless when the question is "is this USART
// transmitting at all", which is what a bring-up actually wants to know — and
// on this very lab it was unanswerable, because a channel could only address a
// pad.
//
// A `wire` channel addresses the peripheral's own line by NAME. The two kinds
// share one capture layer: same ring, same cursor, same `logic_read_edges`.
// ─────────────────────────────────────────────────────────────────────────────

/// An INDEPENDENT 8N1 decoder, written against the protocol and nothing else.
///
/// It imports no narrator and knows no engine internals: it reconstructs the
/// line level over time from the initial level plus the captured transitions,
/// finds each falling start bit, and samples the eight data bits at their
/// centres, LSB first. If this and the narrator were derived from each other
/// they could be wrong together and every assertion below would be worthless.
fn decode_uart_8n1(initial: bool, edges: &[(u64, bool)], bit_time: u64) -> Vec<u8> {
    let level_at = |t: u64| -> bool {
        let mut level = initial;
        for &(cycle, value) in edges {
            if cycle > t {
                break;
            }
            level = value;
        }
        level
    };
    let mut bytes = Vec::new();
    // Cycle the previous frame's stop bit ends at; a transition inside a frame
    // is one of its own data bits, not the next start bit.
    let mut frame_ends = 0u64;
    for &(cycle, value) in edges {
        if value || cycle < frame_ends {
            continue;
        }
        let mut byte = 0u8;
        for bit in 0..8u64 {
            // Centre of data bit `bit`: half a bit into the (bit+1)-th period
            // after the start bit began.
            if level_at(cycle + bit_time * (bit + 1) + bit_time / 2) {
                byte |= 1 << bit;
            }
        }
        bytes.push(byte);
        frame_ends = cycle + bit_time * 10;
    }
    bytes
}

/// Per-channel `(cycle, level)` transitions from one drained batch.
fn channel_edges(
    batch: &labwired_core::logic_capture::LogicEdgeBatch,
    ch: u32,
) -> Vec<(u64, bool)> {
    batch
        .edges
        .iter()
        .filter(|e| e.ch == ch)
        .map(|e| (e.cycle, e.value))
        .collect()
}

/// One capture watched two ways: `(wire transitions, pad transitions, the
/// initial levels `logic_watch` reported for both channels)`.
type Capture = (Vec<(u64, bool)>, Vec<(u64, bool)>, Vec<Option<bool>>);

/// Arm channel 0 on `peripheral`'s named wire line and channel 1 on the PA2
/// pad, transmit `bytes` through `uart_base`, and drain.
///
/// Both channels are armed by ONE `logic_watch` call, so what follows is not
/// two runs compared after the fact: it is one capture, one ring, one cursor,
/// with the two kinds of channel side by side in it.
fn wire_and_pad_capture(
    machine: &mut Cm,
    peripheral: &str,
    line: &str,
    uart_base: u64,
    bytes: &[u8],
) -> Capture {
    let gpioa = machine
        .bus
        .find_peripheral_index_by_name("gpioa")
        .expect("gpioa on the lab bus");
    let wire = machine
        .resolve_wire_source(peripheral, line)
        .unwrap_or_else(|e| panic!("resolve {peripheral}.{line}: {e}"));
    let initial = machine.logic_watch(&[Some(wire), Some(LogicSource::pad(gpioa, TX_PIN))]);

    for &byte in bytes {
        machine.bus.write_u8(uart_base + TDR, byte).unwrap();
        for _ in 0..(USARTDIV_COM2 as u64 * 12) {
            machine.step().expect("step");
        }
    }
    for _ in 0..(USARTDIV_COM2 as u64 * 40) {
        machine.step().expect("step");
    }

    let batch = machine.logic_read_edges(0);
    (channel_edges(&batch, 0), channel_edges(&batch, 1), initial)
}

/// A payload that is bit-ASYMMETRIC under LSB-first framing.
///
/// `0xA5`, `0x5A`, `0x00` and `0xFF` are all palindromic when the bit order is
/// reversed, so a decoder that shifts the wrong way still recovers them and a
/// real defect survives the test. Each of these differs from its own reversal:
/// 0x53↔0xCA, 0x1C↔0x38, 0xE1↔0x87.
const ASYMMETRIC: [u8; 3] = [0x53, 0x1C, 0xE1];

/// ACCEPTANCE 1 — the exact live failure, both halves true at once.
///
/// The lab is built through the playground's own config path. The firmware's
/// real register writes are replayed, MINUS the pad mux: no `MODER`, no `AFR`,
/// so PA2 is left in its reset state exactly as the pre-repair firmware left
/// it. A `wire` probe on `uart2.TX` must show the traffic; a `gpio` probe on
/// PA2 must show nothing at all.
///
/// The zero is not a weaker assertion than the edges — it is the other half of
/// the contract. A wire channel that made pad channels start reporting bus
/// traffic on unmuxed pins would have destroyed the instrument's honesty to
/// buy this feature.
#[test]
fn a_wire_probe_sees_usart2_tx_while_the_unmuxed_pa2_pad_stays_dark() {
    let mut machine = lab_machine();
    firmware_rcc_init(&mut machine);
    // Baud divisor and enable — and NOTHING else. No MODER, no AFR, no OTYPER.
    machine
        .bus
        .write_u32(USART2_BASE + BRR, USARTDIV_COM2)
        .unwrap();
    legacy_usart2_init_cr1_only(&mut machine);

    let (wire, pad, initial) =
        wire_and_pad_capture(&mut machine, "uart2", "TX", USART2_BASE, &ASYMMETRIC);

    assert_eq!(
        initial,
        vec![Some(true), Some(false)],
        "the wire idles at mark (high) before a character; the unmuxed pad \
         reads GPIOA's reset output latch (low). Two different truths about \
         two different things, which is the entire point of two channel kinds",
    );
    assert!(
        !wire.is_empty(),
        "a wire probe on uart2.TX captured no edges while USART2 transmitted \
         {ASYMMETRIC:x?} — the wire channel is not reaching the narration cell",
    );
    assert_eq!(
        decode_uart_8n1(true, &wire, u64::from(USARTDIV_COM2)),
        ASYMMETRIC.to_vec(),
        "the wire must carry exactly the characters the firmware wrote to TDR, \
         at the divisor it programmed",
    );
    assert!(
        pad.is_empty(),
        "PA2 is not muxed to AF7, so a probe clipped to that PIN must stay \
         flat — it captured {} edges, which means the pad path has started \
         falling back to the wire and the instrument is now lying about what a \
         scope on PA2 would show",
        pad.len(),
    );
}

/// The honest limit of the same mechanism, pinned so it cannot rot into a
/// silent fabrication.
///
/// With `CR1` alone — no `BRR` — there is no bit period, so there is no
/// waveform to publish and the wire channel is EMPTY, not "empty-looking".
/// This is not a gap in the wire channel: a UART whose baud generator is
/// unprogrammed transmits nothing on real silicon either, and narrating a
/// character at an invented rate would produce a trace measuring a frequency
/// the firmware never asked for. `Uart::bit_time_cycles` states that rule; this
/// is the wire-channel arm of it, alongside the pad arm above
/// (`diagnostic_af7_alone_is_not_enough_without_brr`).
#[test]
fn a_wire_probe_stays_silent_when_no_baud_divisor_was_ever_programmed() {
    let mut machine = lab_machine();
    firmware_rcc_init(&mut machine);
    legacy_usart2_init_cr1_only(&mut machine);

    let (wire, pad, _) =
        wire_and_pad_capture(&mut machine, "uart2", "TX", USART2_BASE, &ASYMMETRIC);

    assert!(
        wire.is_empty(),
        "no BRR ⇒ no timebase ⇒ no waveform; got {} edges, which would mean the \
         engine had invented a baud rate",
        wire.len(),
    );
    assert!(pad.is_empty(), "and the unmuxed pad is dark either way");
}

/// ACCEPTANCE 2 — UART4 and UART5.
///
/// `wire_stm32_uart_pads` iterates `1u8..=3`, so no pad on any STM32 has ever
/// been bound to UART4 or UART5 and no pad probe could reach them. That table
/// is UNCHANGED here — this test does not add a route, it stops needing one.
///
/// The STM32L476 has both instances (DS10198 Rev 11, Table 13, p53), and this
/// lab's firmware brings all four ports up: `firmware_all_ports_init` is the
/// four `PORT(...)` rows of `phy_labwired.c`, which program BRR on each.
#[test]
fn uart4_and_uart5_are_probeable_on_their_own_wire_with_no_pad_route() {
    for (name, base) in [("uart4", UART4_BASE), ("uart5", UART5_BASE)] {
        let mut machine = lab_machine();
        firmware_rcc_init(&mut machine);
        firmware_all_ports_init(&mut machine);

        let (wire, _, initial) = wire_and_pad_capture(&mut machine, name, "TX", base, &ASYMMETRIC);

        assert_eq!(
            initial[0],
            Some(true),
            "{name}.TX idles at mark before anything is sent",
        );
        assert!(
            !wire.is_empty(),
            "{name} transmitted {ASYMMETRIC:x?} and its wire probe captured \
             nothing — an instance outside the pad table is exactly the case a \
             wire channel exists to reach",
        );
        assert_eq!(
            decode_uart_8n1(true, &wire, u64::from(USARTDIV_COM2)),
            ASYMMETRIC.to_vec(),
            "{name}'s wire must decode to the characters it was given",
        );
    }
}

/// The wire probe is addressed by NAME, and a name it cannot resolve is an
/// ERROR — never channel zero.
///
/// Falling through to line 0 would draw a confident, correct-looking waveform
/// of the wrong signal: ask for `MISO` on a UART and get its TX trace back,
/// labelled MISO. That is the failure this whole naming layer exists to
/// prevent, so it is asserted rather than assumed.
#[test]
fn an_unknown_line_name_is_reported_and_never_silently_channel_zero() {
    use labwired_core::logic_capture::LogicRefError;

    let machine = lab_machine();

    assert_eq!(
        machine.resolve_wire_source("uart2", "tx"),
        machine.resolve_wire_source("uart2", "TX"),
        "line names match ignoring case — an engineer types `tx`",
    );
    assert_ne!(
        machine.resolve_wire_source("uart2", "TX").unwrap(),
        machine.resolve_wire_source("uart2", "RX").unwrap(),
        "TX and RX are different channels, not the same one twice",
    );

    match machine.resolve_wire_source("uart2", "MISO") {
        Err(LogicRefError::UnknownLine {
            peripheral,
            line,
            available,
        }) => {
            assert_eq!(peripheral, "uart2");
            assert_eq!(line, "MISO");
            assert_eq!(available, vec!["TX", "RX"], "and it says what IS there");
        }
        other => panic!("an unknown line name must be reported, got {other:?}"),
    }

    match machine.resolve_wire_source("no_such_peripheral", "TX") {
        Err(LogicRefError::UnknownPeripheral { peripheral }) => {
            assert_eq!(peripheral, "no_such_peripheral");
        }
        other => panic!("an unknown peripheral must be reported, got {other:?}"),
    }

    match machine.resolve_wire_source("gpioa", "TX") {
        Err(LogicRefError::NoWireLines { peripheral }) => assert_eq!(peripheral, "gpioa"),
        other => panic!("a peripheral with no wire must say so, got {other:?}"),
    }
}

/// A peer that really transmits: it hands over `bytes`, one per service tick,
/// and then falls silent forever.
///
/// This is the same seam a `uart_cross_link` binds through — the far end of the
/// IO-Link C/Q wire this lab's `env4.yaml` connects `uart2` to is a
/// `UartStreamDevice` exactly like this one. Nothing here reaches into the
/// UART: it only offers bytes, which is all a peer on a wire can do.
struct Peer {
    remaining: std::collections::VecDeque<u8>,
}

impl labwired_core::peripherals::uart::UartStreamDevice for Peer {
    fn poll(&mut self, _elapsed_us: u32) -> Option<u8> {
        self.remaining.pop_front()
    }
}

/// ACCEPTANCE 3 — RX is DRIVEN, and only where something drives it.
///
/// `LINE_RX` was read in four places and written in none, so the receive half
/// of every serial conversation was invisible on every family: a lab could
/// decode the request and never see the reply. It is now published from the
/// one place a character really arrives — the stream-device service loop, i.e.
/// a peer that actually transmitted.
///
/// The TX assertion is the guard against the cheap way to pass this: RX is not
/// a mirror of TX, and this UART transmits nothing at all here.
#[test]
fn a_peer_driving_the_wire_makes_rx_visible_to_a_wire_probe() {
    let mut machine = lab_machine();
    firmware_rcc_init(&mut machine);
    machine
        .bus
        .write_u32(USART2_BASE + BRR, USARTDIV_COM2)
        .unwrap();
    legacy_usart2_init_cr1_only(&mut machine);
    machine
        .bus
        .attach_uart_stream_by_id(
            "uart2",
            Box::new(Peer {
                remaining: ASYMMETRIC.into_iter().collect(),
            }),
        )
        .expect("attach a peer to the lab's C/Q port");

    let rx = machine.resolve_wire_source("uart2", "RX").unwrap();
    let tx = machine.resolve_wire_source("uart2", "TX").unwrap();
    let initial = machine.logic_watch(&[Some(rx), Some(tx)]);
    assert_eq!(
        initial,
        vec![Some(true), Some(true)],
        "both directions idle at mark before anyone speaks",
    );

    // Long enough for the peer to be polled three times and for the narrator to
    // have the wire time to paint three characters at 38.4 kbaud.
    for _ in 0..(USARTDIV_COM2 as u64 * 400) {
        machine.step().expect("step");
    }

    let batch = machine.logic_read_edges(0);
    let rx_edges = channel_edges(&batch, 0);
    let tx_edges = channel_edges(&batch, 1);

    assert!(
        !rx_edges.is_empty(),
        "a peer transmitted {ASYMMETRIC:x?} into this UART and the RX wire \
         probe stayed flat — the receive half is still unpublished",
    );
    assert_eq!(
        decode_uart_8n1(true, &rx_edges, u64::from(USARTDIV_COM2)),
        ASYMMETRIC.to_vec(),
        "RX must carry exactly what the peer sent, LSB-first at the programmed \
         divisor",
    );
    assert!(
        tx_edges.is_empty(),
        "this UART transmitted nothing, so TX must be flat; {} edges there \
         would mean RX is being mirrored onto TX (or the two line names are \
         swapped) rather than each direction being published from its own \
         source",
        tx_edges.len(),
    );
}

/// The other half of criterion 3, and the one that keeps it honest: where
/// NOTHING drives RX, the line stays silent.
///
/// Identical setup, no peer. A fabricated RX waveform would be worse than an
/// empty channel — it would show a conversation that never happened — so the
/// absence is asserted, not assumed.
#[test]
fn rx_stays_silent_when_nothing_is_driving_it() {
    let mut machine = lab_machine();
    firmware_rcc_init(&mut machine);
    machine
        .bus
        .write_u32(USART2_BASE + BRR, USARTDIV_COM2)
        .unwrap();
    legacy_usart2_init_cr1_only(&mut machine);

    let rx = machine.resolve_wire_source("uart2", "RX").unwrap();
    machine.logic_watch(&[Some(rx)]);
    // The firmware transmits the whole time; RX must not move because of it.
    for &byte in &ASYMMETRIC {
        machine.bus.write_u8(USART2_BASE + TDR, byte).unwrap();
        for _ in 0..(USARTDIV_COM2 as u64 * 12) {
            machine.step().expect("step");
        }
    }
    for _ in 0..(USARTDIV_COM2 as u64 * 100) {
        machine.step().expect("step");
    }

    assert!(
        machine.logic_read_edges(0).edges.is_empty(),
        "nothing drove RX, so RX must be flat — an invented level here is the \
         failure mode this criterion exists to forbid",
    );
}

/// The two kinds coexist on ONE wire cell, and neither erases the other.
///
/// This is the hazard `PadLines::merge_tap` exists for, arriving from a new
/// direction. A wire channel registers on the same cell a routed pad's
/// `PadRoutes` registers on; installing rather than merging would wipe the
/// pad's channels and leave it silently empty while the levels still read
/// correctly — the exact failure mode that already cost this codebase a
/// silently-empty SPI data channel on the STM32WBA52.
///
/// So: mux PA2 properly (the repaired firmware's own sequence), watch the PAD
/// and the WIRE at once, and require both to carry the same characters.
#[test]
fn a_routed_pad_and_the_wire_behind_it_capture_the_same_traffic_at_once() {
    let mut machine = lab_machine();
    firmware_rcc_init(&mut machine);
    firmware_all_ports_init(&mut machine);

    let (wire, pad, initial) =
        wire_and_pad_capture(&mut machine, "uart2", "TX", USART2_BASE, &ASYMMETRIC);

    assert_eq!(
        initial,
        vec![Some(true), Some(true)],
        "PA2 is muxed to AF7 now, so the pad reads the WIRE's idle mark rather \
         than GPIOA's output latch",
    );
    assert_eq!(
        decode_uart_8n1(true, &wire, u64::from(USARTDIV_COM2)),
        ASYMMETRIC.to_vec(),
        "the wire channel carries the characters",
    );
    assert_eq!(
        decode_uart_8n1(true, &pad, u64::from(USARTDIV_COM2)),
        ASYMMETRIC.to_vec(),
        "and so does the pad channel — a wire probe must not disarm the pad \
         probe watching the same signal",
    );
    assert_eq!(
        wire, pad,
        "one wire, one set of transitions: watched two ways in one capture, \
         the two channels must be cycle-for-cycle identical",
    );
}
