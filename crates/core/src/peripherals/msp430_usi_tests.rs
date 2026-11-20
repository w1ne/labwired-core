// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use super::*;
use std::sync::Mutex;

const COUNTER: u32 = 40;
const START: u32 = 41;
const LINE: u32 = 11;
// USICKCTL: SMCLK.
const SMCLK: u8 = 2 << 2;
const SWCLK_SOURCE: u8 = 4 << 2;
const USICKPL: u8 = 0x02;

fn usi() -> Msp430Usi {
    let mut usi = Msp430Usi::new(
        Some(UsiRequest {
            counter_source: COUNTER,
            start_source: START,
            line: LINE,
        }),
        UsiConfig {
            aclk_hz: 32_768,
            cpu_hz: 1_000_000,
        },
    );
    usi.attach_msp430_irq(&Msp430Irq::new());
    usi
}

fn w(usi: &mut Msp430Usi, offset: u64, value: u8) {
    usi.write(offset, value).unwrap();
}

fn r(usi: &Msp430Usi, offset: u64) -> u8 {
    usi.read(offset).unwrap()
}

/// Run until `USIIFG` sets; the CPU cycles it took.
fn run(usi: &mut Msp430Usi) -> u64 {
    let mut n = 0;
    while r(usi, CTL1) & USIIFG == 0 {
        usi.tick_elapsed(1);
        n += 1;
        assert!(n < 100_000, "the transfer never ends");
    }
    n
}

/// An SPI device that records what it gets and answers from a script.
struct Spi {
    seen: Arc<Mutex<Vec<u8>>>,
    answers: Vec<u8>,
    cs: &'static str,
}

impl SpiDevice for Spi {
    fn transfer(&mut self, mosi: u8) -> u8 {
        let mut seen = self.seen.lock().unwrap();
        let answer = self.answers.get(seen.len()).copied().unwrap_or(0);
        seen.push(mosi);
        answer
    }

    fn cs_pin(&self) -> &str {
        self.cs
    }
}

fn spi_device(usi: &mut Msp430Usi, answers: &[u8]) -> Arc<Mutex<Vec<u8>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    usi.spi_attached_devices_mut().unwrap().push(Box::new(Spi {
        seen: seen.clone(),
        answers: answers.to_vec(),
        cs: "",
    }));
    seen
}

/// SPI master on SMCLK, SCLK/SDO/SDI enabled, released from reset.
fn spi_master(usi: &mut Msp430Usi, ctl0: u8, ckctl: u8) {
    let ctl0 = USIPE7 | 0x40 | USIPE5 | USIMST | USIOE | ctl0;
    w(usi, CTL0, ctl0 | USISWRST);
    w(usi, CKCTL, ckctl);
    w(usi, CTL0, ctl0);
}

#[test]
fn reset_holds_usiifg_and_keeps_the_shift_register() {
    let mut usi = usi();
    assert_eq!(r(&usi, CTL0), USISWRST);
    assert_eq!(r(&usi, CTL1), USIIFG);
    w(&mut usi, SRL, 0x5A);
    // In reset a counter write neither clears USIIFG nor clocks.
    w(&mut usi, CNT, 8);
    assert_eq!(r(&usi, CTL1), USIIFG);
    w(&mut usi, CTL1, 0);
    assert_eq!(r(&usi, CTL1), USIIFG, "USISWRST holds USIIFG");
    usi.on_system_reset();
    assert_eq!(r(&usi, SRL), 0x5A, "USISR is unchanged by a reset");
    assert_eq!(r(&usi, CNT), 0);
}

#[test]
fn spi_master_exchanges_a_byte_in_eight_divided_clocks() {
    let mut usi = usi();
    let seen = spi_device(&mut usi, &[0xC3]);
    // SMCLK / 4.
    spi_master(&mut usi, 0, 2 << 5 | SMCLK);
    w(&mut usi, SRL, 0xA5);
    w(&mut usi, CNT, 8);
    assert_eq!(r(&usi, CTL1) & USIIFG, 0, "a count write clears USIIFG");
    usi.tick_elapsed(4 * 3);
    assert_eq!(r(&usi, CNT), 5);
    assert_eq!(run(&mut usi), 4 * 5);
    assert_eq!(*seen.lock().unwrap(), vec![0xA5]);
    assert_eq!(r(&usi, SRL), 0xC3);
    assert_eq!(r(&usi, CNT), 0);
    // Nothing moves until the next count write.
    usi.tick_elapsed(1000);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn lsb_first_and_sixteen_bit_frames_keep_wire_order() {
    let mut usi = usi();
    let seen = spi_device(&mut usi, &[0x80, 0x12, 0x34]);
    spi_master(&mut usi, USILSB, SMCLK);
    w(&mut usi, SRL, 0x01);
    w(&mut usi, CNT, 8);
    run(&mut usi);
    // LSB first: 0x01 leaves as 1000_0000; the answer 0x80 arrives the same way.
    assert_eq!(seen.lock().unwrap()[0], 0x80);
    assert_eq!(r(&usi, SRL), 0x01);
    spi_master(&mut usi, 0, SMCLK);
    usi.write_u16(SRL, 0xBEEF).unwrap();
    w(&mut usi, CNT, USI16B | 16);
    assert_eq!(run(&mut usi), 16);
    assert_eq!(seen.lock().unwrap()[1..], [0xBE, 0xEF]);
    assert_eq!(usi.read_u16(SRL).unwrap(), 0x1234);
}

#[test]
fn short_transfers_and_no_device_sample_zero() {
    let mut usi = usi();
    let seen = spi_device(&mut usi, &[0xFF]);
    spi_master(&mut usi, 0, SMCLK);
    w(&mut usi, SRL, 0xFF);
    w(&mut usi, CNT, 7);
    assert_eq!(run(&mut usi), 7);
    assert!(seen.lock().unwrap().is_empty(), "a 7-bit frame is no byte");
    assert_eq!(r(&usi, SRL), 0x80);
    let mut alone = self::usi();
    spi_master(&mut alone, 0, SMCLK);
    w(&mut alone, SRL, 0xFF);
    w(&mut alone, CNT, 8);
    run(&mut alone);
    assert_eq!(r(&alone, SRL), 0);
}

#[test]
fn count_writes_and_usiifgcc_control_the_flag() {
    let mut usi = usi();
    spi_master(&mut usi, 0, SMCLK);
    w(&mut usi, CNT, 0);
    assert_ne!(r(&usi, CTL1) & USIIFG, 0, "a count of 0 sets USIIFG");
    w(&mut usi, CNT, USIIFGCC | 4);
    assert_ne!(r(&usi, CTL1) & USIIFG, 0, "USIIFGCC keeps USIIFG");
    usi.tick_elapsed(100);
    assert_eq!(r(&usi, CNT) & USICNTX, 4, "USIIFG stops the clock");
    // Clearing USIIFG by software starts the clock.
    w(&mut usi, CTL1, 0);
    assert_eq!(run(&mut usi), 4);
}

#[test]
fn software_clock_shifts_on_each_rising_write() {
    let mut usi = usi();
    spi_master(&mut usi, 0, SWCLK_SOURCE);
    w(&mut usi, SRL, 0x80);
    w(&mut usi, CNT, 2);
    usi.tick_elapsed(100);
    assert_eq!(r(&usi, CNT), 2, "no free-running clock");
    w(&mut usi, CKCTL, SWCLK_SOURCE | USISWCLK);
    w(&mut usi, CKCTL, SWCLK_SOURCE | USISWCLK);
    assert_eq!(r(&usi, CNT), 1, "only a rising edge shifts");
    w(&mut usi, CKCTL, SWCLK_SOURCE);
    w(&mut usi, CKCTL, SWCLK_SOURCE | USISWCLK);
    assert_eq!(r(&usi, CNT), 0);
    assert_ne!(r(&usi, CTL1) & USIIFG, 0);
    assert_eq!(r(&usi, SRL), 0);
}

#[test]
fn aclk_bits_count_against_the_cpu_clock_and_slave_mode_waits() {
    let mut usi = usi();
    spi_master(&mut usi, 0, 1 << 2);
    w(&mut usi, CNT, 1);
    // One ACLK period is 30.5 CPU cycles at 1 MHz.
    assert_eq!(run(&mut usi), 31);
    let mut slave = self::usi();
    w(&mut slave, CTL0, USIPE5 | USISWRST);
    w(&mut slave, CKCTL, SMCLK);
    w(&mut slave, CTL0, USIPE5);
    w(&mut slave, CNT, 8);
    slave.tick_elapsed(1000);
    assert_eq!(r(&slave, CNT), 8, "a slave waits for an external clock");
}

#[test]
fn counter_and_start_flags_share_the_usi_line() {
    let irq = Msp430Irq::new();
    let mut usi = Msp430Usi::new(
        Some(UsiRequest {
            counter_source: COUNTER,
            start_source: START,
            line: LINE,
        }),
        UsiConfig {
            aclk_hz: 0,
            cpu_hz: 0,
        },
    );
    usi.attach_msp430_irq(&irq);
    spi_master(&mut usi, 0, SMCLK);
    w(&mut usi, CTL1, USIIE);
    w(&mut usi, CNT, 1);
    assert_eq!(irq.pending(), 0);
    run(&mut usi);
    assert_eq!(irq.pending(), 1 << LINE);
    irq.accept(LINE);
    assert_eq!(irq.pending(), 1 << LINE, "acceptance leaves USIIFG set");
    w(&mut usi, CNT, 1);
    assert_eq!(irq.pending(), 0);
}

// ── I2C ──────────────────────────────────────────────────────────────────

#[derive(Default)]
struct Log {
    events: Vec<String>,
}

struct Slave {
    address: u8,
    log: Arc<Mutex<Log>>,
    next: u8,
}

impl I2cDevice for Slave {
    fn address(&self) -> u8 {
        self.address
    }

    fn read(&mut self) -> u8 {
        self.next = self.next.wrapping_add(1);
        self.log
            .lock()
            .unwrap()
            .events
            .push(format!("read {:02X}", self.next));
        self.next
    }

    fn write(&mut self, data: u8) {
        self.log
            .lock()
            .unwrap()
            .events
            .push(format!("write {data:02X}"));
    }

    fn start(&mut self) {
        self.log.lock().unwrap().events.push("start".into());
    }

    fn stop(&mut self) {
        self.log.lock().unwrap().events.push("stop".into());
    }
}

/// An I2C master as the TI USI I2C examples set it up: SCL/SDA enabled,
/// SMCLK / 2, SCL idle high.
fn i2c_master() -> (Msp430Usi, Arc<Mutex<Log>>) {
    let mut usi = usi();
    let log = Arc::new(Mutex::new(Log::default()));
    usi.attach_slave(Box::new(Slave {
        address: 0x48,
        log: log.clone(),
        next: 0x10,
    }));
    w(&mut usi, CTL0, USIPE7 | 0x40 | USIMST | USISWRST);
    w(&mut usi, CTL1, USII2C);
    w(&mut usi, CKCTL, 1 << 5 | SMCLK | USICKPL);
    w(&mut usi, CTL0, USIPE7 | 0x40 | USIMST);
    (usi, log)
}

fn shift(usi: &mut Msp430Usi, bits: u8) {
    w(usi, CNT, bits);
    run(usi);
}

fn i2c_start(usi: &mut Msp430Usi) {
    w(usi, SRL, 0x00);
    w(usi, CTL0, r(usi, CTL0) | USIGE | USIOE);
    w(usi, CTL0, r(usi, CTL0) & !USIGE);
}

fn i2c_stop(usi: &mut Msp430Usi) {
    w(usi, CTL0, r(usi, CTL0) | USIOE);
    w(usi, SRL, 0x00);
    shift(usi, 1);
    w(usi, SRL, 0xFF);
    w(usi, CTL0, r(usi, CTL0) | USIGE);
    w(usi, CTL0, r(usi, CTL0) & !(USIGE | USIOE));
}

/// Send `byte` and return the acknowledge bit (0 = ACK).
fn i2c_send(usi: &mut Msp430Usi, byte: u8) -> u8 {
    w(usi, CTL0, r(usi, CTL0) | USIOE);
    w(usi, SRL, byte);
    shift(usi, 8);
    w(usi, CTL0, r(usi, CTL0) & !USIOE);
    shift(usi, 1);
    r(usi, SRL) & 1
}

/// Receive a byte and answer ACK (`ack`) or NACK.
fn i2c_receive(usi: &mut Msp430Usi, ack: bool) -> u8 {
    w(usi, CTL0, r(usi, CTL0) & !USIOE);
    shift(usi, 8);
    let byte = r(usi, SRL);
    w(usi, CTL0, r(usi, CTL0) | USIOE);
    w(usi, SRL, if ack { 0x00 } else { 0xFF });
    shift(usi, 1);
    byte
}

#[test]
fn i2c_write_transfer_reaches_the_addressed_device() {
    let (mut usi, log) = i2c_master();
    w(&mut usi, CTL1, USII2C);
    i2c_start(&mut usi);
    assert_ne!(r(&usi, CTL1) & USISTTIFG, 0, "the USI sees its own START");
    assert_eq!(i2c_send(&mut usi, 0x48 << 1), 0, "address ACK");
    assert_eq!(
        r(&usi, CTL1) & USISTP,
        0,
        "releasing SDA for the ACK after a 0 is no STOP"
    );
    assert_eq!(i2c_send(&mut usi, 0x01), 0);
    assert_eq!(i2c_send(&mut usi, 0x60), 0);
    i2c_stop(&mut usi);
    assert_ne!(r(&usi, CTL1) & USISTP, 0);
    assert_eq!(r(&usi, CTL1) & USIAL, 0);
    assert_eq!(
        log.lock().unwrap().events,
        ["start", "write 01", "write 60", "stop"]
    );
    // A count write clears USISTP.
    w(&mut usi, CNT, 1);
    assert_eq!(r(&usi, CTL1) & USISTP, 0);
}

#[test]
fn i2c_read_transfer_ends_on_the_masters_nack() {
    let (mut usi, log) = i2c_master();
    i2c_start(&mut usi);
    assert_eq!(i2c_send(&mut usi, 0x48 << 1 | 1), 0);
    assert_eq!(i2c_receive(&mut usi, true), 0x11);
    assert_eq!(i2c_receive(&mut usi, false), 0x12);
    i2c_stop(&mut usi);
    assert_eq!(
        log.lock().unwrap().events,
        ["start", "read 11", "read 12", "stop"]
    );
}

#[test]
fn i2c_unknown_address_is_not_acknowledged() {
    let (mut usi, log) = i2c_master();
    i2c_start(&mut usi);
    assert_eq!(i2c_send(&mut usi, 0x50 << 1), 1, "NACK");
    assert_eq!(i2c_send(&mut usi, 0x01), 1, "nobody listens");
    i2c_stop(&mut usi);
    assert!(log.lock().unwrap().events.is_empty());
}

#[test]
fn a_master_driving_one_over_an_ack_loses_arbitration() {
    let (mut usi, _) = i2c_master();
    i2c_start(&mut usi);
    i2c_send(&mut usi, 0x48 << 1);
    // Keep the output on through the next acknowledge bit with a 1 latched.
    let ctl0 = r(&usi, CTL0);
    w(&mut usi, CTL0, ctl0 | USIOE);
    w(&mut usi, SRL, 0x00);
    shift(&mut usi, 8);
    w(&mut usi, SRL, 0xFF);
    shift(&mut usi, 1);
    assert_ne!(r(&usi, CTL1) & USIAL, 0);
    assert_eq!(r(&usi, CTL0) & USIOE, 0, "USIAL clears USIOE");
    assert_eq!(r(&usi, SRL) & 1, 0, "the device's ACK won");
}

#[test]
fn a_start_through_the_transparent_latch_needs_no_latch_change() {
    let (mut usi, log) = i2c_master();
    // Leave a 0 in the output latch, then release SDA.
    let ctl0 = r(&usi, CTL0);
    w(&mut usi, CTL0, ctl0 | USIOE);
    w(&mut usi, SRL, 0x00);
    shift(&mut usi, 1);
    let ctl0 = r(&usi, CTL0);
    w(&mut usi, CTL0, ctl0 & !USIOE);
    w(&mut usi, CTL1, USII2C);
    i2c_start(&mut usi);
    assert_ne!(r(&usi, CTL1) & USISTTIFG, 0);
    assert_eq!(i2c_send(&mut usi, 0x48 << 1 | 1), 0);
    assert_eq!(i2c_receive(&mut usi, false), 0x11);
    i2c_stop(&mut usi);
    assert_eq!(log.lock().unwrap().events, ["start", "read 11", "stop"]);
}
