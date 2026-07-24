// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C3 RTC_SLOW self-consistency gate.
//!
//! The honesty requirement (see `peripherals/esp32c3/rtc_timer.rs`
//! `RTC_SLOW_HZ_MEASURED`): the model has ONE deterministic RTC_SLOW frequency,
//! and firmware must observe THAT rate however it measures it — no second,
//! independent pin. This test drives the two real measurement paths a firmware
//! uses and asserts they recover the same modelled constant:
//!
//!   1. The TIMG0 calibration REGISTER PROTOCOL (what IDF `rtc_clk_cal` runs):
//!      write `TIMG_RTC_CALI_MAX` slow cycles + `START`, poll `RDY`, read the
//!      counted XTAL cycles from `RTCCALICFG1`, and invert with IDF's own
//!      formula `freq = xtal_hz * slowclk_cycles / value`.
//!   2. The RTC_CNTL free-running TIME counter (what `rtc_time_get` reads):
//!      with the slow-clock rate modelled, one CPU-second of ticks must read
//!      back exactly `RTC_SLOW_HZ_MEASURED` RTC ticks.
//!
//! Before the fix these disagreed: the RTC_CNTL scale said 136_700 Hz while the
//! TIMG feature returned a hardcoded `max * 533` ratio (an unrelated ~9 kHz
//! once its C3 field layout is read correctly). Both are now derived from the
//! single `RTC_SLOW_HZ_MEASURED` constant, measured on real silicon via the
//! TIMG calibration protocol over USB-JTAG (board 9C:CC:01:D0:71:54, 2026-07-24,
//! ~148.15 kHz).

use labwired_core::peripherals::esp32::timg::{RtcCalProfile, Timg};
use labwired_core::peripherals::esp32c3::rtc_timer::{
    Esp32c3RtcTimer, C3_XTAL_HZ, CPU_HZ, RTC_SLOW_HZ_MEASURED,
};
use labwired_core::Peripheral;

// C3 TIMG0 RTCCALICFG register offsets + bit fields (TRM / IDF
// soc/esp32c3/timer_group_reg.h).
const RTCCALICFG: u64 = 0x68;
const RTCCALICFG1: u64 = 0x6C;
const CALI_START: u32 = 1 << 31;
const CALI_RDY: u32 = 1 << 15;

// C3 RTC_CNTL main-timer register offsets + bit (rtc_timer.rs).
const TIME_UPDATE: u64 = 0x0C;
const TIME_UPDATE_LATCH: u32 = 1 << 31;
const TIME_LOW: u64 = 0x10;
const TIME_HIGH: u64 = 0x14;

/// Run the TIMG0 calibration protocol for `slowclk_cycles` and recover the
/// frequency exactly as IDF's `rtc_clk_cal` does.
fn timg_calibrated_hz(timg: &mut Timg, slowclk_cycles: u32) -> u64 {
    // CLK_SEL=0 (RTC_MUX/150k), START_CYCLING=0, MAX=slowclk_cycles, START=1.
    timg.write_u32(RTCCALICFG, CALI_START | (slowclk_cycles << 16))
        .unwrap();
    assert!(
        timg.read_u32(RTCCALICFG).unwrap() & CALI_RDY != 0,
        "calibration RDY must latch"
    );
    let xtal_cycles = u64::from(timg.read_u32(RTCCALICFG1).unwrap() >> 7);
    assert!(xtal_cycles > 0, "counted XTAL cycles must be non-zero");
    C3_XTAL_HZ * u64::from(slowclk_cycles) / xtal_cycles
}

/// Measure the RTC_CNTL TIME counter over exactly one CPU-second with the
/// modelled slow-clock rate engaged (mirrors `rtc_time_get`).
fn rtc_counter_hz_over_one_cpu_second() -> u64 {
    let mut rtc = Esp32c3RtcTimer::new();
    rtc.set_slow_clock_hz(RTC_SLOW_HZ_MEASURED);
    // Legacy drive (no cycle clock attached): advance one CPU-second of cycles.
    rtc.tick_elapsed(CPU_HZ);
    rtc.write_u32(TIME_UPDATE, TIME_UPDATE_LATCH).unwrap();
    let lo = u64::from(rtc.read_u32(TIME_LOW).unwrap());
    let hi = u64::from(rtc.read_u32(TIME_HIGH).unwrap());
    lo | (hi << 32)
}

#[test]
fn timg_calibration_and_rtc_counter_agree_on_one_slow_rate() {
    let mut timg = Timg::new(0x6001_F000).with_rtc_cal(RtcCalProfile {
        xtal_hz: C3_XTAL_HZ,
        slow_hz: RTC_SLOW_HZ_MEASURED,
    });

    // Path 1: TIMG calibration protocol, several cycle counts (the reading is
    // rate-invariant, exactly as on silicon).
    for &cycles in &[100u32, 1024, 3000, 0x7FFF] {
        let cal_hz = timg_calibrated_hz(&mut timg, cycles);
        let err = cal_hz.abs_diff(RTC_SLOW_HZ_MEASURED);
        assert!(
            err <= 20,
            "cycles={cycles}: TIMG cal recovered {cal_hz} Hz, expected {RTC_SLOW_HZ_MEASURED} Hz \
             (err {err} Hz — rounding only, no hardcoded value)"
        );
    }

    // Path 2: the RTC_CNTL counter ticks at the SAME constant.
    let counter_hz = rtc_counter_hz_over_one_cpu_second();
    assert_eq!(
        counter_hz, RTC_SLOW_HZ_MEASURED,
        "RTC_CNTL counter must tick at the modelled RTC_SLOW rate"
    );

    // The weld: calibration and counter observe one rate. No second pin.
    let cal_hz = timg_calibrated_hz(&mut timg, 1024);
    assert!(
        cal_hz.abs_diff(counter_hz) <= 20,
        "TIMG cal ({cal_hz} Hz) and RTC_CNTL counter ({counter_hz} Hz) must be self-consistent"
    );
}
