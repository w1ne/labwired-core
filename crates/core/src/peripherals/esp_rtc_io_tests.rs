// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Tests of the RTC_IO model (`esp_rtc_io`), alone and paired with the GPIO
//! model of its part, the way the bus pairs them.

use super::*;
use crate::peripherals::esp32::gpio::Esp32Gpio;
use crate::peripherals::esp32s3::gpio::Esp32s3Gpio;
use crate::pins::{Out, PinPort};

/// Classic GPIO registers used here (esp-idf `soc/esp32/include/soc/gpio_reg.h`).
const GPIO_OUT_W1TS: u64 = 0x08;
const GPIO_OUT_W1TC: u64 = 0x0C;
const GPIO_ENABLE_W1TS: u64 = 0x24;
const GPIO_IN: u64 = 0x3C;
const GPIO_IN1: u64 = 0x40;

/// Classic pad registers (`esp32.svd`).
const PAD_DAC1: u64 = 0x84;
const PAD_DAC2: u64 = 0x88;
const XTAL_32K_PAD: u64 = 0x8C;
const ADC_PAD: u64 = 0x80;
const TOUCH_PAD1: u64 = 0x98;
const TOUCH_PAD7: u64 = 0xB0;
/// Their MUX_SEL / FUN_IE / RUE / RDE / HOLD bits (TOUCH_PADn, PAD_DACn).
const TOUCH_MUX_SEL: u32 = 1 << 19;
const TOUCH_FUN_IE: u32 = 1 << 13;
const TOUCH_HOLD: u32 = 1 << 31;
const DAC_MUX_SEL: u32 = 1 << 17;
const DAC_FUN_IE: u32 = 1 << 11;
const RUE: u32 = 1 << 27;
const RDE: u32 = 1 << 28;

/// A classic RTC_IO + GPIO pair sharing pads, as the bus wires them.
fn classic() -> (EspRtcIo, Esp32Gpio) {
    let rtc = EspRtcIo::new_esp32();
    let mut gpio = Esp32Gpio::new();
    assert!(gpio.attach_rtc_io_pads(rtc.rtc_io_pads().unwrap()));
    (rtc, gpio)
}

fn s3() -> (EspRtcIo, Esp32s3Gpio) {
    let rtc = EspRtcIo::new_esp32s3();
    let mut gpio = Esp32s3Gpio::new();
    assert!(gpio.attach_rtc_io_pads(rtc.rtc_io_pads().unwrap()));
    (rtc, gpio)
}

fn set(p: &mut dyn Peripheral, off: u64, bits: u32) {
    let v = p.read_u32(off).unwrap() | bits;
    p.write_u32(off, v).unwrap();
}

fn clear(p: &mut dyn Peripheral, off: u64, bits: u32) {
    let v = p.read_u32(off).unwrap() & !bits;
    p.write_u32(off, v).unwrap();
}

/// One register of an SVD peripheral, as the SVD spells it.
#[derive(Debug, PartialEq, Eq)]
struct SvdReg {
    name: String,
    offset: u64,
    reset: u32,
    /// Bits of the fields that are not read-only.
    wmask: u32,
}

fn tag<'a>(body: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let at = body.find(&open)? + open.len();
    let end = body[at..].find(&close)? + at;
    Some(body[at..end].trim())
}

fn num(text: &str) -> u64 {
    match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16).unwrap(),
        None => text.parse().unwrap(),
    }
}

/// Every `<open>..</close>` block of `body`, in order.
fn blocks<'a>(body: &'a str, open: &str, close: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(start) = body[at..].find(open) {
        let start = at + start;
        let end = body[start..].find(close).unwrap() + start;
        out.push(&body[start + open.len()..end]);
        at = end + close.len();
    }
    out
}

/// The registers of SVD peripheral `periph` in `svd` (a repo path), `dim`
/// arrays expanded. A register without `resetValue` takes the device default,
/// which both ESP SVDs declare as 0x00000000.
fn svd_registers(svd: &str, periph: &str) -> Vec<SvdReg> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(svd);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{svd}: {e}"));
    let block = blocks(&text, "<peripheral>", "</peripheral>")
        .into_iter()
        .find(|b| tag(b, "name") == Some(periph))
        .unwrap_or_else(|| panic!("{svd} has no {periph}"));
    let mut regs = Vec::new();
    for reg in blocks(block, "<register>", "</register>") {
        let name = tag(reg, "name").unwrap().to_string();
        let offset = num(tag(reg, "addressOffset").unwrap());
        let reset = tag(reg, "resetValue").map(num).unwrap_or(0) as u32;
        let dim = tag(reg, "dim").map(num).unwrap_or(1);
        let inc = tag(reg, "dimIncrement").map(num).unwrap_or(0);
        let mut wmask = 0u32;
        for field in blocks(reg, "<field>", "</field>") {
            let bit = num(tag(field, "bitOffset").unwrap());
            let width = num(tag(field, "bitWidth").unwrap());
            if tag(field, "access") != Some("read-only") {
                wmask |= (((1u64 << width) - 1) << bit) as u32;
            }
        }
        for i in 0..dim {
            regs.push(SvdReg {
                name: name.replace("%s", &i.to_string()),
                offset: offset + i * inc,
                reset,
                wmask,
            });
        }
    }
    regs
}

/// Every architected register sits where the SVD puts it, with its reset
/// value and writable bits, and a fresh model reads every reset value back
/// (`IN` reads 0 on an idle bus, the write-only views 0).
#[test]
fn registers_match_the_svd() {
    for (svd, regs, fresh) in [
        (
            "tests/fixtures/real_world/esp32.svd",
            ESP32_REGS,
            EspRtcIo::new_esp32(),
        ),
        (
            "tests/fixtures/svd/esp32s3.svd",
            ESP32S3_REGS,
            EspRtcIo::new_esp32s3(),
        ),
    ] {
        let from_svd = svd_registers(svd, "RTC_IO");
        assert_eq!(from_svd.len(), regs.len(), "{svd}: register count");
        for (want, &(offset, reset, wmask)) in from_svd.iter().zip(regs) {
            assert_eq!(
                (want.offset, want.reset, want.wmask),
                (offset, reset, wmask),
                "{svd}: {}",
                want.name
            );
            assert_eq!(
                fresh.read_u32(offset).unwrap(),
                reset,
                "{svd}: {} reads its reset value",
                want.name
            );
        }
    }
}

/// `MUX_SEL` resets to 0 on every pad of both parts: every pad starts in
/// the digital domain, the XTAL_32K pads (GPIO32/33) included.
#[test]
fn every_pad_resets_digital() {
    assert_eq!(EspRtcIo::new_esp32().pads().view().rtc, 0);
    assert_eq!(EspRtcIo::new_esp32s3().pads().view().rtc, 0);
}

/// Classic RTC GPIO numbers are not GPIO numbers: GPIO0 is RTC_GPIO11, at
/// bit 14 + 11 = 25 of RTC_GPIO_OUT / ENABLE, through TOUCH_PAD1.
#[test]
fn classic_gpio0_is_rtc_gpio11_at_bit_25() {
    let (mut rtc, gpio) = classic();
    let pads = rtc.pads();
    assert_eq!(pads.rtc_channel(0), Some(11));
    assert_eq!(pads.rtc_channel(32), Some(9), "X32P");
    assert_eq!(pads.rtc_channel(33), Some(8), "X32N");
    assert_eq!(pads.rtc_channel(36), Some(0));
    assert_eq!(pads.rtc_channel(5), None, "GPIO5 has no RTC function");
    set(&mut rtc, TOUCH_PAD1, TOUCH_MUX_SEL);
    rtc.write_u32(OUT_W1TS, 1 << 25).unwrap();
    rtc.write_u32(ENABLE_W1TS, 1 << 25).unwrap();
    assert_eq!(rtc.read_u32(OUT).unwrap(), 1 << 25);
    assert_eq!(rtc.read_u32(OUT_W1TS).unwrap(), 0, "write-only view");
    let d = gpio.driver(0).unwrap();
    assert_eq!(d.out, Out::High, "RTC_GPIO11 drives GPIO0");
    // The pad's own RTC pull-up (TOUCH_PAD1 reset 0x4A00_0000: RUE) stays.
    assert_eq!(d.pull, Pull::Up);
    // Nothing else moved: GPIO4 (RTC_GPIO10, bit 24) is still digital.
    assert_eq!(gpio.driver(4).unwrap().out, Out::Off);
}

/// `MUX_SEL` hands the pad between the domains: GPIO_OUT / ENABLE drive it
/// while it is clear, the RTC GPIO while it is set, and the GPIO latch is
/// back in charge the moment it clears.
#[test]
fn mux_sel_hands_the_pad_between_the_domains() {
    let (mut rtc, mut gpio) = classic();
    gpio.write_u32(GPIO_ENABLE_W1TS, 1 << 25).unwrap();
    gpio.write_u32(GPIO_OUT_W1TS, 1 << 25).unwrap();
    assert_eq!(gpio.driver(25).unwrap().out, Out::High, "digital");
    // RTC_GPIO6 (PAD_DAC1, bit 20), output stage off: the pad is released
    // although GPIO_ENABLE still has it.
    set(&mut rtc, PAD_DAC1, DAC_MUX_SEL);
    assert_eq!(gpio.driver(25).unwrap().out, Out::Off, "RTC, not enabled");
    rtc.write_u32(ENABLE_W1TS, 1 << 20).unwrap();
    assert_eq!(gpio.driver(25).unwrap().out, Out::Low, "RTC_GPIO_OUT low");
    gpio.write_u32(GPIO_OUT_W1TC, 1 << 25).unwrap();
    rtc.write_u32(OUT_W1TS, 1 << 20).unwrap();
    assert_eq!(gpio.driver(25).unwrap().out, Out::High, "RTC_GPIO_OUT high");
    clear(&mut rtc, PAD_DAC1, DAC_MUX_SEL);
    assert_eq!(
        gpio.driver(25).unwrap().out,
        Out::Low,
        "digital again: the GPIO latch (now low)"
    );
}

/// `RTC_GPIO_IN` and `GPIO_IN` read an RTC-domain pad through its `FUN_IE`:
/// 0 while it is clear, the pad once it is set.
#[test]
fn fun_ie_gates_the_input_on_both_sides() {
    let (mut rtc, mut gpio) = classic();
    // GPIO26 = RTC_GPIO7, bit 21.
    set(&mut rtc, PAD_DAC2, DAC_MUX_SEL);
    gpio.set_external(26, External::Level(true));
    assert_eq!(rtc.read_u32(IN).unwrap() & (1 << 21), 0, "FUN_IE clear");
    assert_eq!(gpio.read_u32(GPIO_IN).unwrap() & (1 << 26), 0);
    assert!(gpio.level(26).unwrap(), "a probe still sees the wire");
    set(&mut rtc, PAD_DAC2, DAC_FUN_IE);
    assert_eq!(rtc.read_u32(IN).unwrap() & (1 << 21), 1 << 21);
    assert_ne!(gpio.read_u32(GPIO_IN).unwrap() & (1 << 26), 0);
    gpio.set_external(26, External::Level(false));
    assert_eq!(rtc.read_u32(IN).unwrap() & (1 << 21), 0);
    // A digital-domain pad reads in RTC_GPIO_IN as GPIO_IN reads it: GPIO0
    // (RTC_GPIO11, bit 25) on its reset pull-up, GPIO4 (bit 24) pulled down.
    let rtc_in = rtc.read_u32(IN).unwrap();
    assert_eq!(rtc_in & (3 << 24), 1 << 25, "{rtc_in:#x}");
}

/// A floating RTC-domain pad sits at its `RUE` / `RDE` pull.
#[test]
fn rue_and_rde_hold_a_floating_pad() {
    let (mut rtc, gpio) = classic();
    // GPIO27 = RTC_GPIO17 (TOUCH_PAD7, reset: no pull), bit 31.
    set(&mut rtc, TOUCH_PAD7, TOUCH_MUX_SEL | TOUCH_FUN_IE | RUE);
    assert_eq!(rtc.read_u32(IN).unwrap() & (1 << 31), 1 << 31, "pulled up");
    assert_eq!(gpio.driver(27).unwrap(), PadDriver::released(Pull::Up));
    assert!(gpio.level(27).unwrap());
    clear(&mut rtc, TOUCH_PAD7, RUE);
    set(&mut rtc, TOUCH_PAD7, RDE);
    assert_eq!(rtc.read_u32(IN).unwrap() & (1 << 31), 0, "pulled down");
    assert_eq!(gpio.driver(27).unwrap(), PadDriver::released(Pull::Down));
    assert!(!gpio.level(27).unwrap());
}

/// Classic errata 3.6: an RTC-capable pad's pull is its RTC_IO `RUE` /
/// `RDE` in the digital domain too, so the silicon reset pulls (TOUCH_PAD1
/// RUE on GPIO0, TOUCH_PAD0 RDE on GPIO4) are on a digital input at reset.
/// The S3 keeps digital pulls apart: its RTC pulls act only in RTC mode.
#[test]
fn classic_digital_pads_take_the_rtc_pulls_the_s3_does_not() {
    let (_rtc, gpio) = classic();
    assert_eq!(gpio.driver(0).unwrap(), PadDriver::released(Pull::Up));
    assert_eq!(gpio.driver(4).unwrap(), PadDriver::released(Pull::Down));
    assert_eq!(gpio.driver(5).unwrap(), PadDriver::OFF, "no RTC function");
    assert_eq!(
        gpio.driver(32).unwrap(),
        PadDriver::OFF,
        "X32P: no reset pull"
    );
    assert_eq!(
        gpio.read_u32(GPIO_IN).unwrap() & 1,
        1,
        "GPIO0 released at reset reads its pull-up"
    );
    assert!(!gpio.input(4).unwrap());

    let (_rtc, s3gpio) = s3();
    // TOUCH_PAD0 (GPIO0) resets with RDE set, digital: no pull from RTC_IO.
    assert_eq!(s3gpio.driver(0).unwrap().pull, Pull::None);
}

/// `RTC_GPIO_PINn.PAD_DRIVER` makes the RTC output open drain: a 1 is
/// released, a 0 driven.
#[test]
fn pad_driver_makes_the_rtc_output_open_drain() {
    let (mut rtc, gpio) = classic();
    set(&mut rtc, PAD_DAC1, DAC_MUX_SEL);
    rtc.write_u32(ENABLE_W1TS, 1 << 20).unwrap();
    rtc.write_u32(PIN0 + 4 * 6, 1 << 2).unwrap();
    rtc.write_u32(OUT_W1TS, 1 << 20).unwrap();
    assert_eq!(gpio.driver(25).unwrap().out, Out::Off, "open-drain 1");
    rtc.write_u32(OUT_W1TC, 1 << 20).unwrap();
    assert_eq!(gpio.driver(25).unwrap().out, Out::Low, "open-drain 0");
}

/// `HOLD` freezes the RTC output stage as it was when it rose.
#[test]
fn hold_freezes_the_rtc_output() {
    let (mut rtc, gpio) = classic();
    // GPIO0 = RTC_GPIO11, bit 25.
    set(&mut rtc, TOUCH_PAD1, TOUCH_MUX_SEL);
    rtc.write_u32(ENABLE_W1TS, 1 << 25).unwrap();
    rtc.write_u32(OUT_W1TS, 1 << 25).unwrap();
    set(&mut rtc, TOUCH_PAD1, TOUCH_HOLD);
    rtc.write_u32(OUT_W1TC, 1 << 25).unwrap();
    rtc.write_u32(ENABLE_W1TC, 1 << 25).unwrap();
    clear(&mut rtc, TOUCH_PAD1, RUE);
    assert_eq!(rtc.read_u32(OUT).unwrap(), 0, "the register still stores");
    let d = gpio.driver(0).unwrap();
    assert_eq!((d.out, d.pull), (Out::High, Pull::Up), "held");
    clear(&mut rtc, TOUCH_PAD1, TOUCH_HOLD);
    let d = gpio.driver(0).unwrap();
    assert_eq!((d.out, d.pull), (Out::Off, Pull::None), "released");
}

/// GPIO34..39 (RTC_GPIO0..5) are input-only in the RTC domain too.
#[test]
fn input_only_pads_never_drive() {
    let (mut rtc, gpio) = classic();
    // GPIO34 = RTC_GPIO4 (ADC1: MUX_SEL 29, FUN_IE 23), bit 18.
    set(&mut rtc, ADC_PAD, (1 << 29) | (1 << 23));
    rtc.write_u32(ENABLE_W1TS, 1 << 18).unwrap();
    rtc.write_u32(OUT_W1TS, 1 << 18).unwrap();
    assert_eq!(gpio.driver(34).unwrap(), PadDriver::OFF);
    let mut gpio = gpio;
    gpio.set_external(34, External::Level(true));
    assert_eq!(rtc.read_u32(IN).unwrap() & (1 << 18), 1 << 18);
    assert_ne!(gpio.read_u32(GPIO_IN1).unwrap() & (1 << 2), 0);
}

/// `RTC_GPIO_PINn.INT_TYPE` latches `RTC_GPIO_STATUS`: edges from the
/// outside and from the pad's own RTC output, cleared by `STATUS_W1TC`; a
/// level type re-latches while its level holds.
#[test]
fn rtc_gpio_interrupts_latch_and_clear() {
    let (mut rtc, mut gpio) = classic();
    // GPIO26 = RTC_GPIO7, bit 21; rising edge.
    set(&mut rtc, PAD_DAC2, DAC_MUX_SEL | DAC_FUN_IE);
    rtc.write_u32(PIN0 + 4 * 7, 1 << 7).unwrap();
    gpio.set_external(26, External::Level(false));
    assert_eq!(rtc.read_u32(STATUS).unwrap(), 0);
    gpio.set_external(26, External::Level(true));
    assert_eq!(rtc.read_u32(STATUS).unwrap(), 1 << 21, "rising edge");
    rtc.write_u32(STATUS_W1TC, 1 << 21).unwrap();
    assert_eq!(rtc.read_u32(STATUS).unwrap(), 0);
    gpio.set_external(26, External::Level(false));
    assert_eq!(rtc.read_u32(STATUS).unwrap(), 0, "falling: not latched");
    // High level: latched while high, again after every clear.
    rtc.write_u32(PIN0 + 4 * 7, 5 << 7).unwrap();
    assert_eq!(rtc.read_u32(STATUS).unwrap(), 0);
    gpio.set_external(26, External::Level(true));
    rtc.write_u32(STATUS_W1TC, 1 << 21).unwrap();
    assert_eq!(rtc.read_u32(STATUS).unwrap(), 1 << 21, "level holds");
    gpio.set_external(26, External::Level(false));
    rtc.write_u32(STATUS_W1TC, 1 << 21).unwrap();
    assert_eq!(rtc.read_u32(STATUS).unwrap(), 0);
    // The pad's own RTC output edge counts (input buffer on): any edge.
    gpio.set_external(26, External::Released);
    rtc.write_u32(PIN0 + 4 * 7, 3 << 7).unwrap();
    rtc.write_u32(ENABLE_W1TS, 1 << 21).unwrap();
    rtc.write_u32(OUT_W1TS, 1 << 21).unwrap();
    assert_eq!(rtc.read_u32(STATUS).unwrap(), 1 << 21, "own output edge");
    // STATUS_W1TS sets, as the TRM says.
    rtc.write_u32(STATUS_W1TC, u32::MAX).unwrap();
    rtc.write_u32(STATUS_W1TS, 1 << 14).unwrap();
    assert_eq!(rtc.read_u32(STATUS).unwrap(), 1 << 14);
}

/// The matrix signal routed to a pad goes dark while the RTC IO MUX owns
/// it, and is back the moment `MUX_SEL` clears.
#[test]
fn a_matrix_route_is_dark_in_the_rtc_domain() {
    use crate::peripherals::esp32::gpio::VSPI_SIGNALS;
    use crate::peripherals::esp_gpspi_wire::{LINE_SCK, SPI_NET_LINES};
    use crate::peripherals::pad_lines::{LineDrive, PadLines};
    let (mut rtc, mut gpio) = classic();
    let wire = Arc::new(PadLines::new(SPI_NET_LINES, &[false, false, true, false]));
    gpio.bind_spi_lines(&wire, VSPI_SIGNALS);
    // GPIO25 routed to VSPICLK (63) and driven by the line.
    gpio.write_u32(0x530 + 4 * 25, 63).unwrap();
    gpio.write_u32(GPIO_ENABLE_W1TS, 1 << 25).unwrap();
    wire.drive_line(LINE_SCK, LineDrive::PushPull, true);
    assert_eq!(gpio.driver(25).unwrap().out, Out::High, "the matrix route");
    set(&mut rtc, PAD_DAC1, DAC_MUX_SEL);
    assert_eq!(gpio.driver(25).unwrap().out, Out::Off, "dark: RTC owns it");
    assert!(!gpio.level(25).unwrap());
    let routing = gpio.gpio_routing(25).unwrap();
    assert_eq!(routing.func.as_deref(), Some("RTC_GPIO6"));
    clear(&mut rtc, PAD_DAC1, DAC_MUX_SEL);
    assert_eq!(gpio.driver(25).unwrap().out, Out::High, "the route is back");
}

/// S3: RTC_GPIOn is GPIOn, at bit 10 + n; `MUX_SEL` hands GPIO5 over.
#[test]
fn s3_rtc_gpio_n_is_gpio_n_at_bit_10_plus_n() {
    let (mut rtc, mut gpio) = s3();
    assert_eq!(rtc.pads().rtc_channel(21), Some(21));
    assert_eq!(rtc.pads().rtc_channel(22), None);
    // GPIO5: TOUCH_PAD5 at 0x84 + 4 * 5.
    let pad5 = 0x84 + 4 * 5;
    gpio.write_u32(0x24, 1 << 5).unwrap(); // GPIO_ENABLE_W1TS
    set(&mut rtc, pad5, TOUCH_MUX_SEL | TOUCH_FUN_IE);
    rtc.write_u32(ENABLE_W1TS, 1 << 15).unwrap();
    rtc.write_u32(OUT_W1TS, 1 << 15).unwrap();
    assert_eq!(gpio.driver(5).unwrap().out, Out::High);
    assert_eq!(rtc.read_u32(IN).unwrap(), 1 << 15, "reads its own output");
    assert_ne!(gpio.read_u32(0x3C).unwrap() & (1 << 5), 0, "GPIO_IN too");
    clear(&mut rtc, pad5, TOUCH_MUX_SEL);
    assert_eq!(gpio.driver(5).unwrap().out, Out::Low, "GPIO_OUT is 0");
    // XTAL_32P (0xC0) is GPIO15 = RTC_GPIO15, bit 25.
    set(&mut rtc, 0xC0, TOUCH_MUX_SEL | RUE);
    assert_eq!(gpio.driver(15).unwrap(), PadDriver::released(Pull::Up));
}

/// A part's RTC pads go to that part's GPIO model only.
#[test]
fn a_gpio_model_takes_only_its_own_parts_rtc_pads() {
    let mut classic_gpio = Esp32Gpio::new();
    let mut s3_gpio = Esp32s3Gpio::new();
    let s3_pads = EspRtcIo::new_esp32s3().rtc_io_pads().unwrap();
    let classic_pads = EspRtcIo::new_esp32().rtc_io_pads().unwrap();
    assert!(!classic_gpio.attach_rtc_io_pads(s3_pads));
    assert!(!s3_gpio.attach_rtc_io_pads(classic_pads));
}

/// A runtime snapshot restores through the shared state: the GPIO model
/// attached before the restore sees the restored mux.
#[test]
fn runtime_snapshot_restores_through_the_shared_state() {
    let (mut rtc, mut gpio) = classic();
    set(&mut rtc, PAD_DAC1, DAC_MUX_SEL);
    rtc.write_u32(ENABLE_W1TS, 1 << 20).unwrap();
    rtc.write_u32(OUT_W1TS, 1 << 20).unwrap();
    rtc.write_u32(0x900, 0x1234).unwrap();
    let blob = rtc.runtime_snapshot();

    let mut resumed = EspRtcIo::new_esp32();
    assert!(gpio.attach_rtc_io_pads(resumed.rtc_io_pads().unwrap()));
    assert_eq!(gpio.driver(25).unwrap().out, Out::Off);
    resumed.restore_runtime_snapshot(&blob).unwrap();
    assert_eq!(gpio.driver(25).unwrap().out, Out::High);
    assert_eq!(resumed.read_u32(0x900).unwrap(), 0x1234, "the SENS tail");
    assert!(EspRtcIo::new_esp32s3()
        .restore_runtime_snapshot(&blob)
        .is_err());
}

/// Past the 0xCC-byte block, below the 0x400 the bus window keeps for it,
/// offsets read 0 and drop writes; the classic window's tail from 0x400 on
/// round-trips as the stub it replaces did.
#[test]
fn holes_read_zero_and_the_tail_round_trips() {
    let mut rtc = EspRtcIo::new_esp32();
    rtc.write_u32(0x100, 0xFFFF_FFFF).unwrap();
    assert_eq!(rtc.read_u32(0x100).unwrap(), 0);
    rtc.write_u32(0x500, 0xA5A5_5A5A).unwrap();
    assert_eq!(rtc.read_u32(0x500).unwrap(), 0xA5A5_5A5A);
    // A byte store into a write-1-to-set view carries only its own bits.
    rtc.write_u32(ENABLE, 0).unwrap();
    rtc.write(ENABLE_W1TS + 3, 0x01).unwrap();
    assert_eq!(rtc.read_u32(ENABLE).unwrap(), 1 << 24);
}

/// On a bus the pair finds itself: the RTC_IO write is bracketed, and the
/// GPIO port reports the pad it moved.
#[test]
fn the_bus_links_the_pair_and_brackets_rtc_writes() {
    use crate::bus::SystemBus;
    use crate::Bus;
    let mut bus = SystemBus::new();
    bus.add_peripheral(
        "gpio",
        0x3FF4_4000,
        0x1000,
        None,
        Box::new(Esp32Gpio::new()),
    );
    bus.add_peripheral(
        "rtcio",
        0x3FF4_8400,
        0x1000,
        None,
        Box::new(EspRtcIo::new_esp32()),
    );
    let link = bus.rtc_io_link.expect("linked");
    assert_eq!(
        (
            bus.peripherals[link.rtc].name.as_str(),
            bus.peripherals[link.gpio].name.as_str()
        ),
        ("rtcio", "gpio")
    );
    bus.write_u32(0x3FF4_8400 + PAD_DAC1, 0x8000_0000 | DAC_MUX_SEL)
        .unwrap();
    bus.write_u32(0x3FF4_8400 + ENABLE_W1TS, 1 << 20).unwrap();
    bus.write_u32(0x3FF4_8400 + OUT_W1TS, 1 << 20).unwrap();
    let gpio = bus.peripherals[link.gpio].dev.pins().unwrap();
    assert_eq!(gpio.driver(25).unwrap().out, Out::High);
    // The classic XTAL_32K pads too, with the reset value of the register.
    assert_eq!(
        bus.read_u32(0x3FF4_8400 + XTAL_32K_PAD).unwrap(),
        0x8410_0010
    );
}
