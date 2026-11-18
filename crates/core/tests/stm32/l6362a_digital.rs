// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//! Primary-source digital truth and the existing GPIO/UART seams. These tests
//! do not establish analog current thresholds or physical propagation bounds.
use labwired_core::network::timed_uart::{
    cycles_to_ps, DigitalDrive, LineFormat, Parity, TimedUartNet,
};
use labwired_core::peripherals::components::l6362a::{L6362a, CUTOFF_PS, OL_DELAY_PS, RETRY_PS};
use labwired_core::peripherals::rs485::PinSense;
use labwired_core::peripherals::uart::{Uart, UartRegisterLayout};
use labwired_core::peripherals::uart_transceiver::DigitalUartTransceiver;
use labwired_core::{Bus, CycleClock, Peripheral};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
const HZ: u64 = 80_000_000;

#[test]
fn sourced_push_pull_and_receiver_truth_tables_are_independent_of_enable() {
    for in1 in [false, true] {
        for in2 in [false, true] {
            assert_eq!(L6362a::cq_drive(true, in1, in2).level(), Some(in1 ^ in2));
            assert_eq!(
                L6362a::cq_drive(false, in1, in2),
                DigitalDrive::HighImpedance
            );
        }
        for cq in [false, true] {
            assert_eq!(L6362a::receiver(in1, cq, true), in1 ^ cq);
            assert!(!L6362a::receiver(in1, cq, false));
            let phy = L6362a::new("off", PinSense::Const(false), in1);
            let sample = phy.sample(0, HZ, Some(false), DigitalDrive::from_level(Some(cq)));
            assert_eq!(sample.outiq, Some(in1 ^ cq));
            assert!(!sample.diag_high);
            assert!(sample.ol_high);
        }
    }
}
#[test]
fn overload_requires_known_opposed_drives_and_retains_a_real_ol_delay() {
    let phy = L6362a::new("device", PinSense::Const(true), false);
    for peer in [
        DigitalDrive::HighImpedance,
        DigitalDrive::Low,
        DigitalDrive::Unknown,
    ] {
        let s = phy.sample(0, HZ, Some(false), peer);
        assert!(s.ol_high && !s.current_limited);
    }
    let start = 1000;
    let s = phy.sample(start, HZ, Some(false), DigitalDrive::High);
    assert!(s.current_limited && s.diag_high && s.ol_high);
    assert!(
        phy.sample(start + 199, HZ, Some(false), DigitalDrive::High)
            .ol_high
    );
    assert_eq!(cycles_to_ps(200, HZ), OL_DELAY_PS);
    assert!(
        !phy.sample(start + 200, HZ, Some(false), DigitalDrive::High)
            .ol_high
    );
    let s = phy.sample(start + 201, HZ, Some(false), DigitalDrive::HighImpedance);
    assert!(s.ol_high && s.diag_high && !s.current_limited);
    // Unknown/undriven IN2 must not masquerade as an opposed logical level.
    let s = phy.sample(start + 202, HZ, None, DigitalDrive::High);
    assert_eq!(s.drive, DigitalDrive::Unknown);
    assert!(s.ol_high);
}
#[test]
fn short_opposition_does_not_become_a_wakeup_and_persistent_overload_cuts_off() {
    let phy = L6362a::new("device", PinSense::Const(true), false);
    phy.sample(0, HZ, Some(false), DigitalDrive::High);
    assert!(
        phy.sample(199, HZ, Some(false), DigitalDrive::HighImpedance)
            .ol_high
    );
    assert!(phy.sample(201, HZ, Some(false), DigitalDrive::High).ol_high);
    let end = 201 + 400_000;
    assert_eq!(cycles_to_ps(400_000, HZ), CUTOFF_PS);
    let s = phy.sample(end, HZ, Some(false), DigitalDrive::High);
    assert!(s.cut_off && !s.diag_high && s.ol_high);
    assert_eq!(s.drive, DigitalDrive::HighImpedance);
    assert!(!phy.allow_tx());
    assert_eq!(cycles_to_ps(6_400_000, HZ), RETRY_PS);
    assert!(
        phy.sample(
            end + 6_399_999,
            HZ,
            Some(false),
            DigitalDrive::HighImpedance
        )
        .cut_off
    );
    let s = phy.sample(
        end + 6_400_000,
        HZ,
        Some(false),
        DigitalDrive::HighImpedance,
    );
    assert!(!s.cut_off && s.diag_high && s.ol_high);
}
#[test]
fn thermal_shutdown_and_uvlo_do_not_generate_ol_wake() {
    let phy = L6362a::new("device", PinSense::Const(true), false);
    phy.set_faults(true, true, false);
    let s = phy.sample(0, HZ, Some(false), DigitalDrive::High);
    assert!(!s.diag_high && s.ol_high);
    assert_eq!(s.outiq, Some(true));
    assert_eq!(s.drive, DigitalDrive::HighImpedance);
    assert!(!phy.allow_tx());
    assert!(phy.accept_peer_byte());
    phy.set_faults(false, false, false);
    let s = phy.sample(1000, HZ, Some(false), DigitalDrive::High);
    assert!(!s.diag_high && s.ol_high);
    assert_eq!(s.outiq, Some(false));
    assert!(!phy.receiver_valid());
}
#[test]
fn digital_history_uses_timestamped_peer_levels_and_fails_closed_after_eviction() {
    let net = TimedUartNet::new();
    let (_, a, b) = net.add_link(("a", "u", HZ), ("b", "u", HZ), 25_000, 0, 1);
    a.set_path_delays(12_500, 0);
    a.drive_digital(10, DigitalDrive::Low).unwrap();
    a.drive_digital(20, DigitalDrive::High).unwrap();
    assert_eq!(b.peer_digital_drive(12), DigitalDrive::HighImpedance);
    assert_eq!(b.peer_digital_drive(13), DigitalDrive::Low);
    assert_eq!(b.peer_digital_drive(22), DigitalDrive::Low);
    assert_eq!(b.peer_digital_drive(23), DigitalDrive::High);
    assert!(a.drive_digital(19, DigitalDrive::Low).is_err());
    for i in 0..4200 {
        a.drive_digital(
            30 + i,
            if i % 2 == 0 {
                DigitalDrive::Low
            } else {
                DigitalDrive::High
            },
        )
        .unwrap();
    }
    assert!(a.digital_history_discarded() > 0);
    assert_eq!(b.peer_digital_drive(13), DigitalDrive::Unknown);
}
fn write32(uart: &mut Uart, offset: u64, value: u32) {
    for (i, b) in value.to_le_bytes().into_iter().enumerate() {
        uart.write(offset + i as u64, b).unwrap();
    }
}
fn read32(uart: &Uart, offset: u64) -> u32 {
    let mut b = [0; 4];
    for (i, v) in b.iter_mut().enumerate() {
        *v = uart.read(offset + i as u64).unwrap();
    }
    u32::from_le_bytes(b)
}
#[test]
fn physical_noninverting_phy_composes_with_stm32_pin_inversion() {
    let net = TimedUartNet::new();
    let (_, port, peer) = net.add_link(("device", "usart1", HZ), ("master", "cq", HZ), 0, 0, 1);
    let en = Arc::new(AtomicBool::new(true));
    let gate = L6362a::new("device", PinSense::Cell(en.clone()), false);
    let clock = CycleClock::default();
    let mut uart = Uart::new_with_layout(UartRegisterLayout::Stm32V2);
    uart.attach_cycle_clock(clock.clone());
    uart.attach_timed_port(port).unwrap();
    uart.set_digital_transceiver(Arc::new(gate.clone()))
        .unwrap();
    write32(&mut uart, 0xc, 2083);
    write32(&mut uart, 4, (1 << 16) | (1 << 17));
    write32(
        &mut uart,
        0,
        1 | (1 << 2) | (1 << 3) | (1 << 12) | (1 << 10),
    );
    let fmt = LineFormat {
        bit_ps: cycles_to_ps(2083, HZ),
        data_bits: 8,
        parity: Parity::Even,
        stop_half_bits: 2,
        rx_enabled: true,
        tx_enabled: true,
        tx_inverted: true,
        rx_inverted: true,
    };
    peer.publish_format(fmt);
    write32(&mut uart, 0x28, 0xa5);
    clock.publish(1);
    uart.tick();
    let (got, lost) = peer.receive_due(25000, &fmt, false);
    let got = got.unwrap();
    assert_eq!(lost, 0);
    assert_eq!(got.value, 0xa5);
    assert!(!got.parity_error && !got.framing_error);
    clock.publish(25000);
    uart.tick();
    assert_eq!(
        read32(&uart, 0x1c) & ((1 << 5) | 0xf),
        1 << 5,
        "L6362A receiver remains active during TX: genuine echo is preserved"
    );
    assert_eq!(uart.read(0x24).unwrap(), 0xa5);
    en.store(false, Ordering::Relaxed);
    peer.transmit(30000, 0x3c, fmt);
    clock.publish(55000);
    uart.tick();
    assert_ne!(read32(&uart, 0x1c) & (1 << 5), 0);
    assert_eq!(uart.read(0x24).unwrap(), 0x3c);
    assert_eq!(read32(&uart, 0x1c) & 3, 0);
    gate.set_faults(false, false, false);
    peer.transmit(60000, 0x5a, fmt);
    clock.publish(85000);
    uart.tick();
    assert_eq!(
        read32(&uart, 0x1c) & (1 << 5),
        0,
        "UVLO cannot deliver a clean received character"
    );
}

#[test]
fn a_real_gpio_opposition_pends_the_device_ol_exti_line() {
    use labwired_config::{ChipDescriptor, SystemManifest};
    use labwired_core::bus::SystemBus;
    let chip = ChipDescriptor::from_file(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../configs/chips/stm32l476.yaml"
    ))
    .unwrap();
    let manifest:SystemManifest=serde_yaml::from_str("name: physical-iod\nchip: stm32l476\ncpu_hz: 80000000\nexternal_devices:\n- id: phy\n  type: l6362a\n  connection: uart1\n  config: {en: PC7, in1: low, in2: PA9, outiq: PA10, diag: PA4, ol: PA6}\n").unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let net = TimedUartNet::new();
    let (_, device, master) = net.add_link(("d", "u", HZ), ("m", "u", HZ), 0, 0, 1);
    bus.attach_timed_uart_by_id("uart1", device).unwrap();
    bus.write_u32(0x4002104c, 5).unwrap(); // GPIOA/C
    bus.write_u32(0x40021060, 1).unwrap(); // SYSCFG clock
    bus.write_u32(0x48000000, 1 << 18).unwrap(); // PA9 output, SIO low
    bus.write_u32(0x48000800, 1 << 14).unwrap(); // PC7 EN output
    bus.write_u32(0x48000818, 1 << 7).unwrap();
    bus.write_u32(0x40010400, 1 << 6).unwrap(); // EXTI6 mask
    bus.write_u32(0x4001040c, 1 << 6).unwrap(); // falling OL
    bus.set_current_cycle(1);
    bus.tick_peripherals_fully_forced();
    assert_ne!(bus.read_u32(0x48000010).unwrap() & (1 << 6), 0);
    master.drive_digital(1000, DigitalDrive::High).unwrap();
    bus.set_current_cycle(1000);
    bus.tick_peripherals_fully_forced();
    assert_eq!(bus.read_u32(0x40010414).unwrap() & (1 << 6), 0);
    bus.set_current_cycle(1200);
    bus.tick_peripherals_fully_forced();
    assert_eq!(bus.read_u32(0x48000010).unwrap() & (1 << 6), 0);
    assert_ne!(
        bus.read_u32(0x48000010).unwrap() & (1 << 4),
        0,
        "healthy DIAG distinguishes wake from shutdown"
    );
    assert_ne!(bus.read_u32(0x40010414).unwrap() & (1 << 6), 0);
}
