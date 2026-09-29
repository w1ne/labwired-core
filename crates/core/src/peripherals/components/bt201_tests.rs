// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! BT201 unit tests. Expected strings are from the BT201 KT1025A/B manual
//! V2.3 (see the model doc), not from the model.

use super::*;

/// A module that is ready now: boot done, start-up output drained.
fn ready() -> Bt201 {
    let mut m = Bt201::default();
    let boot = run(&mut m, 600_000);
    assert!(boot.contains("TS+00\r\n"), "boot output: {boot:?}");
    m
}

/// Advance `us` of device time in 100 µs polls; return what the host got.
fn run(m: &mut Bt201, us: u64) -> String {
    let mut out = Vec::new();
    let mut left = us;
    while left > 0 {
        let step = left.min(100);
        left -= step;
        let mut credit = step as u32;
        while let Some(b) = m.poll(credit) {
            out.push(b);
            credit = 0;
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

fn send(m: &mut Bt201, text: &str) {
    for b in text.bytes() {
        m.on_tx_byte(b);
    }
}

/// Send one command and return the host output of the next 5 ms.
fn cmd(m: &mut Bt201, text: &str) -> String {
    send(m, text);
    run(m, 5_000)
}

fn log(m: &Bt201, name: &str) -> Vec<String> {
    m.logs()
        .into_iter()
        .find(|l| l.name == name)
        .unwrap_or_else(|| panic!("no log {name}"))
        .lines()
}

#[test]
fn start_up_block_then_link_status_after_boot_time() {
    let mut m = Bt201::default();
    assert_eq!(run(&mut m, 499_000), "", "silent while booting");
    let out = run(&mut m, 2_000);
    assert_eq!(
        out,
        "AT+VER2.3-20190517\r\nQA+30\r\nQM+00\r\nQN+01\r\nQK+01\r\nQG+01\r\nQ1+01\r\n\
         TS+00\r\nTL+02\r\n"
    );
    let link = log(&m, "link");
    assert!(link[0].ends_with("power on"), "{link:?}");
    assert!(link.iter().any(|l| l.ends_with("TL+02 ble advertising")));
}

#[test]
fn bytes_before_ready_are_ignored() {
    let mut m = Bt201::default();
    send(&mut m, "AT+TM\r\n");
    let out = run(&mut m, 600_000);
    assert!(!out.contains("TM+"), "{out:?}");
    assert!(log(&m, "at").is_empty());
}

#[test]
fn every_fb200_boot_command_is_answered() {
    let mut m = ready();
    assert_eq!(cmd(&mut m, "AT+TM\r\n"), "TM+BT201-BLE\r\n");
    assert_eq!(cmd(&mut m, "AT+BDFB200 Audio\r\n"), "OK\r\n");
    assert_eq!(cmd(&mut m, "AT+BMFB200FB200\r\n"), "OK\r\n");
    assert_eq!(cmd(&mut m, "AT+CN00\r\n"), "OK\r\n");
    assert_eq!(cmd(&mut m, "AT+B501\r\n"), "OK\r\n");
    assert_eq!(cmd(&mut m, "AT+B401\r\n"), "OK\r\n");
    assert_eq!(cmd(&mut m, "AT+B500\r\n"), "OK\r\n");
    let at = log(&m, "at");
    assert!(at[0].ends_with("AT+TM -> TM+BT201-BLE"), "{at:?}");
    assert!(at[1].ends_with("AT+BDFB200 Audio -> OK"), "{at:?}");
    assert_eq!(at.len(), 7);
}

#[test]
fn reply_comes_after_the_reply_delay() {
    let mut m = ready();
    send(&mut m, "AT+CN00\r\n");
    assert_eq!(run(&mut m, 900), "", "not before 1 ms");
    assert_eq!(run(&mut m, 200), "OK\r\n");
}

#[test]
fn unknown_command_and_bad_parameter_are_errors() {
    let mut m = ready();
    assert_eq!(cmd(&mut m, "AT+XX\r\n"), "ER+2\r\n");
    assert_eq!(cmd(&mut m, "AT+VER2.3\r\n"), "ER+2\r\n");
    assert_eq!(cmd(&mut m, "AT\r\n"), "ER+2\r\n");
    assert_eq!(cmd(&mut m, "AT+B502\r\n"), "ER+4\r\n");
    assert_eq!(cmd(&mut m, "AT+CN1\r\n"), "ER+4\r\n");
    assert_eq!(cmd(&mut m, "AT+BD\r\n"), "ER+4\r\n");
    let long = format!("AT+BD{}\r\n", "x".repeat(33));
    assert_eq!(cmd(&mut m, &long), "ER+4\r\n");
    assert_eq!(cmd(&mut m, "AT+TMX\r\n"), "ER+4\r\n");
}

#[test]
fn names_take_effect_at_reset() {
    let mut m = ready();
    cmd(&mut m, "AT+BDFB200 Audio\r\n");
    cmd(&mut m, "AT+BMFB200FB200\r\n");
    assert_eq!(
        cmd(&mut m, "AT+TM\r\n"),
        "TM+BT201-BLE\r\n",
        "old name until reset"
    );
    assert_eq!(cmd(&mut m, "AT+TD\r\n"), "TD+BT201-AUDIO\r\n");
    assert_eq!(cmd(&mut m, "AT+CZ\r\n"), "OK\r\n");
    let boot = run(&mut m, 600_000);
    assert!(boot.starts_with("AT+VER"), "{boot:?}");
    assert_eq!(cmd(&mut m, "AT+TM\r\n"), "TM+FB200FB200\r\n");
    assert_eq!(cmd(&mut m, "AT+TD\r\n"), "TD+FB200 Audio\r\n");
}

#[test]
fn ble_off_at_reset_stops_advertising_and_refuses_the_phone() {
    let mut m = ready();
    cmd(&mut m, "AT+B400\r\n");
    cmd(&mut m, "AT+CZ\r\n");
    let boot = run(&mut m, 600_000);
    assert!(!boot.contains("TL+02"), "{boot:?}");
    m.set_input("ble_link", 1.0).unwrap();
    assert!(!m.ble_connected());
    assert!(log(&m, "link")
        .iter()
        .any(|l| l.ends_with("ble link refused (not advertising)")));
    assert_eq!(cmd(&mut m, "AT+TL\r\n"), "TL+01\r\n");
}

#[test]
fn edr_link_status_is_pushed_and_repeated() {
    let mut m = ready();
    m.set_input("edr_link", 1.0).unwrap();
    assert_eq!(run(&mut m, 100), "TS+01\r\n");
    // The next repeat is due 500 ms after the ready time.
    let out = run(&mut m, 1_000_000);
    assert_eq!(out.matches("TS+01\r\n").count(), 2, "{out:?}");
    assert_eq!(cmd(&mut m, "AT+TS\r\n"), "TS+01\r\n");
    m.set_input("edr_link", 0.0).unwrap();
    assert_eq!(run(&mut m, 100), "TS+00\r\n");
}

#[test]
fn status_pushes_stop_after_cr00() {
    let mut m = ready();
    assert_eq!(cmd(&mut m, "AT+CR00\r\n"), "OK\r\n");
    m.set_input("ble_link", 1.0).unwrap();
    assert_eq!(run(&mut m, 2_000_000), "");
    assert!(m.ble_connected());
}

#[test]
fn ble_connect_disconnect_notifications() {
    let mut m = ready();
    m.set_input("ble_link", 1.0).unwrap();
    assert_eq!(run(&mut m, 100), "TL+03\r\n");
    assert_eq!(cmd(&mut m, "AT+TL\r\n"), "TL+03\r\n");
    m.set_input("ble_link", 0.0).unwrap();
    assert_eq!(run(&mut m, 100), "TL+04\r\nTL+02\r\n");
}

#[test]
fn transparent_data_both_ways_while_connected() {
    let mut m = ready();
    m.set_input("ble_link", 1.0).unwrap();
    run(&mut m, 100);
    // Phone -> MCU.
    let frame = [0xAA, 0x55, 0x01, 0x00, 0x00, 0xC8, 0xCF];
    m.inject_remote(&frame).unwrap();
    let out: Vec<u8> = run(&mut m, 100).into_bytes();
    assert_eq!(String::from_utf8_lossy(&frame).as_bytes(), out.as_slice());
    // MCU -> phone: a packet ends after the idle gap.
    for b in [0xAA, 0x55, 0x02, 0x01, 0x07] {
        m.on_tx_byte(b);
    }
    assert!(m.to_phone().is_empty(), "packet still open");
    run(&mut m, 1_100);
    assert_eq!(m.to_phone(), &[0xAA, 0x55, 0x02, 0x01, 0x07]);
    let air = log(&m, "air");
    assert!(
        air[0].ends_with("phone->mcu aa 55 01 00 00 c8 cf"),
        "{air:?}"
    );
    assert!(air[1].ends_with("mcu->phone aa 55 02 01 07"), "{air:?}");
}

#[test]
fn a_command_line_is_not_forwarded_to_the_phone() {
    let mut m = ready();
    m.set_input("ble_link", 1.0).unwrap();
    run(&mut m, 100);
    assert_eq!(cmd(&mut m, "AT+B401\r\n"), "OK\r\n");
    assert!(m.to_phone().is_empty());
    // "A" not followed by "T" is data.
    send(&mut m, "AB");
    run(&mut m, 2_000);
    assert_eq!(m.to_phone(), b"AB");
}

#[test]
fn long_data_is_split_into_128_byte_packets() {
    let mut m = ready();
    m.set_input("ble_link", 1.0).unwrap();
    run(&mut m, 100);
    for i in 0..200u32 {
        m.on_tx_byte(0x80 | (i as u8 & 0x7F));
    }
    run(&mut m, 2_000);
    assert_eq!(m.to_phone().len(), 200);
    let air = log(&m, "air");
    assert_eq!(air.len(), 2);
    assert_eq!(air[0].matches(' ').count(), 128 + 1, "time + 128 bytes");
}

#[test]
fn data_without_a_link_is_dropped() {
    let mut m = ready();
    m.inject_remote(&[0xAA, 0x55]).unwrap();
    assert_eq!(run(&mut m, 100), "", "phone data needs a link");
    send(&mut m, "\u{1}\u{2}");
    run(&mut m, 2_000);
    assert!(m.to_phone().is_empty());
    let air = log(&m, "air");
    assert!(
        air[0].ends_with("phone->mcu dropped (no ble link) aa 55"),
        "{air:?}"
    );
    assert!(
        air[1].ends_with("mcu->phone dropped (no ble link) 01 02"),
        "{air:?}"
    );
}

#[test]
fn edr_link_refused_while_booting_or_edr_off() {
    let mut m = Bt201::default();
    m.set_input("edr_link", 1.0).unwrap();
    assert!(log(&m, "link").iter().any(|l| l.contains("refused")));
    run(&mut m, 600_000);
    cmd(&mut m, "AT+B500\r\n");
    cmd(&mut m, "AT+CZ\r\n");
    let boot = run(&mut m, 600_000);
    assert!(!boot.contains("TS+"), "EDR off: no EDR status: {boot:?}");
    m.set_input("edr_link", 1.0).unwrap();
    assert_eq!(cmd(&mut m, "AT+TS\r\n"), "TS+00\r\n");
}

#[test]
fn out_of_range_input_is_rejected() {
    let mut m = ready();
    assert!(m.set_input("edr_link", 4.0).is_err());
    assert!(m.set_input("nope", 1.0).is_err());
}

#[test]
fn next_wake_points_at_the_pending_reply() {
    let mut m = ready();
    // Idle: only the EDR status repeat is scheduled.
    let idle = m.next_wake_us().expect("status repeat");
    assert!(idle <= 500_000);
    send(&mut m, "AT+TM\r\n");
    assert_eq!(m.next_wake_us(), Some(1_000));
}
