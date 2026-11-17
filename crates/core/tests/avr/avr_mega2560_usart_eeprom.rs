// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ATmega2560 USART0..3 and data EEPROM, driven by hand-assembled firmware on
//! the bus built from `configs/chips/atmega2560.yaml`.
//!
//! Every expectation comes from Microchip DS2549 (ATmega640/1280/1281/2560/
//! 2561): the USART chapter (frame formats, baud equations of Table 22-1,
//! the transmit buffer and shift register, the two-level receive FIFO and its
//! error flags, the UCSRnA/B/C bit layouts), the register summary (USART1 at
//! 0xC8, USART2 at 0xD0, USART3 at 0x130), Table 14-1 (vector slots) and the
//! EEPROM section (EECR, EEPM modes and their programming times).
//!
//! The host side is the one every other UART uses: bytes go in through
//! `deliver_uart_rx_named` and come out through `attach_uart_tx_sink_named`.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::{Avr, AvrModel};
use labwired_core::Machine;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const MAIN: usize = 0x80;
const ISR: usize = 0x100;

// UCSRnA / UCSRnB bits (DS2549 USART register description).
const RXC: u8 = 1 << 7;
const TXC: u8 = 1 << 6;
const UDRE: u8 = 1 << 5;
const FE: u8 = 1 << 4;
const DOR: u8 = 1 << 3;
const UPE: u8 = 1 << 2;
const U2X: u8 = 1 << 1;
const RXCIE: u8 = 1 << 7;
const TXCIE: u8 = 1 << 6;
const UDRIE: u8 = 1 << 5;
const RXEN: u8 = 1 << 4;
const TXEN: u8 = 1 << 3;
const UCSZ2: u8 = 1 << 2;
const RXB8: u8 = 1 << 1;

/// (name, UCSRnA data address, RX/UDRE/TX vector slots of Table 14-1).
const USARTS: [(&str, u16, [usize; 3]); 4] = [
    ("usart0", 0xC0, [26, 27, 28]),
    ("usart1", 0xC8, [37, 38, 39]),
    ("usart2", 0xD0, [52, 53, 54]),
    ("usart3", 0x130, [55, 56, 57]),
];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn ldi(d: u16, k: u8) -> u16 {
    let k = u16::from(k);
    0xE000 | ((k & 0xF0) << 4) | ((d - 16) << 4) | (k & 0x0F)
}
fn sts(addr: u16, r: u16) -> [u16; 2] {
    [0x9200 | (r << 4), addr]
}
fn lds(r: u16, addr: u16) -> [u16; 2] {
    [0x9000 | (r << 4), addr]
}
fn jmp(k: usize) -> [u16; 2] {
    [0x940C, k as u16]
}
fn out(a: u16, r: u16) -> u16 {
    0xB800 | ((a & 0x30) << 5) | (r << 4) | (a & 0x0F)
}
fn inp(r: u16, a: u16) -> u16 {
    0xB000 | ((a & 0x30) << 5) | (r << 4) | (a & 0x0F)
}
fn sbi(a: u16, b: u16) -> u16 {
    0x9A00 | (a << 3) | b
}
fn cbi(a: u16, b: u16) -> u16 {
    0x9800 | (a << 3) | b
}
fn sbic(a: u16, b: u16) -> u16 {
    0x9900 | (a << 3) | b
}
fn sbrs(r: u16, b: u16) -> u16 {
    0xFE00 | (r << 4) | b
}
fn inc(r: u16) -> u16 {
    0x9403 | (r << 4)
}
fn cpi(d: u16, k: u8) -> u16 {
    let k = u16::from(k);
    0x3000 | ((k & 0xF0) << 4) | ((d - 16) << 4) | (k & 0x0F)
}
const SEI: u16 = 0x9478;
const RETI: u16 = 0x9518;
/// `ADIW r24, 1`.
const ADIW_R24_1: u16 = 0x9601;

/// Firmware under assembly: a word image, written at explicit word addresses.
struct Prog(Vec<u16>);

impl Prog {
    fn new() -> Self {
        let mut p = Prog(vec![0; 0x200]);
        p.put(0, &jmp(MAIN));
        p
    }
    fn put(&mut self, at: usize, words: &[u16]) -> usize {
        self.0[at..at + words.len()].copy_from_slice(words);
        at + words.len()
    }
    /// Point a vector slot (1-based, Table 14-1) at `target`.
    fn vector(&mut self, slot: usize, target: usize) {
        self.put((slot - 1) * 2, &jmp(target));
    }
}

/// Code that appends `words` at `*at` and returns where it was put.
fn emit(p: &mut Prog, at: &mut usize, words: &[u16]) -> usize {
    let start = *at;
    *at = p.put(*at, words);
    start
}

/// `RJMP .-2`: stay here.
fn halt(p: &mut Prog, at: &mut usize) -> usize {
    emit(p, at, &[0xCFFF])
}

/// About 260 000 clocks of `ADIW r24,1 / BRNE`: long enough for several
/// frames at 115200 baud to arrive while the firmware is not looking.
fn wait_long(p: &mut Prog, at: &mut usize) {
    emit(p, at, &[ldi(24, 0), ldi(25, 0), ADIW_R24_1, 0xF7F1]);
}

/// UBRRn = `ubrr`, UCSRnA/B/C as given, for the USART at `base`.
fn setup(p: &mut Prog, at: &mut usize, base: u16, ubrr: u16, a: u8, b: u8, c: u8) {
    emit(p, at, &[ldi(16, (ubrr >> 8) as u8)]);
    emit(p, at, &sts(base + 5, 16));
    emit(p, at, &[ldi(16, ubrr as u8)]);
    emit(p, at, &sts(base + 4, 16));
    emit(p, at, &[ldi(16, a)]);
    emit(p, at, &sts(base, 16));
    emit(p, at, &[ldi(16, c)]);
    emit(p, at, &sts(base + 2, 16));
    emit(p, at, &[ldi(16, b)]);
    emit(p, at, &sts(base + 1, 16));
}

struct Rig {
    machine: Machine<Avr>,
    console: Arc<Mutex<Vec<u8>>>,
}

fn boot(p: &Prog) -> Rig {
    boot_with_eeprom(p, None)
}

fn boot_with_eeprom(p: &Prog, eeprom: Option<&[u8]>) -> Rig {
    let yaml = root().join("configs/systems/arduino-mega-2560.yaml");
    let manifest = SystemManifest::from_file(&yaml).expect("load system yaml");
    let chip_path = yaml.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip");
    let bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    let mut cpu = Avr::for_chip_name(&chip.name);
    assert_eq!(cpu.model, AvrModel::Atmega2560);
    if let Some(saved) = eeprom {
        cpu.load_eeprom(saved);
    }
    cpu.load_words(0, &p.0);
    let console = Arc::new(Mutex::new(Vec::new()));
    cpu.set_serial_sink(console.clone());
    Rig {
        machine: Machine::new(cpu, bus),
        console,
    }
}

impl Rig {
    fn sink(&mut self, name: &str) -> Arc<Mutex<Vec<u8>>> {
        let sink = Arc::new(Mutex::new(Vec::new()));
        assert!(
            self.machine
                .bus
                .attach_uart_tx_sink_named(name, sink.clone(), false),
            "the chip descriptor maps a host window named {name}"
        );
        sink
    }
    fn inject(&mut self, name: &str, bytes: &[u8]) {
        self.machine
            .bus
            .deliver_uart_rx_named(name, bytes)
            .unwrap_or_else(|e| panic!("{e}"));
    }
    /// Step until the PC is at word `word`; returns the cycle count there.
    fn run_to(&mut self, word: usize, max_steps: usize) -> u64 {
        for _ in 0..max_steps {
            if self.machine.cpu.pc as usize == word * 2 {
                return self.machine.cpu.cycles;
            }
            self.machine.step().expect("step");
        }
        panic!(
            "word {word:#x} not reached; pc at word {:#x}",
            self.machine.cpu.pc / 2
        );
    }
    fn run_steps(&mut self, n: usize) {
        for _ in 0..n {
            self.machine.step().expect("step");
        }
    }
    fn r(&self, n: usize) -> u8 {
        self.machine.cpu.r[n]
    }
}

/// Serial1..Serial3 receive through the RX-complete interrupt at their own
/// Table 14-1 slots and transmit through their own UDRn: a Marlin-style
/// interrupt-driven reader echoing G-code. USART0's console stays silent.
#[test]
fn usart1_to_3_echo_host_bytes_from_their_rx_complete_interrupts() {
    for &(name, base, [rx_slot, _, _]) in &USARTS[1..] {
        let mut p = Prog::new();
        let mut at = MAIN;
        // 115200 baud at 16 MHz, normal speed: UBRR = 16e6 / (16 * 115200) - 1 = 8.
        setup(&mut p, &mut at, base, 8, 0, RXCIE | RXEN | TXEN, 0x06);
        emit(&mut p, &mut at, &[SEI]);
        halt(&mut p, &mut at);
        p.vector(rx_slot, ISR);
        let mut isr = ISR;
        emit(&mut p, &mut isr, &lds(17, base + 6));
        emit(&mut p, &mut isr, &sts(base + 6, 17));
        emit(&mut p, &mut isr, &[RETI]);

        let mut rig = boot(&p);
        let tx = rig.sink(name);
        rig.inject(name, b"G28\n");
        for _ in 0..200_000 {
            rig.machine.step().expect("step");
            if tx.lock().unwrap().len() >= 4 {
                break;
            }
        }
        assert_eq!(tx.lock().unwrap().as_slice(), b"G28\n", "{name} echo");
        assert!(
            rig.console.lock().unwrap().is_empty(),
            "{name} bytes are not the USART0 console"
        );
    }
}

/// UDRE is a level: with UDRIE set the interrupt keeps firing while the
/// transmit buffer is empty, and the buffer only empties as the shift register
/// takes the next byte. At 115200 baud 8N1 (10 bits of 16 x 9 clocks) the
/// first byte goes straight into the shift register and the second into the
/// buffer; each later one is accepted one frame after the previous, so the
/// eighth is written six frame times in. The frames leave back to back, so
/// the host has all eight when the eighth stop bit is out, eight frame times
/// after the first write.
#[test]
fn udre_interrupt_feeds_the_transmitter_at_the_line_rate() {
    let (name, base, [_, udre_slot, _]) = USARTS[2];
    let mut p = Prog::new();
    let mut at = MAIN;
    emit(&mut p, &mut at, &[ldi(20, b'a')]);
    setup(&mut p, &mut at, base, 8, 0, UDRIE | TXEN, 0x06);
    emit(&mut p, &mut at, &[SEI]);
    let spin = halt(&mut p, &mut at);
    p.vector(udre_slot, ISR);
    let mut isr = ISR;
    emit(&mut p, &mut isr, &sts(base + 6, 20));
    emit(&mut p, &mut isr, &[inc(20), cpi(20, b'a' + 8)]);
    // BRNE +3 over the UDRIE disable (LDI + two-word STS).
    emit(&mut p, &mut isr, &[0xF419, ldi(16, TXEN)]);
    emit(&mut p, &mut isr, &sts(base + 1, 16));
    emit(&mut p, &mut isr, &[RETI]);

    let mut rig = boot(&p);
    let tx = rig.sink(name);
    let start = rig.run_to(spin, 1_000);
    let mut last_written = None;
    let mut done = None;
    for _ in 0..100_000 {
        rig.machine.step().expect("step");
        if last_written.is_none() && rig.r(20) == b'a' + 8 {
            last_written = Some(rig.machine.cpu.cycles);
        }
        if done.is_none() && tx.lock().unwrap().len() == 8 {
            done = Some(rig.machine.cpu.cycles);
        }
    }
    assert_eq!(tx.lock().unwrap().as_slice(), b"abcdefgh");
    let frame = 10 * 16 * 9;
    let accepted = last_written.expect("eighth write") - start;
    assert!(
        (6 * frame..6 * frame + 200).contains(&accepted),
        "eighth byte accepted after {accepted} clocks, expected about {}",
        6 * frame
    );
    let took = done.expect("eighth byte") - start;
    assert!(
        (8 * frame..8 * frame + 200).contains(&took),
        "eight bytes took {took} clocks, expected about {}",
        8 * frame
    );
}

/// Build: set up USART3, write one byte to UDR3, poll UCSR3A until TXC.
/// Returns the clocks from the write to the TXC poll exit.
fn tx_frame_clocks(ucsra: u8, ucsrb: u8, ucsrc: u8, ubrr: u16) -> u64 {
    let (_, base, _) = USARTS[3];
    let mut p = Prog::new();
    let mut at = MAIN;
    setup(&mut p, &mut at, base, ubrr, ucsra, ucsrb, ucsrc);
    emit(&mut p, &mut at, &[ldi(16, 0x55)]);
    let write = emit(&mut p, &mut at, &sts(base + 6, 16));
    let poll = at;
    emit(&mut p, &mut at, &lds(17, base));
    emit(&mut p, &mut at, &[sbrs(17, 6), 0xCFFC]);
    let done = halt(&mut p, &mut at);
    assert_eq!(poll + 4, done);

    let mut rig = boot(&p);
    let t0 = rig.run_to(write, 1_000);
    let t1 = rig.run_to(done, 100_000);
    assert_ne!(rig.r(17) & TXC, 0);
    assert_ne!(rig.r(17) & UDRE, 0, "transmit buffer empty after TXC");
    t1 - t0
}

/// TXC sets when the last stop bit has left the shift register. One bit is
/// 16 (U2X = 0) or 8 (U2X = 1) clocks per UBRR + 1 (Table 22-1); a frame is
/// start + data bits + parity + stop bits (frame formats section).
#[test]
fn txc_follows_the_frame_length_baud_and_double_speed() {
    // 8N1, U2X = 1, UBRR = 16: 10 bits of 8 * 17 clocks.
    let fast = tx_frame_clocks(U2X, TXEN, 0x06, 16);
    assert!((1360..1360 + 16).contains(&fast), "8N1 U2X: {fast}");
    // Same UBRR without U2X: twice as long.
    let slow = tx_frame_clocks(0, TXEN, 0x06, 16);
    assert!((2720..2720 + 16).contains(&slow), "8N1: {slow}");
    // 9 data bits (UCSZ = 7), even parity (UPM = 2), two stop bits:
    // 1 + 9 + 1 + 2 = 13 bits.
    let long = tx_frame_clocks(U2X, TXEN | UCSZ2, 0x20 | 0x08 | 0x06, 16);
    assert!((13 * 136..13 * 136 + 16).contains(&long), "9E2 U2X: {long}");
}

/// TXC is cleared by hardware when the TX-complete vector is executed, so
/// the interrupt runs once per frame and not forever.
#[test]
fn tx_complete_interrupt_runs_once_and_clears_txc() {
    let (_, base, [_, _, tx_slot]) = USARTS[3];
    let mut p = Prog::new();
    let mut at = MAIN;
    emit(&mut p, &mut at, &[ldi(21, 0)]);
    setup(&mut p, &mut at, base, 16, U2X, TXCIE | TXEN, 0x06);
    emit(&mut p, &mut at, &[SEI, ldi(16, b'x')]);
    emit(&mut p, &mut at, &sts(base + 6, 16));
    halt(&mut p, &mut at);
    p.vector(tx_slot, ISR);
    let mut isr = ISR;
    emit(&mut p, &mut isr, &[inc(21)]);
    emit(&mut p, &mut isr, &lds(22, base));
    emit(&mut p, &mut isr, &[RETI]);

    let mut rig = boot(&p);
    rig.run_steps(20_000);
    assert_eq!(rig.r(21), 1, "TX complete vector taken once");
    assert_eq!(rig.r(22) & TXC, 0, "TXC cleared on vector entry");
}

/// The receive buffer is a two-level FIFO behind the shift register. With
/// both levels full and a third character waiting in the shift register, the
/// start bit of a fourth overruns: the waiting character is lost and DOR is
/// reported with the character that follows the gap. Reading UDR0 moves the
/// FIFO along.
#[test]
fn receive_overrun_loses_a_character_and_flags_dor() {
    let (name, base, _) = USARTS[0];
    let mut p = Prog::new();
    let mut at = MAIN;
    setup(&mut p, &mut at, base, 8, 0, RXEN, 0x06);
    wait_long(&mut p, &mut at);
    for (status, data) in [(2, 3), (4, 5), (6, 7)] {
        emit(&mut p, &mut at, &lds(status, base));
        emit(&mut p, &mut at, &lds(data, base + 6));
    }
    emit(&mut p, &mut at, &lds(8, base));
    let done = halt(&mut p, &mut at);

    let mut rig = boot(&p);
    rig.inject(name, b"ABCD");
    rig.run_to(done, 1_000_000);
    let got = [rig.r(3), rig.r(5), rig.r(7)];
    assert_eq!(&got, b"ABD", "C is lost to the overrun");
    assert_eq!(rig.r(2) & (RXC | DOR), RXC);
    assert_eq!(rig.r(4) & (RXC | DOR), RXC);
    assert_eq!(rig.r(6) & (RXC | DOR), RXC | DOR, "DOR comes with D");
    assert_eq!(rig.r(8) & RXC, 0, "FIFO drained");
}

/// Receive one host byte (the host line is 8N1) with USART1 set to `ucsrb`/
/// `ucsrc`, and return (UCSR1A, UDR1, UCSR1B) as read by the firmware.
fn receive_with_format(ucsrb: u8, ucsrc: u8, byte: u8) -> (u8, u8, u8) {
    let (name, base, _) = USARTS[1];
    let mut p = Prog::new();
    let mut at = MAIN;
    setup(&mut p, &mut at, base, 8, 0, ucsrb, ucsrc);
    wait_long(&mut p, &mut at);
    emit(&mut p, &mut at, &lds(2, base));
    emit(&mut p, &mut at, &lds(4, base + 1));
    emit(&mut p, &mut at, &lds(3, base + 6));
    let done = halt(&mut p, &mut at);
    let mut rig = boot(&p);
    rig.inject(name, &[byte]);
    rig.run_to(done, 1_000_000);
    (rig.r(2), rig.r(3), rig.r(4))
}

/// The receiver samples the line with its own frame format. A host 8N1 frame
/// read as 7 data bits puts host bit 7 where the stop bit is expected (FE when
/// it is 0) and the unused MSB reads 0; read with even parity the host's stop
/// bit is taken as the parity bit (UPE when it disagrees); read as 9 bits the
/// host's stop bit becomes RXB8.
#[test]
fn receiver_frame_format_sets_fe_upe_and_rxb8() {
    // 7N1 (UCSZ = 2).
    let (a, d, _) = receive_with_format(RXEN, 0x04, 0x41);
    assert_eq!((a & (RXC | FE), d), (RXC | FE, 0x41), "7N1 bit 7 = 0");
    let (a, d, _) = receive_with_format(RXEN, 0x04, 0xC1);
    assert_eq!((a & (RXC | FE), d), (RXC, 0x41), "7N1 bit 7 = 1");
    // 8E1: parity bit position reads the host stop bit (1).
    let (a, d, _) = receive_with_format(RXEN, 0x20 | 0x06, 0x41);
    assert_eq!(
        (a & (UPE | FE), d),
        (UPE, 0x41),
        "two ones: even parity bit is 0"
    );
    let (a, _, _) = receive_with_format(RXEN, 0x20 | 0x06, 0x43);
    assert_eq!(a & (UPE | FE), 0, "three ones: even parity bit is 1");
    // 9N1 (UCSZ = 7): the ninth bit is the host's stop bit.
    let (a, d, b) = receive_with_format(RXEN | UCSZ2, 0x06, 0x41);
    assert_eq!((a & (FE | UPE), d, b & RXB8), (0, 0x41, RXB8));
}

// EEPROM: EECR/EEDR/EEARL/EEARH at I/O 0x1F..0x22.
const EECR: u16 = 0x1F;
const EEDR: u16 = 0x20;
const EEARL: u16 = 0x21;
const EEARH: u16 = 0x22;
const EE_READY_SLOT: usize = 31;

/// The avr-libc `eeprom_write_byte` sequence for `addr`/`data`.
fn ee_write(p: &mut Prog, at: &mut usize, addr: u16, data: u8) {
    let wait = *at;
    emit(p, at, &[sbic(EECR, 1), 0xCFFE]);
    assert_eq!(wait + 2, *at);
    emit(p, at, &[ldi(16, (addr >> 8) as u8), out(EEARH, 16)]);
    emit(p, at, &[ldi(16, addr as u8), out(EEARL, 16)]);
    emit(p, at, &[ldi(16, data), out(EEDR, 16)]);
    emit(p, at, &[sbi(EECR, 2), sbi(EECR, 1)]);
}

/// The 4 KB array is addressed by all twelve EEAR bits; EE_READY (EERIE with
/// no write in progress) vectors to slot 31 once the 3.4 ms erase-and-write
/// (EEPM = 00) has finished, and EERE reads the cell back.
#[test]
fn eeprom_top_cell_write_and_ee_ready_interrupt() {
    let mut p = Prog::new();
    let mut at = MAIN;
    emit(&mut p, &mut at, &[ldi(20, 0)]);
    ee_write(&mut p, &mut at, 0x0FFF, 0x5A);
    emit(&mut p, &mut at, &[sbi(EECR, 3), SEI]);
    halt(&mut p, &mut at);
    p.vector(EE_READY_SLOT, ISR);
    let mut isr = ISR;
    emit(
        &mut p,
        &mut isr,
        &[cbi(EECR, 3), sbi(EECR, 0), inp(20, EEDR), RETI],
    );

    let mut rig = boot(&p);
    let mut entered = None;
    for _ in 0..200_000 {
        rig.machine.step().expect("step");
        if entered.is_none() && rig.machine.cpu.pc as usize == ISR * 2 {
            entered = Some(rig.machine.cpu.cycles);
        }
    }
    assert_eq!(rig.machine.cpu.eeprom().len(), 4096);
    assert_eq!(rig.machine.cpu.eeprom()[0x0FFF], 0x5A);
    assert_eq!(rig.machine.cpu.eeprom()[0x0000], 0xFF, "no wrap to cell 0");
    assert_eq!(rig.r(20), 0x5A, "EERE in the EE_READY handler");
    let t = entered.expect("EE_READY taken");
    assert!(t >= 54_400, "EE_READY after 3.4 ms at 16 MHz, got {t}");
    assert!(t < 54_400 + 200, "EE_READY right after the write, got {t}");
}

/// Settings written in one run (Marlin M500) are there in the next (M501)
/// when the host carries the array over, and survive a reset in the same run.
#[test]
fn eeprom_contents_carry_into_the_next_run_and_survive_reset() {
    let mut p = Prog::new();
    let mut at = MAIN;
    ee_write(&mut p, &mut at, 0x0123, 0x77);
    let done = halt(&mut p, &mut at);
    let mut first = boot(&p);
    first.run_to(done, 1_000);
    first.run_steps(100_000);
    let saved = first.machine.cpu.eeprom().to_vec();
    assert_eq!(saved[0x0123], 0x77);

    let mut p = Prog::new();
    let mut at = MAIN;
    emit(&mut p, &mut at, &[ldi(16, 0x01), out(EEARH, 16)]);
    emit(&mut p, &mut at, &[ldi(16, 0x23), out(EEARL, 16)]);
    emit(&mut p, &mut at, &[sbi(EECR, 0), inp(20, EEDR)]);
    let done = halt(&mut p, &mut at);
    let mut second = boot_with_eeprom(&p, Some(&saved));
    second.run_to(done, 1_000);
    assert_eq!(second.r(20), 0x77);

    second.machine.reset().expect("reset");
    assert_eq!(second.machine.cpu.eeprom()[0x0123], 0x77);
}
