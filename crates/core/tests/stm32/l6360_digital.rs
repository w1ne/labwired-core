// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//! Exact digital mode truth and the existing I2C/GPIO/UART attach paths.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::network::timed_uart::DigitalDrive;
use labwired_core::peripherals::components::l6360::L6360;
use labwired_core::peripherals::components::l6360_control::{L6360Control, L6360Mode};
use labwired_core::peripherals::device::I2cDevice;
use labwired_core::peripherals::rs485::PinSense;
use labwired_core::peripherals::uart::Uart;
use labwired_core::peripherals::uart_transceiver::DigitalUartTransceiver;
use labwired_core::Bus;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
const HZ: u64 = 80_000_000;
fn direct_write(c: &L6360Control, b: &[u8]) {
    let mut d = c.i2c_device();
    d.start();
    for &v in b {
        d.write(v);
    }
    d.stop();
}
fn configured(encq: bool) -> (L6360, Arc<AtomicBool>) {
    let c = L6360Control::new(0x60).unwrap();
    direct_write(&c, &[0x80, 0xa0, 0x20, 0, 0, 0, 0, 0x50]);
    direct_write(&c, &[0xe0, 0xa1]);
    let en = Arc::new(AtomicBool::new(encq));
    let p = L6360::new(
        "p",
        c,
        PinSense::Cell(en.clone()),
        PinSense::Const(true),
        PinSense::Const(true),
    );
    (p, en)
}
#[test]
fn table23_all_modes_and_unknown_inputs_are_not_replaced_by_push_pull() {
    use DigitalDrive::{High as H, HighImpedance as Z, Low as L, Unknown as X};
    for (mode, expected) in [
        (L6360Mode::Off, [Z, Z, Z, Z]),
        (L6360Mode::TriState, [Z, Z, Z, Z]),
        (L6360Mode::LowSide, [Z, Z, Z, L]),
        (L6360Mode::HighSide, [Z, Z, H, Z]),
        (L6360Mode::PushPull, [Z, Z, H, L]),
        (L6360Mode::LowSideOn, [L, L, L, L]),
        (L6360Mode::HighSideOn, [H, H, H, H]),
    ] {
        for (i, (en, input)) in [(false, false), (false, true), (true, false), (true, true)]
            .into_iter()
            .enumerate()
        {
            assert_eq!(L6360::requested_drive(mode, en, Some(input)), expected[i]);
        }
    }
    for mode in [L6360Mode::LowSide, L6360Mode::HighSide, L6360Mode::PushPull] {
        assert_eq!(L6360::requested_drive(mode, true, None), X);
        assert_eq!(L6360::requested_drive(mode, false, None), Z);
    }
}
#[test]
fn shared_i2c_configuration_controls_uart_direction_and_inversion() {
    let (p, en) = configured(false);
    let c = p.control();
    let s = p.sample(0, HZ, Some(false), DigitalDrive::HighImpedance);
    assert_eq!(s.drive, DigitalDrive::HighImpedance);
    assert_eq!(s.outcq, Some(true));
    assert!(s.lplus_enabled);
    assert!(p.accept_peer_byte());
    assert!(!p.allow_tx());
    en.store(true, Ordering::Relaxed);
    let s = p.sample(1, HZ, Some(false), DigitalDrive::HighImpedance);
    assert_eq!(s.drive, DigitalDrive::High);
    assert_eq!(s.outcq, Some(true));
    assert!(p.allow_tx());
    assert!(!p.accept_peer_byte());
    assert!(p.tx_inverted() && p.rx_inverted());
    direct_write(&c, &[0x80, 0x61]);
    assert_eq!(c.snapshot().mode, L6360Mode::TriState);
    assert!(!p.allow_tx());
}
#[test]
fn known_opposition_uses_programmed_cutoff_but_unknown_line_does_not() {
    let (p, _) = configured(true);
    let c = p.control();
    let unknown = p.sample(0, HZ, None, DigitalDrive::Low);
    assert_eq!(unknown.drive, DigitalDrive::Unknown);
    assert!(!unknown.opposed_drivers);
    assert_eq!(c.snapshot().status & 0x10, 0);
    let start = 80;
    let s = p.sample(start, HZ, Some(false), DigitalDrive::Low);
    assert!(s.opposed_drivers);
    assert_eq!(s.drive, DigitalDrive::High);
    assert_eq!(
        p.sample(start + 7999, HZ, Some(false), DigitalDrive::Low)
            .drive,
        DigitalDrive::High
    );
    let s = p.sample(start + 8000, HZ, Some(false), DigitalDrive::Low);
    assert_eq!(s.drive, DigitalDrive::HighImpedance);
    assert!(!s.irq_high);
    assert_eq!(c.snapshot().status & 0x10, 0x10);
    assert!(!p.allow_tx());
}
#[test]
fn static_cq_debounce_filters_short_pulses_and_serial_rejects_filtered_profile() {
    let (p, _) = configured(false);
    let c = p.control();
    // CT1 reset value 0x21 enables 5us CQ filtering; Eq2 check=0xc2.
    direct_write(&c, &[0x21, 0xc2]);
    assert!(!p.receiver_valid());
    assert!(!p.accept_peer_byte());
    assert_eq!(p.sample(0, HZ, Some(false), DigitalDrive::Low).outcq, None);
    assert_eq!(
        p.sample(400, HZ, Some(false), DigitalDrive::Low).outcq,
        Some(true)
    );
    assert_eq!(
        p.sample(500, HZ, Some(false), DigitalDrive::High).outcq,
        Some(true)
    );
    assert_eq!(
        p.sample(899, HZ, Some(false), DigitalDrive::Low).outcq,
        Some(true)
    );
    assert_eq!(
        p.sample(1000, HZ, Some(false), DigitalDrive::High).outcq,
        Some(true)
    );
    assert_eq!(
        p.sample(1400, HZ, Some(false), DigitalDrive::High).outcq,
        Some(false)
    );
    assert_eq!(
        p.sample(1401, HZ, Some(false), DigitalDrive::Unknown).outcq,
        None
    );
}
fn bus(extra: &str, second: bool) -> SystemBus {
    let chip = ChipDescriptor::from_file(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../configs/chips/stm32l476.yaml"
    ))
    .unwrap();
    let mut yaml=format!("name: physical-iom\nchip: stm32l476\ncpu_hz: 80000000\nexternal_devices:\n- id: port1\n  type: l6360\n  connection: i2c1\n  config: {{address: 96, uart: uart1, encq: PC0, enl: PA6, reset_n: PB5, incq: PA9, outcq: PA10, irq: PA4{extra}}}\n");
    if second {
        yaml.push_str("- id: port2\n  type: l6360\n  connection: i2c1\n  config: {address: 97, uart: uart4, encq: PA5, enl: PA8, reset_n: PC1, incq: PA0, outcq: PA1, irq: PB0}\n");
    }
    let manifest: SystemManifest = serde_yaml::from_str(&yaml).unwrap();
    let mut b = SystemBus::from_config(&chip, &manifest).unwrap();
    b.write_u32(0x4002104c, 7).unwrap();
    b.write_u32(0x40021058, (1 << 21) | (1 << 19)).unwrap();
    b.write_u32(0x40021060, 1 << 14).unwrap();
    b.write_u32(
        0x48000000,
        (1 << 0) | (1 << 10) | (1 << 12) | (1 << 16) | (1 << 18),
    )
    .unwrap();
    b.write_u32(0x48000400, 1 << 10).unwrap();
    b.write_u32(0x48000800, 5).unwrap();
    b.write_u32(0x48000418, 1 << 5).unwrap();
    b.write_u32(0x48000818, 1 << 1).unwrap();
    b.write_u32(0x40005400, 1).unwrap();
    tick(&mut b, 5);
    b
}
fn phy<'a>(b: &'a SystemBus, name: &str) -> &'a dyn DigitalUartTransceiver {
    let i = b.find_peripheral_index_by_name(name).unwrap();
    b.peripherals[i].dev.digital_uart_transceiver().unwrap()
}
fn observe_wire(
    b: &mut SystemBus,
    name: &str,
) -> labwired_core::network::timed_uart::TimedUartPort {
    let net = labwired_core::network::timed_uart::TimedUartNet::new();
    let (_, local, peer) = net.add_link(("master", name, HZ), ("observer", name, HZ), 0, 0, 1);
    b.attach_timed_uart_by_id(name, local).unwrap();
    peer
}
fn config_read(b: &mut SystemBus, address: u8) -> u8 {
    // DS8900 random read: register pointer write with STOP, then CONFIG..PARITY.
    i2c_write(b, address, &[1]).unwrap();
    b.write_u32(0x4000541c, 0x3f).unwrap();
    b.write_u32(
        0x40005404,
        (u32::from(address) << 1) | (8 << 16) | (1 << 10) | (1 << 25) | (1 << 13),
    )
    .unwrap();
    let mut bytes = [0u8; 8];
    for value in &mut bytes {
        wait(b, 1 << 2).unwrap();
        *value = b.read_u32(0x40005424).unwrap() as u8;
    }
    wait(b, 1 << 5).unwrap();
    bytes[0]
}
fn tick(b: &mut SystemBus, count: u64) {
    for _ in 0..count {
        b.set_current_cycle(b.current_cycle + 1);
        b.tick_peripherals_fully_forced();
    }
}
fn wait(b: &mut SystemBus, mask: u32) -> Result<(), u32> {
    for _ in 0..50_000 {
        let s = b.read_u32(0x40005418).unwrap();
        if s & 0x10 != 0 {
            return Err(s);
        }
        if s & mask != 0 {
            return Ok(());
        }
        tick(b, 1);
    }
    panic!("I2C timed out ISR={:#x}", b.read_u32(0x40005418).unwrap());
}
fn i2c_write(b: &mut SystemBus, address: u8, bytes: &[u8]) -> Result<(), u32> {
    b.write_u32(0x4000541c, 0x3f).unwrap();
    b.write_u32(
        0x40005404,
        (u32::from(address) << 1) | ((bytes.len() as u32) << 16) | (1 << 25) | (1 << 13),
    )
    .unwrap();
    for &v in bytes {
        wait(b, 2)?;
        b.write_u32(0x40005428, u32::from(v)).unwrap();
    }
    wait(b, 1 << 5)
}
#[test]
fn both_native_i2c_addresses_and_gpio_enables_remain_independent() {
    let mut b = bus("", true);
    let a = observe_wire(&mut b, "uart1");
    let z = observe_wire(&mut b, "uart4");
    assert_eq!(config_read(&mut b, 0x60) & 0xe0, 0x80);
    assert_eq!(config_read(&mut b, 0x61) & 0xe0, 0x80);
    i2c_write(&mut b, 0x60, &[0xe0, 0xa1]).unwrap();
    assert_eq!(config_read(&mut b, 0x60) & 0xe0, 0xe0);
    assert_eq!(config_read(&mut b, 0x61) & 0xe0, 0x80);
    b.write_u32(0x48000818, 1).unwrap();
    b.write_u32(0x48000018, 1 << 6).unwrap();
    tick(&mut b, 20);
    assert_eq!(a.peer_digital_drive(b.current_cycle), DigitalDrive::High);
    assert!(format!("{:?}", b.peripheral_logs("port1").unwrap()).contains("L+=true"));
    assert_eq!(
        z.peer_digital_drive(b.current_cycle),
        DigitalDrive::HighImpedance
    );
    assert!(format!("{:?}", b.peripheral_logs("port2").unwrap()).contains("L+=false"));
    i2c_write(&mut b, 0x61, &[0xe0, 0xa1]).unwrap();
    assert_eq!(config_read(&mut b, 0x61) & 0xe0, 0xe0);
    b.write_u32(0x48000418, 1 << (5 + 16)).unwrap();
    tick(&mut b, 20);
    assert_eq!(
        a.peer_digital_drive(b.current_cycle),
        DigitalDrive::HighImpedance
    );
    assert!(i2c_write(&mut b, 0x60, &[0xe0, 0xa1]).is_err());
    assert_eq!(config_read(&mut b, 0x61) & 0xe0, 0xe0);
}

#[test]
fn the_same_declared_supply_gates_i2c_and_uart_and_gpio() {
    let mut b = bus(", supply_rails: {vcc: {min_v: 18, max_v: 32.5}}", false);
    let p = observe_wire(&mut b, "uart1");
    assert!(!phy(&b, "uart1").clock_valid());
    assert!(i2c_write(&mut b, 0x60, &[0xe0, 0xa1]).is_err());
    b.apply_device_supply_voltages(&[("port1".into(), "vcc".into(), 24.)])
        .unwrap();
    tick(&mut b, 20);
    i2c_write(&mut b, 0x60, &[0xe0, 0xa1]).unwrap();
    b.write_u32(0x48000818, 1).unwrap();
    tick(&mut b, 20);
    assert!(phy(&b, "uart1").allow_tx());
    assert_eq!(p.peer_digital_drive(b.current_cycle), DigitalDrive::High);
    b.apply_device_supply_voltages(&[("port1".into(), "vcc".into(), 0.)])
        .unwrap();
    assert!(!phy(&b, "uart1").allow_tx());
    tick(&mut b, 20);
    assert_eq!(
        p.peer_digital_drive(b.current_cycle),
        DigitalDrive::HighImpedance
    );
    assert!(i2c_write(&mut b, 0x60, &[0xe0, 0xa1]).is_err());
}

#[test]
fn enabled_l6360_driver_does_not_echo_into_a_running_mcu_receiver() {
    use labwired_core::network::timed_uart::TimedUartNet;
    use labwired_core::peripherals::uart::UartRegisterLayout;
    use labwired_core::CycleClock;
    use labwired_core::Peripheral;
    for timed in [false, true] {
        let (phy, _) = configured(true);
        let clock = CycleClock::default();
        let mut uart = Uart::new_with_layout(UartRegisterLayout::Stm32V2);
        uart.attach_cycle_clock(clock.clone());
        if timed {
            let net = TimedUartNet::new();
            let (_, local, _peer) = net.add_link(("master", "u", HZ), ("device", "u", HZ), 0, 0, 1);
            uart.attach_timed_port(local).unwrap();
        }
        uart.set_digital_transceiver(Arc::new(phy)).unwrap();
        let write = |u: &mut Uart, offset: u64, value: u32| {
            for (i, b) in value.to_le_bytes().into_iter().enumerate() {
                u.write(offset + i as u64, b).unwrap();
            }
        };
        let read = |u: &Uart, offset: u64| -> u32 {
            let mut b = [0u8; 4];
            for (i, v) in b.iter_mut().enumerate() {
                *v = u.read(offset + i as u64).unwrap();
            }
            u32::from_le_bytes(b)
        };
        write(&mut uart, 0x0c, 2083);
        write(
            &mut uart,
            0,
            1 | (1 << 2) | (1 << 3) | (1 << 12) | (1 << 10),
        ); // UE, RE, TE, 8E1
        write(&mut uart, 0x28, 0xa2);
        clock.publish(1);
        uart.tick();
        clock.publish(25000);
        uart.tick();
        assert_eq!(
            read(&uart, 0x1c) & ((1 << 5) | 0xf),
            0,
            "OUTCQ is forced high while ENCQ=1; no byte or UART error (timed={timed})"
        );
    }
}
