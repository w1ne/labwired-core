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
    machine.logic_watch(&[Some((gpioa, TX_PIN))]);

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
