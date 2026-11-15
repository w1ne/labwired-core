// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! RS-485 and Modbus RTU: the MAX485 gate, the `xy-md02` part, and
//! the Arduino Uno example that polls two of them.
//!
//! One test binary on purpose (each integration-test binary links the whole
//! core). The independent decoder for the bus frames is a pymodbus test in
//! `scripts/ci/test_modbus_pymodbus.py`; this file produces the frames it reads.

use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn crc16(data: &[u8]) -> [u8; 2] {
    labwired_config::uart_binary::crc16_modbus(data).to_le_bytes()
}

fn frame(body: &[u8]) -> Vec<u8> {
    let mut v = body.to_vec();
    v.extend_from_slice(&crc16(body));
    v
}

mod sensor {
    //! The part on its own: a UART peer driven byte by byte.

    use super::*;
    use labwired_core::peripherals::components::declarative_uart::{
        DeclarativeUartDevice, DeclarativeUartKit,
    };
    use labwired_core::peripherals::device::UartStreamDevice;
    use labwired_core::sim_input::SimInput;

    pub(super) fn sensor(address: i64, baud: u32) -> DeclarativeUartDevice {
        sensor_id("s", address, baud)
    }

    pub(super) fn sensor_id(id: &str, address: i64, baud: u32) -> DeclarativeUartDevice {
        let yaml = labwired_config::embedded_device_yaml("xy-md02").unwrap();
        let mut dev = DeclarativeUartKit::from_yaml(yaml)
            .unwrap()
            .device(id)
            .unwrap();
        dev.seed_var("address", address);
        dev.set_baud(baud);
        dev.set_input("temperature", 21.5).unwrap();
        dev.set_input("humidity", 48.0).unwrap();
        dev
    }

    /// Send `bytes`, then let the line go quiet and collect the answer.
    pub(super) fn exchange(dev: &mut DeclarativeUartDevice, bytes: &[u8]) -> Vec<u8> {
        for &b in bytes {
            dev.on_tx_byte(b);
        }
        let mut out = Vec::new();
        for _ in 0..800 {
            if let Some(b) = dev.poll(250) {
                out.push(b);
            }
        }
        out
    }

    /// The manufacturer manual's own frames, byte for byte. Source: "XY-MD02"
    /// manual, https://www.hestore.hu/prod_getfile.php?id=18062 (retrieved
    /// 2026-10-04). Two printed CRCs in the manual are wrong and are called
    /// out where they occur.
    mod manual_examples {
        use super::*;

        fn hex(s: &str) -> Vec<u8> {
            s.split_whitespace()
                .map(|h| u8::from_str_radix(h, 16).unwrap())
                .collect()
        }

        #[test]
        fn read_temperature_30_5() {
            let mut dev = sensor(1, 9600);
            dev.set_input("temperature", 30.5).unwrap();
            // Request "Master Read Temperature Command Frame (0x04)".
            let r = exchange(&mut dev, &hex("01 04 00 01 00 01 60 0A"));
            // Response from the manual: temperature 0x131 = 305 = 30.5 C.
            assert_eq!(r, hex("01 04 02 01 31 79 74"));
        }

        #[test]
        fn a_negative_temperature_is_a_signed_word_0xff33() {
            let mut dev = sensor(1, 9600);
            dev.set_input("temperature", -20.5).unwrap();
            let r = exchange(&mut dev, &hex("01 04 00 01 00 01 60 0A"));
            assert_eq!(
                r,
                frame(&[1, 4, 2, 0xFF, 0x33]),
                "manual: 0xFF33 is -20.5 C"
            );
        }

        #[test]
        fn read_humidity_54_6() {
            let mut dev = sensor(1, 9600);
            dev.set_input("humidity", 54.6).unwrap();
            let r = exchange(&mut dev, &hex("01 04 00 02 00 01 90 0A"));
            // The manual prints the CRC of this response as D1 BA; that is
            // wrong. CRC-16/MODBUS of 01 04 02 02 22 is 38 49.
            assert_eq!(r, hex("01 04 02 02 22 38 49"));
        }

        #[test]
        fn read_temperature_and_humidity_in_one_request() {
            let mut dev = sensor(1, 9600);
            dev.set_input("temperature", 30.5).unwrap();
            dev.set_input("humidity", 54.6).unwrap();
            let r = exchange(&mut dev, &hex("01 04 00 01 00 02 20 0B"));
            assert_eq!(r, hex("01 04 04 01 31 02 22 2A CE"));
        }

        #[test]
        fn read_the_device_address_register() {
            let mut dev = sensor(1, 9600);
            let r = exchange(&mut dev, &hex("01 03 01 01 00 01 D4 36"));
            // The manual prints the response with address 0x0102, which
            // contradicts its own default of 1. The part answers 1.
            assert_eq!(r, frame(&[1, 3, 2, 0x00, 0x01]));
        }

        #[test]
        fn write_the_device_address_8() {
            let mut dev = sensor(1, 9600);
            let req = hex("01 06 01 01 00 08 D8 30");
            // Function 06 echoes the request (the manual prints D4 0F as the
            // response CRC, which is the CRC of another frame).
            assert_eq!(exchange(&mut dev, &req), req);
            assert_eq!(
                exchange(&mut dev, &frame(&[8, 3, 1, 1, 0, 1])),
                frame(&[8, 3, 2, 0, 8]),
                "the slave now answers as 8"
            );
        }

        #[test]
        fn write_address_32_and_baud_9600_in_one_function_10_frame() {
            let mut dev = sensor(1, 9600);
            let r = exchange(&mut dev, &hex("01 10 01 01 00 02 04 00 20 25 80 25 09"));
            assert_eq!(r, hex("01 10 01 01 00 02 11 F4"));
            let r = exchange(&mut dev, &frame(&[0x20, 3, 1, 1, 0, 2]));
            assert_eq!(r, frame(&[0x20, 3, 4, 0x00, 0x20, 0x25, 0x80]));
        }
    }

    #[test]
    fn the_register_map_matches_the_manual() {
        let mut dev = sensor(1, 9600);
        // Input registers 0x0001 and 0x0002: temperature and humidity, tenths.
        assert_eq!(
            exchange(&mut dev, &frame(&[1, 4, 0, 1, 0, 2])),
            frame(&[1, 4, 4, 0x00, 0xD7, 0x01, 0xE0])
        );
        // Holding registers 0x0101..0x0104 and their factory values: address
        // 1, 9600 (0x2580) baud, no corrections.
        assert_eq!(
            exchange(&mut dev, &frame(&[1, 3, 1, 1, 0, 4])),
            frame(&[1, 3, 8, 0x00, 0x01, 0x25, 0x80, 0x00, 0x00, 0x00, 0x00])
        );
    }

    #[test]
    fn registers_of_the_wrong_kind_or_outside_the_map_are_exception_02() {
        let mut dev = sensor(1, 9600);
        for req in [
            // Input register 0 and 3 do not exist.
            [1, 4, 0, 0, 0, 1],
            [1, 4, 0, 3, 0, 1],
            // A read that starts inside the map and runs past it.
            [1, 4, 0, 2, 0, 2],
            [1, 3, 1, 4, 0, 2],
            // Holding read of an input register, and the reverse.
            [1, 3, 0, 1, 0, 1],
            [1, 4, 1, 1, 0, 1],
        ] {
            let fc = req[1];
            assert_eq!(
                exchange(&mut dev, &frame(&req)),
                frame(&[1, 0x80 | fc, 2]),
                "{req:?}"
            );
        }
        // A write to an input register.
        assert_eq!(
            exchange(&mut dev, &frame(&[1, 6, 0, 1, 0, 9])),
            frame(&[1, 0x86, 2])
        );
    }

    #[test]
    fn exceptions_for_bad_function_and_count() {
        let mut dev = sensor(1, 9600);
        // 01: illegal function (a diagnostics request).
        assert_eq!(
            exchange(&mut dev, &frame(&[1, 8, 0, 0, 0, 0])),
            frame(&[1, 0x88, 1])
        );
        // 03: illegal data value, a count of zero and a count of 126.
        assert_eq!(
            exchange(&mut dev, &frame(&[1, 4, 0, 1, 0, 0])),
            frame(&[1, 0x84, 3])
        );
        assert_eq!(
            exchange(&mut dev, &frame(&[1, 3, 1, 1, 0, 126])),
            frame(&[1, 0x83, 3])
        );
    }

    #[test]
    fn a_negative_temperature_is_a_signed_16_bit_word() {
        let mut dev = sensor(1, 9600);
        dev.set_input("temperature", -5.5).unwrap();
        let r = exchange(&mut dev, &frame(&[1, 4, 0, 1, 0, 1]));
        assert_eq!(r, frame(&[1, 4, 2, 0xFF, 0xC9]), "-55 tenths is 0xFFC9");
    }

    #[test]
    fn the_answer_follows_the_input_channel() {
        let mut dev = sensor(1, 9600);
        dev.set_input("temperature", 30.0).unwrap();
        let r = exchange(&mut dev, &frame(&[1, 4, 0, 1, 0, 1]));
        assert_eq!(r, frame(&[1, 4, 2, 0x01, 0x2C]));
    }

    #[test]
    fn function_06_writes_the_temperature_correction_and_the_reading_follows() {
        let mut dev = sensor(1, 9600);
        // Register 0x0103 = +0.5 C. The answer echoes the request.
        let req = frame(&[1, 6, 0x01, 0x03, 0x00, 0x05]);
        assert_eq!(exchange(&mut dev, &req), req);
        let r = exchange(&mut dev, &frame(&[1, 4, 0, 1, 0, 1]));
        assert_eq!(r, frame(&[1, 4, 2, 0x00, 0xDC]), "21.5 + 0.5 = 22.0 = 220");
        // The humidity correction moves the humidity, and a negative one is a
        // signed word: -1.0 %RH is 0xFFF6.
        let req = frame(&[1, 6, 0x01, 0x04, 0xFF, 0xF6]);
        assert_eq!(exchange(&mut dev, &req), req);
        let r = exchange(&mut dev, &frame(&[1, 4, 0, 2, 0, 1]));
        assert_eq!(r, frame(&[1, 4, 2, 0x01, 0xD6]), "48.0 - 1.0 = 47.0 = 470");
        // Read back as written.
        assert_eq!(
            exchange(&mut dev, &frame(&[1, 3, 1, 3, 0, 2])),
            frame(&[1, 3, 4, 0x00, 0x05, 0xFF, 0xF6])
        );
    }

    #[test]
    fn corrections_are_limited_to_plus_minus_ten() {
        let mut dev = sensor(1, 9600);
        // +10.0 and -10.0 are the limits and are accepted.
        for v in [100u16, 0xFF9C] {
            let req = frame(&[1, 6, 1, 3, (v >> 8) as u8, v as u8]);
            assert_eq!(exchange(&mut dev, &req), req);
        }
        // +10.1 and -10.1 are not.
        for v in [101u16, 0xFF9B] {
            assert_eq!(
                exchange(&mut dev, &frame(&[1, 6, 1, 3, (v >> 8) as u8, v as u8])),
                frame(&[1, 0x86, 3])
            );
        }
    }

    #[test]
    fn the_baud_register_holds_the_bit_rate() {
        let mut dev = sensor(1, 9600);
        let req = frame(&[1, 6, 0x01, 0x02, 0x4B, 0x00]); // 19200
        assert_eq!(exchange(&mut dev, &req), req);
        assert_eq!(
            exchange(&mut dev, &frame(&[1, 3, 1, 2, 0, 1])),
            frame(&[1, 3, 2, 0x4B, 0x00])
        );
        // Not a supported rate: exception 03.
        assert_eq!(
            exchange(&mut dev, &frame(&[1, 6, 0x01, 0x02, 0x00, 0x07])),
            frame(&[1, 0x86, 3])
        );
    }

    #[test]
    fn function_16_writes_a_range_and_checks_the_byte_count() {
        let mut dev = sensor(1, 9600);
        // Temperature correction -1.0 C.
        let req = frame(&[1, 0x10, 0x01, 0x03, 0x00, 0x01, 0x02, 0xFF, 0xF6]);
        assert_eq!(
            exchange(&mut dev, &req),
            frame(&[1, 0x10, 0x01, 0x03, 0x00, 0x01])
        );
        let r = exchange(&mut dev, &frame(&[1, 4, 0, 1, 0, 1]));
        assert_eq!(r, frame(&[1, 4, 2, 0x00, 0xCD]), "21.5 - 1.0 = 20.5 = 205");
        // A byte count that does not match the register count: exception 03.
        let bad = frame(&[
            1, 0x10, 0x01, 0x03, 0x00, 0x01, 0x04, 0x00, 0x01, 0x00, 0x02,
        ]);
        assert_eq!(exchange(&mut dev, &bad), frame(&[1, 0x90, 3]));
        // A range that leaves the holding registers: exception 02.
        let out = frame(&[1, 0x10, 0x00, 0x01, 0x00, 0x01, 0x02, 0x00, 0x01]);
        assert_eq!(exchange(&mut dev, &out), frame(&[1, 0x90, 2]));
        // An address of 0 inside a two-register write: exception 03, and
        // nothing is written (the baud register keeps its value).
        let bad = frame(&[1, 0x10, 0x01, 0x01, 0x00, 0x02, 0x04, 0, 0, 0x4B, 0x00]);
        assert_eq!(exchange(&mut dev, &bad), frame(&[1, 0x90, 3]));
        assert_eq!(
            exchange(&mut dev, &frame(&[1, 3, 1, 2, 0, 1])),
            frame(&[1, 3, 2, 0x25, 0x80])
        );
    }

    #[test]
    fn writing_the_address_register_moves_the_slave() {
        let mut dev = sensor(1, 9600);
        let req = frame(&[1, 6, 0x01, 0x01, 0x00, 0x09]);
        // The answer still carries the old address, as on hardware.
        assert_eq!(exchange(&mut dev, &req), req);
        assert!(exchange(&mut dev, &frame(&[1, 4, 0, 1, 0, 1])).is_empty());
        assert_eq!(
            exchange(&mut dev, &frame(&[9, 4, 0, 1, 0, 1])),
            frame(&[9, 4, 2, 0x00, 0xD7])
        );
        // Address 0 and 248 are not addresses.
        for v in [0u8, 248] {
            let bad = frame(&[9, 6, 0x01, 0x01, 0x00, v]);
            assert_eq!(exchange(&mut dev, &bad), frame(&[9, 0x86, 3]));
        }
    }

    #[test]
    fn a_wrong_crc_a_wrong_address_and_a_broadcast_get_no_answer() {
        let mut dev = sensor(1, 9600);
        let mut bad = frame(&[1, 4, 0, 1, 0, 2]);
        *bad.last_mut().unwrap() ^= 0x01;
        assert!(exchange(&mut dev, &bad).is_empty(), "wrong CRC");
        assert_eq!(dev.check_errors(), 1);
        assert!(
            exchange(&mut dev, &frame(&[2, 4, 0, 1, 0, 2])).is_empty(),
            "wrong address"
        );
        // No broadcast: the manual has none, so address 0 is ignored and the
        // correction is not written.
        assert!(exchange(&mut dev, &frame(&[0, 6, 0x01, 0x03, 0x00, 0x0A])).is_empty());
        let r = exchange(&mut dev, &frame(&[1, 4, 0, 1, 0, 1]));
        assert_eq!(r, frame(&[1, 4, 2, 0x00, 0xD7]), "still 21.5");
        // A frame shorter than address + function + CRC.
        assert!(exchange(&mut dev, &[0x01, 0x04]).is_empty());
    }

    #[test]
    fn a_gap_longer_than_three_and_a_half_characters_splits_the_frame() {
        // 9600 baud, 10-bit characters: 3.5 characters is 3.65 ms. Send the
        // first half, wait 5 ms, send the second half. Each half is its own
        // frame and neither is a valid request.
        let mut dev = sensor(1, 9600);
        let full = frame(&[1, 4, 0, 1, 0, 2]);
        let mut out = Vec::new();
        for &b in &full[..4] {
            dev.on_tx_byte(b);
        }
        for _ in 0..20 {
            out.extend(dev.poll(250));
        }
        for &b in &full[4..] {
            dev.on_tx_byte(b);
        }
        for _ in 0..800 {
            out.extend(dev.poll(250));
        }
        assert!(
            out.is_empty(),
            "a split frame must not be answered: {out:02X?}"
        );
    }

    #[test]
    fn bytes_closer_than_the_gap_stay_one_frame() {
        // 2 ms between bytes is under 3.65 ms: still one frame.
        let mut dev = sensor(1, 9600);
        let full = frame(&[1, 4, 0, 1, 0, 2]);
        let mut out = Vec::new();
        for &b in &full {
            dev.on_tx_byte(b);
            for _ in 0..8 {
                out.extend(dev.poll(250));
            }
        }
        for _ in 0..800 {
            out.extend(dev.poll(250));
        }
        assert_eq!(out, frame(&[1, 4, 4, 0x00, 0xD7, 0x01, 0xE0]));
    }

    #[test]
    fn the_baud_setting_moves_the_gap_and_the_byte_time() {
        // At 115200 baud the gap is 0.30 ms and a byte is 87 us, so a 100 us
        // pause is NOT a gap at 9600 baud but is one at 115200.
        let mut slow = sensor(1, 9600);
        let mut fast = sensor(1, 115_200);
        let full = frame(&[1, 4, 0, 1, 0, 1]);
        for dev in [&mut slow, &mut fast] {
            for &b in &full[..4] {
                dev.on_tx_byte(b);
            }
        }
        let mut s = Vec::new();
        let mut f = Vec::new();
        for _ in 0..6 {
            s.extend(slow.poll(100));
            f.extend(fast.poll(100));
        }
        for &b in &full[4..] {
            slow.on_tx_byte(b);
            fast.on_tx_byte(b);
        }
        for _ in 0..4000 {
            s.extend(slow.poll(100));
            f.extend(fast.poll(100));
        }
        assert_eq!(
            s,
            frame(&[1, 4, 2, 0x00, 0xD7]),
            "9600: one frame, answered"
        );
        assert!(f.is_empty(), "115200: the 0.6 ms pause split the frame");
    }
}

mod gate {
    //! The transceiver gate on a real `Uart` with two sensors behind it.

    use super::sensor::sensor_id;
    use super::*;
    use labwired_core::peripherals::rs485::{PinSense, Rs485Gate};
    use labwired_core::peripherals::uart::{Uart, UartRegisterLayout};
    use labwired_core::{CycleClock, Peripheral};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    const HZ: u64 = 8_000_000;

    struct Rig {
        uart: Uart,
        clock: CycleClock,
        de: Arc<AtomicBool>,
        re_n: Arc<AtomicBool>,
    }

    fn rig(addresses: &[i64]) -> Rig {
        let mut uart = Uart::new_with_layout(UartRegisterLayout::Stm32F1);
        let clock = CycleClock::default();
        uart.attach_cycle_clock(clock.clone());
        uart.attach_cpu_hz(HZ);
        let de = Arc::new(AtomicBool::new(false));
        let re_n = Arc::new(AtomicBool::new(false));
        uart.set_rs485_gate(Rs485Gate::new(
            "rs485",
            PinSense::Cell(de.clone()),
            PinSense::Cell(re_n.clone()),
        ));
        for (i, a) in addresses.iter().enumerate() {
            // Distinct ids so the bus log names the speaker.
            uart.attach_stream(Box::new(sensor_id(&format!("s{}", i + 1), *a, 9600)));
        }
        // BRR 833 at 8 MHz is 9600 baud.
        uart.write(0x08, (833 & 0xFF) as u8).unwrap();
        uart.write(0x09, (833 >> 8) as u8).unwrap();
        Rig {
            uart,
            clock,
            de,
            re_n,
        }
    }

    impl Rig {
        fn send(&mut self, bytes: &[u8]) {
            for &b in bytes {
                self.uart.write(0x04, b).unwrap();
            }
        }

        /// What firmware does: driver on and receiver off, send, then back to
        /// listening.
        fn transmit(&mut self, bytes: &[u8]) {
            self.de.store(true, Ordering::Relaxed);
            self.re_n.store(true, Ordering::Relaxed);
            self.send(bytes);
            self.de.store(false, Ordering::Relaxed);
            self.re_n.store(false, Ordering::Relaxed);
        }

        /// Run `ms` of simulated time in 1 ms steps and drain the RX queue.
        fn run_ms(&mut self, ms: u64) -> Vec<u8> {
            let mut got = Vec::new();
            for _ in 0..ms {
                self.clock.publish(self.clock.now() + HZ / 1000);
                self.uart.tick();
                while self.uart.read(0x00).unwrap() & 0x20 != 0 {
                    got.push(self.uart.read(0x04).unwrap());
                }
            }
            got
        }
    }

    #[test]
    fn de_high_puts_the_frame_on_the_bus_and_de_low_does_not() {
        let req = frame(&[2, 4, 0, 1, 0, 1]);
        // DE low: the slaves never hear it, so nobody answers.
        let mut r = rig(&[1, 2]);
        r.send(&req);
        assert!(r.run_ms(100).is_empty());
        // DE high, then low again before the slave speaks, as firmware does.
        let mut r = rig(&[1, 2]);
        r.de.store(true, Ordering::Relaxed);
        r.re_n.store(true, Ordering::Relaxed);
        r.send(&req);
        r.de.store(false, Ordering::Relaxed);
        r.re_n.store(false, Ordering::Relaxed);
        assert_eq!(r.run_ms(100), frame(&[2, 4, 2, 0x00, 0xD7]));
    }

    #[test]
    fn only_the_addressed_slave_answers_on_a_multi_drop_bus() {
        let mut r = rig(&[1, 2]);
        r.transmit(&frame(&[1, 4, 0, 1, 0, 1]));
        assert_eq!(r.run_ms(100), frame(&[1, 4, 2, 0x00, 0xD7]));
        r.transmit(&frame(&[2, 4, 0, 2, 0, 1]));
        assert_eq!(r.run_ms(100), frame(&[2, 4, 2, 0x01, 0xE0]));
    }

    #[test]
    fn de_high_and_re_low_echoes_the_masters_own_frame() {
        let req = frame(&[1, 4, 0, 1, 0, 1]);
        let mut r = rig(&[1]);
        r.de.store(true, Ordering::Relaxed); // DE high, /RE low: both on
        r.send(&req);
        // The receiver hears the driver: the echo is in the RX queue at once.
        let mut echo = Vec::new();
        while r.uart.read(0x00).unwrap() & 0x20 != 0 {
            echo.push(r.uart.read(0x04).unwrap());
        }
        assert_eq!(echo, req);
        // With /RE high the receiver is off and there is no echo.
        let mut r = rig(&[1]);
        r.de.store(true, Ordering::Relaxed);
        r.re_n.store(true, Ordering::Relaxed);
        r.send(&req);
        assert_eq!(r.uart.read(0x00).unwrap() & 0x20, 0);
    }

    #[test]
    fn a_receiver_that_is_off_loses_the_answer() {
        let mut r = rig(&[1]);
        r.transmit(&frame(&[1, 4, 0, 1, 0, 1]));
        r.re_n.store(true, Ordering::Relaxed); // /RE high: not listening
        assert!(r.run_ms(100).is_empty());
        let log = r.uart.logs();
        let bus = log.iter().find(|l| l.name == "rs485").unwrap();
        assert!(
            bus.lines()
                .iter()
                .any(|l| l.contains("slave s1: 01 04 02 00 D7")),
            "the answer was on the bus even though nobody heard it: {:?}",
            bus.lines()
        );
    }

    #[test]
    fn a_slave_answering_while_de_is_still_high_is_a_collision() {
        let mut r = rig(&[1]);
        let req = frame(&[1, 4, 0, 1, 0, 1]);
        r.de.store(true, Ordering::Relaxed);
        r.send(&req);
        // Firmware forgot to release DE. The receiver is on, so the master
        // hears its own frame and nothing else.
        let got = r.run_ms(100);
        assert_eq!(got, req, "only the echo arrives; the answer collided");
        let log = r.uart.logs();
        let bus = log.iter().find(|l| l.name == "rs485").unwrap();
        assert!(
            bus.lines().iter().any(|l| l.contains("collision:")),
            "{:?}",
            bus.lines()
        );
        assert!(r.uart.rs485_gate().unwrap().collisions > 0);
    }

    #[test]
    fn two_slaves_at_one_address_collide_and_the_master_hears_nothing() {
        let mut r = rig(&[1, 1]);
        r.transmit(&frame(&[1, 4, 0, 1, 0, 1]));
        // DE low: the master receives, but the two answers collide.
        let got = r.run_ms(100);
        assert!(
            got.is_empty(),
            "collided bytes are not delivered: {got:02X?}"
        );
        assert!(r.uart.rs485_gate().unwrap().collisions > 0);
    }

    /// Feed requests built by an independent encoder (pymodbus, see
    /// `scripts/ci/test_modbus_pymodbus.py`) through the transceiver and report
    /// what the master heard. A no-op unless the script names its files.
    ///
    /// Input: `[{"name": .., "chunks": [{"hex": .., "wait_ms": ..}]}]`, every
    /// scenario on a fresh bus with slaves at addresses 1 and 2. Output:
    /// `[{"name": .., "rx": hex, "bus": [log lines]}]`.
    #[test]
    fn replay_requests_from_the_pymodbus_gate() {
        let (Ok(input), Ok(output)) = (
            std::env::var("LABWIRED_MODBUS_REPLAY_IN"),
            std::env::var("LABWIRED_MODBUS_REPLAY_OUT"),
        ) else {
            return;
        };
        let scenarios: Vec<serde_json::Value> =
            serde_json::from_str(&std::fs::read_to_string(input).unwrap()).unwrap();
        let hex = |b: &[u8]| {
            b.iter()
                .map(|x| format!("{x:02X}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        let mut out = Vec::new();
        for sc in scenarios {
            let mut r = rig(&[1, 2]);
            let mut rx = Vec::new();
            for chunk in sc["chunks"].as_array().unwrap() {
                let bytes: Vec<u8> = chunk["hex"]
                    .as_str()
                    .unwrap()
                    .split_whitespace()
                    .map(|h| u8::from_str_radix(h, 16).unwrap())
                    .collect();
                r.transmit(&bytes);
                rx.extend(r.run_ms(chunk["wait_ms"].as_u64().unwrap()));
            }
            let log = r.uart.logs();
            let bus = log.iter().find(|l| l.name == "rs485").unwrap().lines();
            out.push(serde_json::json!({ "name": sc["name"], "rx": hex(&rx), "bus": bus }));
        }
        std::fs::write(output, serde_json::to_string_pretty(&out).unwrap()).unwrap();
    }

    #[test]
    fn the_bus_log_names_who_spoke() {
        let mut r = rig(&[1, 2]);
        r.transmit(&frame(&[2, 4, 0, 1, 0, 1]));
        r.run_ms(100);
        let log = r.uart.logs();
        let lines = log.iter().find(|l| l.name == "rs485").unwrap().lines();
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].contains("master: 02 04 00 01 00 01"), "{lines:?}");
        assert!(lines[1].contains("slave s2: 02 04 02 00 D7"), "{lines:?}");
    }
}

mod example {
    //! The Uno example end to end: ModbusMaster firmware, a MAX485, two sensors.

    use super::*;
    use labwired_config::{ChipDescriptor, SystemManifest};
    use labwired_core::bus::SystemBus;
    use labwired_core::cpu::Avr;
    use labwired_core::Machine;
    use std::sync::{Arc, Mutex};

    pub struct Rig {
        pub machine: Machine<Avr>,
        pub serial: Arc<Mutex<Vec<u8>>>,
    }

    pub fn boot() -> Rig {
        let yaml = root().join("examples/arduino-uno-modbus-rtu/system.yaml");
        let manifest = SystemManifest::from_file(&yaml).expect("load system.yaml");
        let chip = ChipDescriptor::from_file(yaml.parent().unwrap().join(&manifest.chip))
            .expect("load chip");
        let bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
        let elf = std::fs::read(root().join("tests/fixtures/avr/arduino-uno-modbus-rtu.elf"))
            .expect("missing fixture; build examples/arduino-uno-modbus-rtu");
        let image = labwired_loader::load_elf_bytes(&elf).expect("parse ELF");
        let mut cpu = Avr::new();
        cpu.load_program_image(&image);
        let serial = Arc::new(Mutex::new(Vec::new()));
        cpu.set_serial_sink(serial.clone());
        Rig {
            machine: Machine::new(cpu, bus),
            serial,
        }
    }

    impl Rig {
        pub fn text(&self) -> String {
            String::from_utf8_lossy(&self.serial.lock().unwrap()).into_owned()
        }

        /// Step until the console contains `needle`.
        pub fn run_until(&mut self, needle: &str) {
            for _ in 0..40_000_000u32 {
                self.machine.step().expect("step");
                if self.text().contains(needle) {
                    return;
                }
            }
            panic!(
                "never printed {needle:?}; console: {:?}\nbus: {:#?}",
                self.text(),
                self.bus_log()
            );
        }

        pub fn bus_log(&self) -> Vec<String> {
            let logs = self.machine.bus.peripheral_logs("usart0").expect("usart0");
            logs.iter()
                .find(|l| l.name == "rs485")
                .expect("rs485 log")
                .lines()
        }
    }

    #[test]
    fn the_master_reads_both_sensors_and_the_stimulus_shows_up() {
        let mut rig = boot();
        rig.run_until("poll 1 s2 T=19.0 H=55.5");
        assert!(
            rig.text().contains("poll 1 s1 T=21.5 H=48.0"),
            "{}",
            rig.text()
        );
        rig.run_until("poll 2 s2 write correction -> 0x0");
        assert!(
            rig.text().contains("poll 2 s1 reg 0x0200 -> 0x2"),
            "{}",
            rig.text()
        );
        rig.run_until("poll 3 s2 T=19.5 H=55.5");
        rig.machine
            .set_input_on("s1", "temperature", 25.0)
            .expect("warm slave 1");
        rig.run_until("poll 4 s1 T=25.0 H=48.0");
        let log = rig.bus_log();
        assert!(
            log.iter()
                .any(|l| l.contains("master: 01 04 00 01 00 02 20 0B")),
            "{log:#?}"
        );
        assert!(
            log.iter()
                .any(|l| l.contains("slave s1: 01 04 04 00 D7 01 E0 4B A4")),
            "{log:#?}"
        );
        assert!(!log.iter().any(|l| l.contains("collision")), "{log:#?}");
        if let Ok(out) = std::env::var("LABWIRED_MODBUS_FRAMES_OUT") {
            let frames: Vec<serde_json::Value> = log
                .iter()
                .filter_map(|l| {
                    let (who, hex) = l.split_once(": ")?;
                    let t = who
                        .split_whitespace()
                        .next()
                        .filter(|w| w.starts_with("t="));
                    let who = who.split_whitespace().last()?;
                    Some(serde_json::json!({ "t": t, "from": who, "hex": hex }))
                })
                .collect();
            std::fs::write(out, serde_json::to_string_pretty(&frames).unwrap()).unwrap();
        }
    }
}
