// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! DFPlayer Mini (`configs/devices/dfplayer_mini.yaml`): the 10-byte serial
//! frames are decoded from the RX pad at 9600 8N1, the checksum is checked, and
//! a play command drives BUSY low for the nominal clip window.

use labwired_config::DeviceDescriptor;
use labwired_core::bus::{BusResidentDevice, DevicePins};
use labwired_core::peripherals::components::declarative_gpio::{BoundPin, DeclarativeGpioDevice};

const HZ: u64 = 16_000_000;
const STEP: u64 = 16; // service every 1 us

struct Line {
    rx: bool,
}
impl DevicePins for Line {
    fn output_bit(&self, _: u64, _: u8) -> Option<bool> {
        Some(self.rx)
    }
    fn drive_input_bit(&mut self, _: u64, _: u8, _: bool) -> bool {
        true
    }
    fn drive_idr_bit(&mut self, _: u64, _: u8, _: bool) {}
}

struct Player {
    dev: DeclarativeGpioDevice,
    line: Line,
    now: u64,
}

impl Player {
    fn new() -> Self {
        let desc = DeviceDescriptor::embedded("dfplayer-mini")
            .unwrap()
            .unwrap();
        let pin = |role: &str, bit| BoundPin {
            role: role.into(),
            addr: 0x100,
            bit,
        };
        let dev = DeclarativeGpioDevice::new(
            "U2".into(),
            &desc,
            vec![pin("RX", 3)],
            vec![pin("BUSY", 7), pin("TX", 2)],
            HZ,
            std::borrow::Cow::Owned(vec![]),
        )
        .unwrap();
        let mut p = Self {
            dev,
            line: Line { rx: true },
            now: 0,
        };
        p.idle_us(100);
        p
    }
    fn run_until(&mut self, until: u64) {
        while self.now < until {
            self.dev.service(&mut self.line, self.now);
            self.now += STEP;
        }
    }
    fn idle_us(&mut self, us: u64) {
        self.line.rx = true;
        let t = self.now + us * HZ / 1_000_000;
        self.run_until(t);
    }
    /// Send bytes on RX the way a UART TX does: start bit, 8 data bits LSB
    /// first, stop bit, back to back.
    fn send(&mut self, bytes: &[u8]) {
        let t0 = self.now;
        let bit_at = |k: u64| t0 + k * HZ / 9600;
        let mut k = 0;
        for &b in bytes {
            let mut levels = vec![false];
            levels.extend((0..8).map(|i| (b >> i) & 1 == 1));
            levels.push(true);
            for level in levels {
                self.line.rx = level;
                self.run_until(bit_at(k + 1));
                k += 1;
            }
        }
        self.line.rx = true;
    }
    fn busy(&self) -> bool {
        self.dev.rule_machine().pin_level("BUSY").unwrap()
    }
    fn var(&self, name: &str) -> i64 {
        self.dev.rule_machine().var(name)
    }
}

/// The frame a DFPlayer library builds: checksum = 0 - sum(bytes 1..=6).
fn frame(cmd: u8, arg: u16) -> [u8; 10] {
    let mut f = [
        0x7E,
        0xFF,
        0x06,
        cmd,
        0x00,
        (arg >> 8) as u8,
        arg as u8,
        0,
        0,
        0xEF,
    ];
    let sum: u16 = f[1..7].iter().map(|&b| u16::from(b)).sum();
    let cs = 0u16.wrapping_sub(sum);
    f[7] = (cs >> 8) as u8;
    f[8] = cs as u8;
    f
}

#[test]
fn checksums_match_the_manual_examples() {
    assert_eq!(
        frame(0x03, 1),
        [0x7E, 0xFF, 0x06, 0x03, 0x00, 0x00, 0x01, 0xFE, 0xF7, 0xEF]
    );
    assert_eq!(
        frame(0x06, 20),
        [0x7E, 0xFF, 0x06, 0x06, 0x00, 0x00, 0x14, 0xFE, 0xE1, 0xEF]
    );
}

#[test]
fn idle_module_holds_busy_high() {
    let mut p = Player::new();
    p.idle_us(5_000);
    assert!(p.busy());
    assert_eq!(p.var("frames"), 0);
}

#[test]
fn volume_frame_is_decoded_and_does_not_play() {
    let mut p = Player::new();
    p.send(&frame(0x06, 20));
    p.idle_us(2_000);
    assert_eq!(p.var("frames"), 1);
    assert_eq!(p.var("volume"), 20);
    assert!(p.busy(), "volume must not start playback");
    p.send(&frame(0x06, 45));
    p.idle_us(2_000);
    assert_eq!(p.var("volume"), 30, "volume is clamped to 30");
}

#[test]
fn play_frame_drives_busy_low_for_a_finite_window() {
    let mut p = Player::new();
    p.send(&frame(0x03, 2));
    p.idle_us(1_000);
    assert_eq!(p.var("frames"), 1);
    assert_eq!((p.var("track"), p.var("plays")), (2, 1));
    assert!(!p.busy(), "BUSY is LOW while playing");
    p.idle_us(990_000);
    assert!(!p.busy(), "still playing just before the 1 s window ends");
    p.idle_us(20_000);
    assert!(p.busy(), "BUSY released HIGH when the clip ends");
}

#[test]
fn stop_releases_busy() {
    let mut p = Player::new();
    p.send(&frame(0x03, 1));
    p.idle_us(1_000);
    assert!(!p.busy());
    p.send(&frame(0x16, 0));
    p.idle_us(1_000);
    assert!(p.busy());
}

#[test]
fn a_bad_checksum_is_rejected() {
    let mut p = Player::new();
    let mut f = frame(0x03, 1);
    f[8] ^= 0x01;
    p.send(&f);
    p.idle_us(2_000);
    assert_eq!((p.var("frames"), p.var("bad")), (0, 1));
    assert!(p.busy(), "a corrupt frame must not play");
}

#[test]
fn a_frame_after_line_noise_is_still_found() {
    let mut p = Player::new();
    p.send(&[0x00, 0x55]);
    p.idle_us(500);
    p.send(&frame(0x03, 3));
    p.idle_us(1_000);
    assert_eq!((p.var("frames"), p.var("track")), (1, 3));
    assert!(!p.busy());
}
