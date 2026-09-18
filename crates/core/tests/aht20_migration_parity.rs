// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **`aht20.yaml` against the `components/aht20.rs` it replaces.**
//!
//! The model is DELETED — not kept as an oracle in the tree, because two of the
//! things it did are the things this port deliberately changes: its BUSY bit
//! was a COUNT of status reads (its own header said "we don't actually model
//! elapsed time") and its measurement was a CONSTANT. An oracle in
//! `components/` would be asserting both. It is reproduced verbatim below and
//! every test drives the SAME script object through both.
//!
//! ## What is actually under test
//!
//! 1. **`crc8.covers: { bytes: 6 }`** — one checksum over the first six ANSWER
//!    bytes, computed over what the twin actually answered. Checked against an
//!    INDEPENDENT CRC-8 implementation (the oracle's), at every sampled
//!    temperature/humidity pair, not against a literal.
//! 2. **`response[].fields` straddling byte boundaries** — the packed 40-bit
//!    word, swept across the declared range of both channels, with the shared
//!    nibble of byte 3 asserted explicitly.
//! 3. **BUSY is elapsed time, not a read count.** Both halves: it clears at
//!    exactly 80 000 µs, and it does NOT clear for any number of reads that do
//!    not spend the time. The second is the negative control that separates
//!    this from the thunk.
//! 4. **Byte parity at the model's one operating point** (25 °C / 50 %RH),
//!    which is the only point the constant model could express.

use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;
use labwired_core::peripherals::i2c::I2cDevice;
use labwired_core::sim_input::SimInput;

// ─── the oracle: the deleted model, verbatim ───────────────────────────────

// ⚠️ `clippy::all` is allowed on the ORACLE ONLY, and deliberately: this is a
// verbatim copy of the deleted model. "Simplifying" a `match` here would make
// it something other than what was deleted, which is the one thing an oracle
// must not be.
#[allow(dead_code, clippy::all)]
mod oracle {
    use labwired_core::peripherals::i2c::I2cDevice;

    pub const AHT20_ADDR: u8 = 0x38;
    pub const BUSY_TICKS: u8 = 2;
    pub const STATUS_BUSY: u8 = 0x80;
    pub const STATUS_CAL: u8 = 0x08;
    pub const STATUS_READY: u8 = STATUS_CAL;

    #[derive(Debug)]
    pub struct Aht20 {
        pub payload: [u8; 7],
        busy_remaining: u8,
        read_idx: usize,
        write_idx: u8,
        last_cmd: u8,
    }

    impl Aht20 {
        pub fn new() -> Self {
            // Fixed 25.0 °C, 50 %RH encoded per datasheet:
            //   raw_h (20-bit) = 50 * 2^20 / 100 = 0x80000
            //   raw_t (20-bit) = (25 + 50) * 2^20 / 200 = 0x60000
            let mut payload = [STATUS_READY, 0x80, 0x00, 0x06, 0x00, 0x00, 0x00];
            payload[6] = crc8(&payload[..6]);
            Self {
                payload,
                busy_remaining: 0,
                read_idx: 0,
                write_idx: 0,
                last_cmd: 0,
            }
        }
    }

    impl I2cDevice for Aht20 {
        fn address(&self) -> u8 {
            AHT20_ADDR
        }

        fn start(&mut self) {
            self.read_idx = 0;
            self.write_idx = 0;
        }

        fn write(&mut self, data: u8) {
            match self.write_idx {
                0 => {
                    self.last_cmd = data;
                    match data {
                        0xAC => self.busy_remaining = BUSY_TICKS,
                        0xBA => self.busy_remaining = 0,
                        _ => {}
                    }
                }
                _ => {}
            }
            self.write_idx = self.write_idx.saturating_add(1);
        }

        fn read(&mut self) -> u8 {
            if self.read_idx == 0 {
                let byte = if self.busy_remaining > 0 {
                    self.busy_remaining = self.busy_remaining.saturating_sub(1);
                    STATUS_BUSY | STATUS_CAL
                } else {
                    self.payload[0]
                };
                self.read_idx += 1;
                byte
            } else if self.read_idx < self.payload.len() {
                let byte = self.payload[self.read_idx];
                self.read_idx += 1;
                byte
            } else {
                0xFF
            }
        }
    }

    /// CRC8 poly 0x31, init 0xFF, no final XOR. Per AHT20 datasheet.
    /// ⚠️ COPIED, not imported: importing the engine's helper would make every
    /// checksum assertion compare the engine against itself.
    pub fn crc8(data: &[u8]) -> u8 {
        let mut crc: u8 = 0xFF;
        for &byte in data {
            crc ^= byte;
            for _ in 0..8 {
                if (crc & 0x80) != 0 {
                    crc = (crc << 1) ^ 0x31;
                } else {
                    crc <<= 1;
                }
            }
        }
        crc
    }
}

// ─── the rig ───────────────────────────────────────────────────────────────

fn device() -> GenericI2cDevice {
    GenericI2cDevice::from_yaml(
        labwired_config::embedded_device_yaml("aht20").expect("aht20 descriptor is embedded"),
        0x38,
    )
    .expect("aht20.yaml is a valid declarative i2c descriptor")
}

/// ONE script object: the stimulus, the command bytes, the microseconds to
/// spend, and how many bytes to clock. Both implementations are driven through
/// it; the oracle simply has nowhere to spend the time, which IS the difference.
struct Script {
    stimulus: Vec<(&'static str, f64)>,
    command: Vec<u8>,
    elapsed_us: u64,
    reads: usize,
}

fn run_descriptor(script: &Script) -> Vec<u8> {
    let mut d = device();
    for (k, v) in &script.stimulus {
        d.set_input(k, *v).expect("declared channel");
    }
    d.start();
    for b in &script.command {
        d.write(*b);
    }
    d.stop();
    d.advance_time_us(script.elapsed_us);
    d.start();
    (0..script.reads).map(|_| d.read()).collect()
}

fn run_oracle(script: &Script) -> Vec<u8> {
    let mut d = oracle::Aht20::new();
    d.start();
    for b in &script.command {
        d.write(*b);
    }
    d.start();
    (0..script.reads).map(|_| d.read()).collect()
}

const TRIGGER: [u8; 3] = [0xAC, 0x33, 0x00];

fn full_read(temperature: f64, humidity: f64) -> Vec<u8> {
    run_descriptor(&Script {
        stimulus: vec![("temperature", temperature), ("humidity", humidity)],
        command: TRIGGER.to_vec(),
        elapsed_us: 80_000,
        reads: 7,
    })
}

// ─── byte parity at the model's one operating point ────────────────────────

/// 25 °C / 50 %RH is the ONLY point the deleted model could express — its
/// payload was encoded once at construction. There, the seven bytes must match
/// exactly, checksum included.
#[test]
fn the_seven_bytes_at_25c_50rh_match_the_deleted_model() {
    let script = Script {
        stimulus: vec![("temperature", 25.0), ("humidity", 50.0)],
        command: TRIGGER.to_vec(),
        elapsed_us: 80_000,
        reads: 7,
    };
    // The oracle's BUSY count has to be drained first — it clears after two
    // status reads, which is the thunk. Two throwaway polls, then the read.
    let mut d = oracle::Aht20::new();
    d.start();
    for b in TRIGGER {
        d.write(b);
    }
    for _ in 0..oracle::BUSY_TICKS {
        d.start();
        let _ = d.read();
    }
    d.start();
    let want: Vec<u8> = (0..7).map(|_| d.read()).collect();

    assert_eq!(
        run_descriptor(&script),
        want,
        "the packed measurement and its checksum must be byte-identical"
    );
    assert_eq!(
        want,
        vec![0x08, 0x80, 0x00, 0x06, 0x00, 0x00, oracle::crc8(&want[..6])],
        "oracle control: the datasheet encoding of 25 °C / 50 %RH"
    );
}

// ─── `response[].fields`: the packed 40-bit word ───────────────────────────

/// The whole point of `fields:`. Swept across both declared channel ranges: the
/// twenty-bit humidity and the twenty-bit temperature decode back to what was
/// driven, and byte 3 carries BOTH — the low nibble of the humidity in its high
/// nibble and the high nibble of the temperature in its low nibble.
#[test]
fn the_packed_word_carries_both_measurements_across_the_shared_nibble() {
    for t_milli in (-40_000..=85_000).step_by(517) {
        for h_milli in (0..=100_000).step_by(3_331) {
            let t = f64::from(t_milli) / 1000.0;
            let h = f64::from(h_milli) / 1000.0;
            let b = full_read(t, h);

            let raw_h = (u32::from(b[1]) << 12) | (u32::from(b[2]) << 4) | (u32::from(b[3]) >> 4);
            let raw_t = ((u32::from(b[3]) & 0x0F) << 16) | (u32::from(b[4]) << 8) | u32::from(b[5]);

            assert_eq!(
                raw_h,
                (h * 10485.76).round() as u32,
                "humidity {h} %RH packed into bits[39:20]"
            );
            assert_eq!(
                raw_t,
                (t * 5242.88 + 262144.0).round() as u32,
                "temperature {t} °C packed into bits[19:0]"
            );
        }
    }
}

/// The NEGATIVE control for the sweep above: byte 3 must genuinely be SHARED.
/// A build that gave each measurement its own bytes would pass a decode test
/// that only ever drove round numbers, so this drives a pair chosen to put a
/// non-zero nibble on each side of byte 3 and asserts the byte itself.
#[test]
fn byte_three_carries_a_nibble_of_each_measurement() {
    // raw_h = 0xABCDE ⇒ h = 0xABCDE / 10485.76; raw_t = 0x12345.
    let h = f64::from(0xAB_CDEu32) / 10485.76;
    let t = (f64::from(0x1_2345u32) - 262144.0) / 5242.88;
    let b = full_read(t, h);
    assert_eq!(b[1], 0xAB);
    assert_eq!(b[2], 0xCD);
    assert_eq!(
        b[3], 0xE1,
        "byte 3 is humidity[3:0] in the high nibble and temperature[19:16] in the low"
    );
    assert_eq!(b[4], 0x23);
    assert_eq!(b[5], 0x45);
}

// ─── `crc8.covers: { bytes: 6 }` ───────────────────────────────────────────

/// The checksum covers the SIX bytes the twin answered, whatever they are — not
/// a literal. Checked against the oracle's own CRC-8, which is a copy rather
/// than an import, so this compares two implementations.
#[test]
fn the_checksum_covers_the_six_answer_bytes_at_every_sampled_reading() {
    for (t, h) in [
        (25.0, 50.0),
        (-40.0, 0.0),
        (85.0, 100.0),
        (0.0, 12.5),
        (37.25, 88.75),
        (-12.5, 3.125),
    ] {
        let b = full_read(t, h);
        assert_eq!(b.len(), 7);
        assert_eq!(
            b[6],
            oracle::crc8(&b[..6]),
            "CRC over the six answer bytes at {t} °C / {h} %RH"
        );
    }
}

/// The NEGATIVE control: the checksum must MOVE when the measurement moves. A
/// frozen literal would satisfy the test above on a constant part, which is
/// exactly what the deleted model was.
#[test]
fn the_checksum_is_not_a_literal() {
    let a = full_read(25.0, 50.0);
    let b = full_read(26.0, 50.0);
    assert_ne!(a[6], b[6], "a different reading must checksum differently");
}

// ─── BUSY: elapsed time, not a read count ──────────────────────────────────

/// ⚠️ THE NAMED DIFFERENCE. The deleted model cleared BUSY after two status
/// reads and no time at all; this one clears it after the datasheet's 80 ms and
/// no number of reads.
#[test]
fn busy_clears_on_the_datasheet_80ms_and_not_before() {
    let mut d = device();
    d.start();
    for b in TRIGGER {
        d.write(b);
    }
    d.stop();

    // 79 999 µs is not enough.
    d.advance_time_us(79_999);
    d.start();
    let status = d.read();
    assert_eq!(status, 0x88, "BUSY | CAL while the measurement runs");
    assert_eq!(status & 0x80, 0x80, "BUSY is bit 7");

    // The 80 000th microsecond is.
    d.advance_time_us(1);
    d.start();
    assert_eq!(d.read() & 0x80, 0, "BUSY clears at §5.4's 80 ms");
}

/// The NEGATIVE control that separates a real timer from the thunk: POLLING
/// does not make the measurement finish. The deleted model cleared BUSY on the
/// third read; this one is still busy after a thousand.
#[test]
fn no_number_of_reads_clears_busy_without_spending_the_time() {
    let mut d = device();
    d.start();
    for b in TRIGGER {
        d.write(b);
    }
    d.stop();
    for _ in 0..1000 {
        d.start();
        assert_eq!(d.read(), 0x88, "still BUSY, however many times it is asked");
    }

    // …and the deleted model, driven the same way, gave up after two.
    let mut o = oracle::Aht20::new();
    o.start();
    for b in TRIGGER {
        o.write(b);
    }
    for _ in 0..oracle::BUSY_TICKS {
        o.start();
        assert_eq!(o.read() & 0x80, 0x80);
    }
    o.start();
    assert_eq!(
        o.read() & 0x80,
        0,
        "the thunk this port removes: BUSY cleared on a COUNT"
    );
}

/// The second named difference: a read taken mid-measurement answers 0x88 for
/// ALL seven bytes, where the model answered 0x88 and then the READY payload —
/// a measurement simultaneously unfinished and available.
#[test]
fn a_read_during_the_measurement_is_busy_all_the_way_down() {
    let got = run_descriptor(&Script {
        stimulus: vec![("temperature", 25.0), ("humidity", 50.0)],
        command: TRIGGER.to_vec(),
        elapsed_us: 1_000,
        reads: 7,
    });
    assert_eq!(got, vec![0x88; 7]);

    let want_from_model = run_oracle(&Script {
        stimulus: vec![],
        command: TRIGGER.to_vec(),
        elapsed_us: 0,
        reads: 7,
    });
    assert_eq!(want_from_model[0], 0x88, "the model agreed about byte 0 …");
    assert_eq!(
        want_from_model[1], 0x80,
        "… and then served the ready payload anyway"
    );
}

/// The third: a read BEFORE any command is open bus, where the model answered a
/// full measurement to a bus read that never asked for one.
#[test]
fn a_read_before_any_command_is_open_bus() {
    let mut d = device();
    d.start();
    assert_eq!((0..7).map(|_| d.read()).collect::<Vec<_>>(), vec![0xFF; 7]);

    let mut o = oracle::Aht20::new();
    o.start();
    assert_eq!(
        o.read(),
        0x08,
        "the model answered a reading it was never asked for"
    );
}

// ─── the address, and the commands that are accepted and do nothing ────────

#[test]
fn the_address_is_0x38() {
    assert_eq!(device().address(), oracle::Aht20::new().address());
}

#[test]
fn soft_reset_and_init_are_accepted_and_queue_no_response() {
    for code in [0xBAu8, 0xBE] {
        let got = run_descriptor(&Script {
            stimulus: vec![],
            command: vec![code, 0x08, 0x00],
            elapsed_us: 80_000,
            reads: 2,
        });
        assert_eq!(got, vec![0xFF, 0xFF], "opcode 0x{code:02X} answers nothing");
    }
}
