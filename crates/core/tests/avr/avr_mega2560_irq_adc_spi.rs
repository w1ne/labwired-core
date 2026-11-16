// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ATmega2560 external interrupts, pin-change interrupts, the ADC's timed,
//! auto-triggered and interrupt-driven conversions, and the SPI block, each
//! checked against Microchip DS2549 (ATmega640/1280/1281/2560/2561):
//!
//! - "Interrupts", Table 14-1 (Reset and Interrupt Vectors): INT0..INT7 are
//!   vectors 2..9, PCINT0..2 are 10..12, TIMER0 OVF 24, SPI STC 25, ADC 30.
//! - "External Interrupts": EICRA/EICRB sense control, EIMSK, EIFR, PCICR,
//!   PCIFR, PCMSK0..2 and the pins (INT0..3 on PD0..3, INT4..7 on PE4..7,
//!   PCINT0..7 on PB0..7, PCINT8 on PE0, PCINT9..15 on PJ0..6, PCINT16..23 on
//!   PK0..7).
//! - "ADC": conversion timing (Table 26-1), prescaler (ADPS), free running
//!   and auto-trigger sources (ADCSRB.ADTS, Table 26-6), ADIF/ADIE, the
//!   ADCL/ADCH access lock, input channel (MUX5:0, Table 26-4), voltage
//!   reference (REFS1:0, Table 26-3), ADLAR.
//! - "SPI": SPCR/SPSR/SPDR, SCK frequency (Table 21-5), SPIF/WCOL clearing,
//!   DORD, SS pin in master and slave mode, the SPI interrupt.
//!
//! The programs are hand-assembled so each test states exactly what firmware
//! does; the chip is the shipped `configs/chips/atmega2560.yaml`, so the pads
//! and the ADC inputs are the bus-side models every board uses.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::{Avr, AvrModel};
use labwired_core::{Cpu, Machine};
use std::path::PathBuf;

// ---- data-space addresses (DS2549 "Register Summary") ----
const PINB: u16 = 0x23;
const DDRB: u16 = 0x24;
const PORTB: u16 = 0x25;
const PCIFR: u16 = 0x3B;
const EIFR: u16 = 0x3C;
const EIMSK: u16 = 0x3D;
const TCCR0B: u16 = 0x45;
const SPCR: u16 = 0x4C;
const SPSR: u16 = 0x4D;
const SPDR: u16 = 0x4E;
const SMCR: u16 = 0x53;
const PCICR: u16 = 0x68;
const EICRA: u16 = 0x69;
const EICRB: u16 = 0x6A;
const PCMSK0: u16 = 0x6B;
const PCMSK1: u16 = 0x6C;
const PCMSK2: u16 = 0x6D;
const TIMSK0: u16 = 0x6E;
const ADCL: u16 = 0x78;
const ADCH: u16 = 0x79;
const ADCSRA: u16 = 0x7A;
const ADCSRB: u16 = 0x7B;
const ADMUX: u16 = 0x7C;

// ---- Table 14-1 vector numbers (handler at byte (n - 1) * 4) ----
const VEC_INT0: u32 = 2;
const VEC_PCINT0: u32 = 10;
const VEC_TIMER0_OVF: u32 = 24;
const VEC_SPI_STC: u32 = 25;
const VEC_ADC: u32 = 30;

const MAIN: u32 = 0x200;
const HANDLERS: u32 = 0x800;

// ---- a tiny assembler: just the instructions these programs use ----
fn ldi(d: u16, k: u8) -> u16 {
    assert!((16..32).contains(&d));
    let k = u16::from(k);
    0xE000 | ((k & 0xF0) << 4) | ((d - 16) << 4) | (k & 0x0F)
}
fn inc(d: u16) -> u16 {
    0x9403 | (d << 4)
}
fn dec(d: u16) -> u16 {
    0x940A | (d << 4)
}
fn sbrs(r: u16, b: u16) -> u16 {
    0xFE00 | (r << 4) | b
}
fn sbrc(r: u16, b: u16) -> u16 {
    0xFC00 | (r << 4) | b
}
fn rjmp(k: i16) -> u16 {
    0xC000 | (k as u16 & 0x0FFF)
}
fn brne(k: i16) -> u16 {
    0xF401 | ((k as u16 & 0x7F) << 3)
}
const SEI: u16 = 0x9478;
const RETI: u16 = 0x9518;
const SLEEP: u16 = 0x9588;
const NOP: u16 = 0x0000;

#[derive(Default)]
struct Prog {
    /// (byte address, words)
    chunks: Vec<(u32, Vec<u16>)>,
}

impl Prog {
    /// Start a code block at byte `at`.
    fn at(&mut self, at: u32) -> &mut Vec<u16> {
        self.chunks.push((at, Vec::new()));
        &mut self.chunks.last_mut().unwrap().1
    }
    /// `INC rd; RETI` in the slot of vector `vec`.
    fn count(&mut self, vec: u32, rd: u16) {
        self.at((vec - 1) * 4).extend([inc(rd), RETI]);
    }
    /// `JMP target` in the slot of vector `vec`.
    fn jmp(&mut self, vec: u32, target: u32) {
        self.at((vec - 1) * 4).extend([0x940C, (target / 2) as u16]);
    }
}

trait Emit {
    fn sts(&mut self, addr: u16, value: u8);
    fn lds(&mut self, d: u16, addr: u16);
    fn here(&self, base: u32) -> u32;
    /// `LDS r17, addr; SBRS/SBRC r17, bit; RJMP back`: wait for `bit` to read `level`.
    fn wait_bit(&mut self, addr: u16, bit: u16, level: bool);
    fn spin(&mut self);
}

impl Emit for Vec<u16> {
    /// `LDI r16, value; STS addr, r16`.
    fn sts(&mut self, addr: u16, value: u8) {
        self.extend([ldi(16, value), 0x9200 | (16 << 4), addr]);
    }
    fn lds(&mut self, d: u16, addr: u16) {
        self.extend([0x9000 | (d << 4), addr]);
    }
    fn here(&self, base: u32) -> u32 {
        base + 2 * self.len() as u32
    }
    fn wait_bit(&mut self, addr: u16, bit: u16, level: bool) {
        self.lds(17, addr);
        self.push(if level { sbrs(17, bit) } else { sbrc(17, bit) });
        self.push(rjmp(-4));
    }
    fn spin(&mut self) {
        self.push(rjmp(-1));
    }
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The shipped ATmega2560 chip with `extra` appended to the system yaml.
fn machine(prog: &Prog, extra: &str) -> Machine<Avr> {
    let chip_path = root().join("configs/chips/atmega2560.yaml");
    let yaml = format!(
        "name: mega-irq-adc-spi\nchip: {}\ncpu_hz: 16000000\n{extra}",
        chip_path.display()
    );
    let manifest = SystemManifest::from_yaml(&yaml).expect("system yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("atmega2560 chip");
    let bus = SystemBus::from_config(&chip, &manifest).expect("bus");
    let mut cpu = Avr::for_chip_name(&chip.name);
    assert_eq!(cpu.model, AvrModel::Atmega2560);
    for (at, words) in &prog.chunks {
        cpu.load_words(*at, words);
    }
    cpu.set_pc(MAIN);
    Machine::new(cpu, bus)
}

fn pad(m: &mut Machine<Avr>, port: &str, pin: u8, level: bool) {
    let idx = m
        .bus
        .find_peripheral_index_by_name(port)
        .unwrap_or_else(|| panic!("no {port}"));
    assert!(m.bus.peripherals[idx].dev.set_gpio_input(pin, level));
}

fn adc_in(m: &mut Machine<Avr>, channel: u8, mv: u16) {
    let idx = m.bus.find_peripheral_index_by_name("adc").expect("adc");
    assert!(m.bus.peripherals[idx]
        .dev
        .set_adc_channel_input(channel, mv));
}

fn steps(m: &mut Machine<Avr>, n: usize) {
    for _ in 0..n {
        m.step().expect("step");
    }
}

/// The 10-bit result for `mv` against a `ref_mv` reference:
/// ADC = Vin * 1024 / Vref (DS2549 "ADC Conversion Result").
fn code(mv: u32, ref_mv: u32) -> u16 {
    (mv * 1024 / ref_mv).min(1023) as u16
}

fn run_to_cycle(m: &mut Machine<Avr>, cycle: u64) {
    while m.cpu.cycles < cycle {
        m.step().expect("step");
    }
}

/// Step until PC is `at`; returns the cycle count there.
fn run_to_pc(m: &mut Machine<Avr>, at: u32) -> u64 {
    for _ in 0..2_000_000 {
        if m.cpu.get_pc() == at {
            return m.cpu.cycles;
        }
        m.step().expect("step");
    }
    panic!("never reached {at:#x}; pc {:#x}", m.cpu.get_pc());
}

// =====================================================================
// External interrupts INT0..INT7
// =====================================================================

/// INT0..INT3 are PD0..PD3 (vectors 2..5). EICRA ISCn1:0 = 11 is a rising
/// edge. PD2, the 328P's INT0 pad, is INT2 here.
#[test]
fn int0_to_int3_sit_on_pd0_to_pd3_with_vectors_2_to_5() {
    let mut p = Prog::default();
    for n in 0..4 {
        p.count(VEC_INT0 + n, 20 + n as u16);
    }
    let main = p.at(MAIN);
    main.sts(EICRA, 0xFF);
    main.sts(EIMSK, 0x0F);
    main.push(SEI);
    main.spin();
    let mut m = machine(&p, "");
    steps(&mut m, 20);
    for n in 0..4u8 {
        pad(&mut m, "portd", n, true);
        steps(&mut m, 10);
        let counts: Vec<u8> = (20..24).map(|r| m.cpu.r[r]).collect();
        let want: Vec<u8> = (0..4).map(|k| u8::from(k <= n)).collect();
        assert_eq!(counts, want, "after PD{n} rose");
    }
}

/// INT4..INT7 are PE4..PE7 (vectors 6..9); EICRB ISC41:40 = 10 is a falling
/// edge, so the rising edge does nothing.
#[test]
fn int4_falling_edge_on_pe4_enters_vector_6_only_on_the_fall() {
    let mut p = Prog::default();
    p.count(6, 20);
    let main = p.at(MAIN);
    main.sts(EICRB, 0b10);
    main.sts(EIMSK, 1 << 4);
    main.push(SEI);
    main.spin();
    let mut m = machine(&p, "");
    pad(&mut m, "porte", 4, true);
    steps(&mut m, 20);
    assert_eq!(m.cpu.r[20], 0);
    pad(&mut m, "porte", 4, false);
    steps(&mut m, 10);
    assert_eq!(m.cpu.r[20], 1, "falling edge on PE4");
    pad(&mut m, "porte", 4, true);
    steps(&mut m, 10);
    assert_eq!(m.cpu.r[20], 1, "rising edge must not trigger falling sense");
    // Some other port E pin does nothing.
    pad(&mut m, "porte", 3, true);
    pad(&mut m, "porte", 3, false);
    steps(&mut m, 10);
    assert_eq!(m.cpu.r[20], 1);
}

/// ISCn1:0 = 01: any logical change (INT3 via EICRA, INT6 via EICRB).
#[test]
fn any_edge_sense_counts_both_edges_on_int3_and_int6() {
    let mut p = Prog::default();
    p.count(5, 20); // INT3
    p.count(8, 21); // INT6
    let main = p.at(MAIN);
    main.sts(EICRA, 0b01 << 6);
    main.sts(EICRB, 0b01 << 4);
    main.sts(EIMSK, (1 << 3) | (1 << 6));
    main.push(SEI);
    main.spin();
    let mut m = machine(&p, "");
    steps(&mut m, 20);
    for level in [true, false, true] {
        pad(&mut m, "portd", 3, level);
        pad(&mut m, "porte", 6, level);
        steps(&mut m, 10);
    }
    assert_eq!((m.cpu.r[20], m.cpu.r[21]), (3, 3));
}

/// EIFR: a matching edge sets INTFn even while EIMSK masks it; writing one
/// clears it; enabling with the flag set enters the vector at once, and the
/// vector entry clears the flag.
#[test]
fn intf_latches_while_masked_clears_on_write_one_and_on_vector_entry() {
    let mut p = Prog::default();
    p.count(7, 20); // INT5
    let main = p.at(MAIN);
    main.sts(EICRB, 0b10 << 2); // INT5 falling
    main.push(SEI);
    let wait1 = main.here(MAIN);
    main.push(NOP);
    main.lds(24, EIFR);
    main.sts(EIFR, 1 << 5);
    main.lds(25, EIFR);
    let wait2 = main.here(MAIN);
    main.push(NOP);
    main.sts(EIMSK, 1 << 5);
    main.push(NOP);
    main.lds(26, EIFR);
    main.spin();
    let mut m = machine(&p, "");
    pad(&mut m, "porte", 5, true);
    run_to_pc(&mut m, wait1);
    pad(&mut m, "porte", 5, false);
    m.step().unwrap();
    pad(&mut m, "porte", 5, true);
    run_to_pc(&mut m, wait2);
    assert_eq!(m.cpu.r[24] & (1 << 5), 1 << 5, "INTF5 set while masked");
    assert_eq!(m.cpu.r[25] & (1 << 5), 0, "writing one clears INTF5");
    assert_eq!(m.cpu.r[20], 0);
    pad(&mut m, "porte", 5, false);
    steps(&mut m, 12);
    assert_eq!(
        m.cpu.r[20], 1,
        "enabled with the flag set: vector 7 entered"
    );
    assert_eq!(m.cpu.r[26] & (1 << 5), 0, "vector entry cleared INTF5");
}

/// ISCn1:0 = 00: low level. The request holds for as long as the pad is low,
/// and EIFR stays clear ("always cleared when INTn is configured as a level
/// interrupt").
#[test]
fn int7_low_level_requests_while_low_and_sets_no_flag() {
    let mut p = Prog::default();
    p.count(9, 20); // INT7
    let main = p.at(MAIN);
    main.sts(EICRB, 0);
    main.sts(EIMSK, 1 << 7);
    main.push(SEI);
    let lp = main.here(MAIN);
    main.lds(24, EIFR);
    main.push(rjmp(-3));
    let _ = lp;
    let mut m = machine(&p, "");
    pad(&mut m, "porte", 7, true);
    steps(&mut m, 30);
    assert_eq!(m.cpu.r[20], 0);
    pad(&mut m, "porte", 7, false);
    steps(&mut m, 60);
    assert!(m.cpu.r[20] >= 3, "re-entered while low: {}", m.cpu.r[20]);
    assert_eq!(m.cpu.r[24] & 0x80, 0, "INTF7 reads 0 in level mode");
    pad(&mut m, "porte", 7, true);
    steps(&mut m, 10);
    let held = m.cpu.r[20];
    steps(&mut m, 60);
    assert_eq!(m.cpu.r[20], held, "released: no more requests");
}

/// "Low level interrupts and the edge interrupt on INT3:0 are detected
/// asynchronously"; INT7:4 edges need the I/O clock. In power-down a falling
/// INT5 edge does not wake the core, a falling INT2 edge does.
#[test]
fn power_down_wakes_on_an_int2_edge_but_not_on_an_int5_edge() {
    let mut p = Prog::default();
    p.count(4, 20); // INT2
    p.count(7, 21); // INT5
    let main = p.at(MAIN);
    main.sts(EICRA, 0b10 << 4);
    main.sts(EICRB, 0b10 << 2);
    main.sts(EIMSK, (1 << 2) | (1 << 5));
    main.sts(SMCR, (0b010 << 1) | 1); // power-down, SE
    main.push(SEI);
    main.push(SLEEP);
    main.push(inc(22));
    main.spin();
    let mut m = machine(&p, "");
    pad(&mut m, "portd", 2, true);
    pad(&mut m, "porte", 5, true);
    steps(&mut m, 30);
    assert!(m.cpu.sleeping, "asleep in power-down");
    pad(&mut m, "porte", 5, false);
    steps(&mut m, 30);
    assert!(m.cpu.sleeping, "INT5 edge needs clk_I/O");
    assert_eq!(m.cpu.r[21], 0);
    pad(&mut m, "portd", 2, false);
    steps(&mut m, 30);
    assert!(!m.cpu.sleeping);
    assert_eq!((m.cpu.r[20], m.cpu.r[22]), (1, 1), "INT2 woke the core");
}

// =====================================================================
// Pin-change interrupts PCINT0..23
// =====================================================================

/// PCINT0..7 = PB0..7 (PCMSK0, vector 10), PCINT8 = PE0 and PCINT9..15 =
/// PJ0..6 (PCMSK1, vector 11), PCINT16..23 = PK0..7 (PCMSK2, vector 12).
/// Pins whose PCMSK bit is clear do nothing.
#[test]
fn pcint_groups_sit_on_port_b_pe0_port_j_and_port_k() {
    let mut p = Prog::default();
    p.count(VEC_PCINT0, 20);
    p.count(VEC_PCINT0 + 1, 21);
    p.count(VEC_PCINT0 + 2, 22);
    let main = p.at(MAIN);
    main.sts(PCMSK0, 1 << 3); // PCINT3 = PB3
    main.sts(PCMSK1, 0b11 | (1 << 7)); // PCINT8 = PE0, PCINT9 = PJ0, PCINT15 = PJ6
    main.sts(PCMSK2, 1 << 7); // PCINT23 = PK7
    main.sts(PCICR, 0x07);
    main.push(SEI);
    main.spin();
    let mut m = machine(&p, "");
    steps(&mut m, 30);
    let counts = |m: &Machine<Avr>| (m.cpu.r[20], m.cpu.r[21], m.cpu.r[22]);
    let toggle = |m: &mut Machine<Avr>, port: &str, pin: u8| {
        pad(m, port, pin, true);
        steps(m, 10);
    };
    toggle(&mut m, "portb", 4); // not in PCMSK0
    toggle(&mut m, "porte", 1); // PE1 is not a PCINT pin
    toggle(&mut m, "portj", 7); // PJ7 is not a PCINT pin
    assert_eq!(counts(&m), (0, 0, 0));
    toggle(&mut m, "portb", 3);
    assert_eq!(counts(&m), (1, 0, 0), "PB3 -> PCINT0 vector");
    toggle(&mut m, "porte", 0);
    assert_eq!(counts(&m), (1, 1, 0), "PE0 -> PCINT1 vector");
    toggle(&mut m, "portj", 0);
    assert_eq!(counts(&m), (1, 2, 0), "PJ0 -> PCINT1 vector");
    toggle(&mut m, "portj", 6);
    assert_eq!(counts(&m), (1, 3, 0), "PJ6 (PCINT15) -> PCINT1 vector");
    toggle(&mut m, "portk", 7);
    assert_eq!(counts(&m), (1, 3, 1), "PK7 -> PCINT2 vector");
}

/// PCIFn latches a change on a masked-in pin while PCICR is clear; writing
/// one clears it.
#[test]
fn pcif_latches_while_pcicr_is_clear_and_clears_on_write_one() {
    let mut p = Prog::default();
    let main = p.at(MAIN);
    main.sts(PCMSK2, 1 << 0); // PK0
    let wait = main.here(MAIN);
    main.push(NOP);
    main.lds(24, PCIFR);
    main.sts(PCIFR, 1 << 2);
    main.lds(25, PCIFR);
    main.spin();
    let mut m = machine(&p, "");
    run_to_pc(&mut m, wait);
    pad(&mut m, "portk", 0, true);
    steps(&mut m, 8);
    assert_eq!(m.cpu.r[24] & 0x07, 1 << 2);
    assert_eq!(m.cpu.r[25] & 0x07, 0);
}

// =====================================================================
// ADC
// =====================================================================

const ADEN: u8 = 1 << 7;
const ADSC: u8 = 1 << 6;
const ADATE: u8 = 1 << 5;
const ADIF: u8 = 1 << 4;
const ADIE: u8 = 1 << 3;

/// Table 26-1: the first conversion after ADEN takes 25 ADC clocks, a normal
/// one 13; the ADC clock is the CPU clock over the ADPS division factor
/// (Table 26-5). "The prescaler starts counting from the moment the ADC is
/// switched on" and a conversion "starts at the following rising edge of the
/// ADC clock", so up to one more ADC clock passes before it starts.
/// ADSC reads one until the conversion completes; then ADIF is set.
#[test]
fn conversion_takes_25_then_13_adc_clocks_at_the_adps_division() {
    for (adps, div) in [(7u8, 128u64), (2, 4)] {
        let mut p = Prog::default();
        let main = p.at(MAIN);
        main.sts(ADMUX, 0x40);
        main.sts(ADCSRA, ADEN | adps);
        main.sts(ADCSRA, ADEN | ADSC | adps);
        let first = main.here(MAIN);
        main.wait_bit(ADCSRA, 6, false);
        let first_done = main.here(MAIN);
        main.lds(24, ADCSRA);
        main.sts(ADCSRA, ADEN | ADSC | ADIF | adps);
        let second = main.here(MAIN);
        main.wait_bit(ADCSRA, 6, false);
        let second_done = main.here(MAIN);
        main.spin();
        let mut m = machine(&p, "");
        let t0 = run_to_pc(&mut m, first);
        let t1 = run_to_pc(&mut m, first_done);
        let t2 = run_to_pc(&mut m, second);
        let t3 = run_to_pc(&mut m, second_done);
        let (a, b) = (t1 - t0, t3 - t2);
        assert!(
            (25 * div..=26 * div + 8).contains(&a),
            "ADPS={adps}: first conversion {a} cycles, want 25 x {div}"
        );
        assert!(
            (13 * div..=14 * div + 8).contains(&b),
            "ADPS={adps}: second conversion {b} cycles, want 13 x {div}"
        );
        assert_ne!(
            m.cpu.r[24] & ADIF,
            0,
            "ADIF set when the conversion completes"
        );
    }
}

/// MUX5 (ADCSRB bit 3) with MUX4:0 selects ADC8..15 (Table 26-4); REFS1:0
/// pick AVCC (01), internal 1.1 V (10) or internal 2.56 V (11) (Table 26-3);
/// ADLAR left-adjusts and "affects the ADC Data Register immediately".
#[test]
fn mux5_channel_reference_and_left_adjust() {
    // (ADCSRB, ADMUX) per conversion; results land in r20.. pairs (L, H).
    let cases: [(u8, u8); 5] = [
        (0x00, 0x45), // ADC5, AVCC
        (0x08, 0x45), // ADC13, AVCC
        (0x08, 0x85), // ADC13, 1.1 V
        (0x08, 0xC5), // ADC13, 2.56 V
        (0x08, 0x65), // ADC13, AVCC, ADLAR
    ];
    let mut p = Prog::default();
    let main = p.at(MAIN);
    main.sts(ADCSRA, ADEN | 2);
    for (i, (b, mux)) in cases.iter().enumerate() {
        main.sts(ADCSRB, *b);
        main.sts(ADMUX, *mux);
        main.sts(ADCSRA, ADEN | ADSC | ADIF | 2);
        main.wait_bit(ADCSRA, 6, false);
        main.lds(2 + 2 * i as u16, ADCL);
        main.lds(3 + 2 * i as u16, ADCH);
    }
    // ADLAR off again with no new conversion: the same result right-adjusted.
    main.sts(ADMUX, 0x45);
    main.lds(12, ADCL);
    main.lds(13, ADCH);
    let done = main.here(MAIN);
    main.spin();
    let mut m = machine(&p, "");
    adc_in(&mut m, 5, 0);
    adc_in(&mut m, 13, 550);
    run_to_pc(&mut m, done);
    let word = |i: usize| u16::from(m.cpu.r[2 + 2 * i]) | u16::from(m.cpu.r[3 + 2 * i]) << 8;
    assert_eq!(word(0), 0, "ADC5 is grounded");
    assert_eq!(word(1), code(550, 5000), "ADC13 against AVCC");
    assert_eq!(word(2), 512, "550 mV against the 1.1 V reference");
    assert_eq!(word(3), code(550, 2560), "against 2.56 V");
    assert_eq!(word(4), code(550, 5000) << 6, "ADLAR left-adjusts");
    assert_eq!(
        word(5),
        code(550, 5000),
        "clearing ADLAR re-presents at once"
    );
}

/// "The channel and reference selection is continuously updated until a
/// conversion is started. Once the conversion starts, the channel and
/// reference selection is locked."
#[test]
fn admux_written_during_a_conversion_applies_to_the_next_one() {
    let mut p = Prog::default();
    let main = p.at(MAIN);
    main.sts(ADMUX, 0x41);
    main.sts(ADCSRA, ADEN | ADSC | 2);
    main.sts(ADMUX, 0x42);
    main.wait_bit(ADCSRA, 6, false);
    main.lds(20, ADCL);
    main.lds(21, ADCH);
    main.sts(ADCSRA, ADEN | ADSC | ADIF | 2);
    main.wait_bit(ADCSRA, 6, false);
    main.lds(22, ADCL);
    main.lds(23, ADCH);
    let done = main.here(MAIN);
    main.spin();
    let mut m = machine(&p, "");
    adc_in(&mut m, 1, 5000);
    adc_in(&mut m, 2, 0);
    run_to_pc(&mut m, done);
    assert_eq!((m.cpu.r[20], m.cpu.r[21]), (0xFF, 0x03), "ADC1 was latched");
    assert_eq!(
        (m.cpu.r[22], m.cpu.r[23]),
        (0, 0),
        "next conversion uses ADC2"
    );
}

/// Free running (ADATE with ADTS = 0) with ADIE: the first conversion starts
/// at the first ADC clock edge after ADEN and ADSC are set together and takes
/// 25 clocks; then a new one starts as each completes, every 13 ADC clocks, and
/// each raises the ADC vector (30). ADSC reads one throughout. The handler
/// sees the input as it is at each conversion.
#[test]
fn free_running_conversions_interrupt_every_13_adc_clocks() {
    let mut p = Prog::default();
    p.jmp(VEC_ADC, HANDLERS);
    let isr = p.at(HANDLERS);
    isr.lds(24, ADCL);
    isr.lds(25, ADCH);
    isr.push(inc(20));
    isr.push(RETI);
    let main = p.at(MAIN);
    main.sts(ADMUX, 0x40);
    main.sts(ADCSRB, 0);
    main.sts(ADCSRA, ADEN | ADSC | ADATE | ADIE | 7);
    let started = main.here(MAIN);
    main.push(SEI);
    main.lds(18, ADCSRA);
    main.push(rjmp(-3));
    let mut m = machine(&p, "");
    adc_in(&mut m, 0, 2000);
    let t0 = run_to_pc(&mut m, started);
    let n = 12u64;
    // Conversion k (1-based) completes at about t0 + 26 x 128 + (k - 1) x 13 x 128.
    run_to_cycle(&mut m, t0 + 26 * 128 + (n - 1) * 13 * 128 + 64);
    assert_eq!(
        u64::from(m.cpu.r[20]),
        n,
        "one ADC interrupt per conversion"
    );
    run_to_cycle(&mut m, t0 + 26 * 128 + n * 13 * 128 - 64);
    assert_eq!(
        u64::from(m.cpu.r[20]),
        n,
        "no interrupt between conversions"
    );
    let result = |m: &Machine<Avr>| u16::from(m.cpu.r[24]) | u16::from(m.cpu.r[25]) << 8;
    assert_eq!(result(&m), code(2000, 5000));
    assert_ne!(m.cpu.r[18] & ADSC, 0, "ADSC reads one in free running mode");
    adc_in(&mut m, 0, 4000);
    let later = m.cpu.cycles + 3 * 13 * 128;
    run_to_cycle(&mut m, later);
    assert_eq!(result(&m), code(4000, 5000));
}

/// "Once ADCL is read, ADC access to Data Registers is blocked ... if ADCL
/// has been read, and a conversion completes before ADCH is read, neither
/// register is updated and the result from the conversion is lost. When ADCH
/// is read, ADC access to the ADCH and ADCL Registers is re-enabled."
#[test]
fn reading_adcl_locks_the_data_registers_until_adch_is_read() {
    let mut p = Prog::default();
    let main = p.at(MAIN);
    main.sts(ADMUX, 0x40);
    main.sts(ADCSRA, ADEN | ADSC | ADATE | 2); // free running, /4: 52 cycles
    main.extend([ldi(18, 60), dec(18), brne(-2)]);
    main.lds(20, ADCL);
    let changed = main.here(MAIN);
    main.extend([ldi(18, 100), dec(18), brne(-2)]); // > 5 conversions
    main.lds(21, ADCH);
    main.extend([ldi(18, 60), dec(18), brne(-2)]);
    main.lds(22, ADCL);
    main.lds(23, ADCH);
    let done = main.here(MAIN);
    main.spin();
    let mut m = machine(&p, "");
    adc_in(&mut m, 0, 1000); // 204 = 0x0CC
    run_to_pc(&mut m, changed);
    adc_in(&mut m, 0, 4000); // 819 = 0x333
    run_to_pc(&mut m, done);
    assert_eq!((m.cpu.r[20], m.cpu.r[21]), (0xCC, 0x00), "locked pair");
    assert_eq!((m.cpu.r[22], m.cpu.r[23]), (0x33, 0x03), "unlocked again");
}

/// Auto trigger on Timer0 overflow (ADTS = 100): a conversion starts on the
/// rising edge of TOV0. "The Interrupt Flag must be cleared in order to
/// trigger a new conversion at the next interrupt event": with TOV0 left set
/// one conversion runs; with the Timer0 vector clearing it, one per overflow.
#[test]
fn timer0_overflow_auto_trigger_needs_tov0_cleared_to_retrigger() {
    for timer_isr in [false, true] {
        let mut p = Prog::default();
        p.count(VEC_ADC, 20);
        p.count(VEC_TIMER0_OVF, 21);
        let main = p.at(MAIN);
        main.sts(ADMUX, 0x40);
        main.sts(ADCSRB, 0b100);
        main.sts(ADCSRA, ADEN | ADATE | ADIE | 2); // no ADSC: the trigger starts it
        if timer_isr {
            main.sts(TIMSK0, 1);
        }
        main.sts(TCCR0B, 1); // clk/1: overflow every 256 cycles
        let started = main.here(MAIN);
        main.push(SEI);
        main.spin();
        let mut m = machine(&p, "");
        let t0 = run_to_pc(&mut m, started);
        run_to_cycle(&mut m, t0 + 10 * 256 + 128);
        if timer_isr {
            let (adc, ovf) = (m.cpu.r[20], m.cpu.r[21]);
            assert!(ovf >= 9, "timer0 overflows: {ovf}");
            assert!(
                adc + 1 >= ovf && adc <= ovf,
                "one conversion per overflow: adc {adc}, ovf {ovf}"
            );
        } else {
            assert_eq!(m.cpu.r[20], 1, "TOV0 never cleared: a single trigger");
        }
    }
}

// =====================================================================
// SPI
// =====================================================================

const SPIE: u8 = 1 << 7;
const SPE: u8 = 1 << 6;
const DORD: u8 = 1 << 5;
const MSTR: u8 = 1 << 4;
const SPIF: u8 = 1 << 7;
const WCOL: u8 = 1 << 6;

/// Table 21-5: SCK = fosc / {4, 16, 64, 128} for SPR1:0, halved by SPI2X.
/// A byte is eight SCK periods; SPIF is set when it is done.
#[test]
fn spif_sets_after_eight_sck_periods_of_the_selected_divider() {
    for (spr, spi2x, div) in [(0u8, 0u8, 4u64), (3, 0, 128), (0, 1, 2), (1, 1, 8)] {
        let mut p = Prog::default();
        let main = p.at(MAIN);
        main.sts(DDRB, 0b0111);
        main.sts(PORTB, 0b0001);
        main.sts(SPCR, SPE | MSTR | spr);
        main.sts(SPSR, spi2x);
        main.sts(SPDR, 0xA5);
        let start = main.here(MAIN);
        main.wait_bit(SPSR, 7, true);
        let done = main.here(MAIN);
        main.spin();
        let mut m = machine(&p, "");
        let t0 = run_to_pc(&mut m, start);
        let t1 = run_to_pc(&mut m, done);
        assert!(
            (8 * div..=8 * div + 6).contains(&(t1 - t0)),
            "SPR={spr} SPI2X={spi2x}: {} cycles, want 8 x {div}",
            t1 - t0
        );
    }
}

/// "SPIF is cleared by hardware when executing the corresponding interrupt
/// handling vector. Alternatively, the SPIF bit is cleared by first reading
/// the SPI Status Register with SPIF set, then accessing the SPI Data
/// Register." SPSR bits 7..1 are read-only, so writing one there clears
/// nothing. Writing SPDR during a transfer sets WCOL and is ignored.
#[test]
fn spif_and_wcol_clear_by_reading_spsr_then_accessing_spdr() {
    let mut p = Prog::default();
    let main = p.at(MAIN);
    main.sts(DDRB, 0b0111);
    main.sts(SPCR, SPE | MSTR | 3);
    main.sts(SPDR, 0x11);
    main.sts(SPDR, 0x22); // collides
    main.lds(20, SPSR);
    main.wait_bit(SPSR, 7, true);
    main.sts(SPSR, SPIF | WCOL);
    main.lds(21, SPSR);
    main.lds(22, SPDR);
    main.lds(23, SPSR);
    let done = main.here(MAIN);
    main.spin();
    let mut m = machine(&p, "");
    run_to_pc(&mut m, done);
    assert_eq!(
        m.cpu.r[20] & (SPIF | WCOL),
        WCOL,
        "WCOL on a write mid-transfer"
    );
    assert_eq!(
        m.cpu.r[21] & (SPIF | WCOL),
        SPIF | WCOL,
        "write-one clears nothing"
    );
    assert_eq!(m.cpu.r[23] & (SPIF | WCOL), 0, "SPSR read then SPDR access");
}

/// SPIE: a completed byte enters the SPI STC vector (25), which clears SPIF.
#[test]
fn spi_transfer_complete_enters_vector_25_and_clears_spif() {
    let mut p = Prog::default();
    p.count(VEC_SPI_STC, 20);
    let main = p.at(MAIN);
    main.sts(DDRB, 0b0111);
    main.sts(SPCR, SPIE | SPE | MSTR);
    main.push(SEI);
    main.sts(SPDR, 0x5A);
    let lp = main.here(MAIN);
    main.lds(21, SPSR);
    main.push(rjmp(-3));
    let _ = lp;
    let mut m = machine(&p, "");
    steps(&mut m, 60);
    assert_eq!(m.cpu.r[20], 1, "SPI STC vector entered once");
    assert_eq!(m.cpu.r[21] & SPIF, 0, "vector entry cleared SPIF");
}

/// Send `bytes` as master and keep each received byte in r2.. .
fn sd_program(spcr: u8, bytes: &[u8]) -> (Prog, u32) {
    let mut p = Prog::default();
    let main = p.at(MAIN);
    main.sts(DDRB, 0b0111); // SS, SCK, MOSI out; MISO in
    main.sts(PORTB, 0b0001); // card deselected
    main.sts(SPCR, spcr);
    main.sts(PORTB, 0b0000); // SS (PB0) low: card selected
    for (i, b) in bytes.iter().enumerate() {
        main.sts(SPDR, *b);
        main.wait_bit(SPSR, 7, true);
        main.lds(2 + i as u16, SPDR);
    }
    main.sts(PORTB, 0b0001);
    let done = main.here(MAIN);
    main.spin();
    (p, done)
}

const SD: &str = "external_devices:\n  - id: sd\n    type: microsd\n    connection: spi\n    config:\n      cs_pin: PB0\n";

/// An SD card in SPI mode on SS/SCK/MOSI/MISO = PB0/PB1/PB2/PB3 answers CMD0
/// (GO_IDLE_STATE) with R1 = 0x01 through SPDR.
#[test]
fn sd_card_on_pb0_answers_cmd0_through_spdr() {
    let cmd0 = [0x40, 0, 0, 0, 0, 0x95, 0xFF, 0xFF];
    let (p, done) = sd_program(SPE | MSTR | 1, &cmd0);
    let mut m = machine(&p, SD);
    run_to_pc(&mut m, done);
    let rx: Vec<u8> = (2..10).map(|r| m.cpu.r[r]).collect();
    let r1 = rx[5..].iter().copied().find(|b| *b != 0xFF);
    assert_eq!(r1, Some(0x01), "R1 idle after CMD0; received {rx:02X?}");
}

/// SPI modes 0..3 (CPOL, CPHA = SPCR bits 3, 2; Table 21-3) change only the
/// clock polarity and sampling edge. The card models are byte-level, so every
/// mode moves the same bytes, and SPCR keeps the mode bits.
#[test]
fn every_spi_mode_exchanges_the_same_bytes() {
    const CPOL: u8 = 1 << 3;
    const CPHA: u8 = 1 << 2;
    for mode in [0, CPHA, CPOL, CPOL | CPHA] {
        let cmd0 = [0x40, 0, 0, 0, 0, 0x95, 0xFF, 0xFF];
        let (mut p, done) = sd_program(SPE | MSTR | mode | 1, &cmd0);
        p.at(done).extend([0x9000 | (20 << 4), SPCR, rjmp(-1)]);
        let mut m = machine(&p, SD);
        run_to_pc(&mut m, done + 4);
        let rx: Vec<u8> = (2..10).map(|r| m.cpu.r[r]).collect();
        let r1 = rx[5..].iter().copied().find(|b| *b != 0xFF);
        assert_eq!(r1, Some(0x01), "mode bits {mode:#04x}: received {rx:02X?}");
        assert_eq!(m.cpu.r[20] & (CPOL | CPHA), mode);
    }
}

/// DORD = 1 shifts the LSB first: the card sees each SPDR byte bit-reversed,
/// and its MSB-first answer arrives bit-reversed in SPDR.
#[test]
fn dord_lsb_first_reverses_the_bit_order_on_the_wire() {
    let cmd0: Vec<u8> = [0x40u8, 0, 0, 0, 0, 0x95, 0xFF, 0xFF]
        .iter()
        .map(|b| b.reverse_bits())
        .collect();
    let (p, done) = sd_program(SPE | MSTR | DORD | 1, &cmd0);
    let mut m = machine(&p, SD);
    run_to_pc(&mut m, done);
    let rx: Vec<u8> = (2..10).map(|r| m.cpu.r[r]).collect();
    let r1 = rx[5..].iter().copied().find(|b| *b != 0xFF);
    assert_eq!(r1, Some(0x80), "R1 0x01 seen LSB first; received {rx:02X?}");
}

/// "If SS is configured as an input and is driven low while MSTR is set, MSTR
/// will be cleared, and SPIF in SPSR will become set. If the SPI interrupt is
/// enabled, and the I-bit in SREG is set, the interrupt routine will be
/// executed."
#[test]
fn ss_input_driven_low_in_master_mode_drops_to_slave_and_sets_spif() {
    let mut p = Prog::default();
    p.count(VEC_SPI_STC, 20);
    let main = p.at(MAIN);
    main.sts(DDRB, 0b0110); // SS (PB0) is an input
    main.sts(SPCR, SPIE | SPE | MSTR);
    main.push(SEI);
    let lp = main.here(MAIN);
    main.lds(21, SPCR);
    main.push(rjmp(-3));
    let _ = lp;
    let mut m = machine(&p, "");
    pad(&mut m, "portb", 0, true);
    steps(&mut m, 30);
    assert_ne!(m.cpu.r[21] & MSTR, 0);
    assert_eq!(m.cpu.r[20], 0);
    pad(&mut m, "portb", 0, false);
    steps(&mut m, 30);
    assert_eq!(m.cpu.r[21] & MSTR, 0, "MSTR cleared by the mode fault");
    assert_eq!(m.cpu.r[20], 1, "SPI vector entered");
}

/// PINB still reads the SPI pads as GPIO: SS released high by firmware is
/// visible on the port (the SPI block does not hide the port).
#[test]
fn spi_pads_stay_ordinary_port_b_pins() {
    let mut p = Prog::default();
    let main = p.at(MAIN);
    main.sts(DDRB, 0b0111);
    main.sts(PORTB, 0b0101);
    main.sts(SPCR, SPE | MSTR);
    main.lds(20, PINB);
    let done = main.here(MAIN);
    main.spin();
    let mut m = machine(&p, "");
    run_to_pc(&mut m, done);
    assert_eq!(m.cpu.r[20] & 0b0111, 0b0101);
}
