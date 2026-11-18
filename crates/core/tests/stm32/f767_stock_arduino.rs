//! Nucleo-F767ZI stock STM32duino startup; see the fixture source and README.
use crate::common;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::system::builder::{
    build_machine, BlobMap, BootMode, BuildOptions, BuildRequest, FirmwareSource,
};
use labwired_core::{bus::SystemBus, machine::AdvanceRequest, Bus};
use std::sync::{Arc, Mutex};

fn chip() -> ChipDescriptor {
    ChipDescriptor::from_file(common::root("configs/chips/stm32f767.yaml")).unwrap()
}

fn manifest() -> SystemManifest {
    SystemManifest::from_file(common::root("examples/nucleo-f767zi/system.yaml")).unwrap()
}

#[test]
fn f767_overdrive_crc_and_usart_registers_follow_the_device_layout() {
    let mut bus = SystemBus::from_config(&chip(), &manifest()).unwrap();
    bus.write_u32(0x4000_7000, 1 << 16).unwrap();
    assert_ne!(bus.read_u32(0x4000_7004).unwrap() & (1 << 16), 0, "ODRDY");
    bus.write_u32(0x4000_7000, (1 << 16) | (1 << 17)).unwrap();
    assert_ne!(bus.read_u32(0x4000_7004).unwrap() & (1 << 17), 0, "ODSWRDY");
    bus.write_u32(0x4000_7000, 0).unwrap();
    assert_eq!(bus.read_u32(0x4000_7004).unwrap() & (3 << 16), 0);
    assert_eq!(
        bus.read_u32(0x4002_3014).unwrap(),
        0x04c1_1db7,
        "CRC polynomial reset"
    );
    let sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);
    for (i, address) in [0x4001_1000, 0x4000_4400, 0x4000_4800, 0x4001_1400]
        .into_iter()
        .enumerate()
    {
        bus.write_u32(address, 9).unwrap(); // CR1: UE and TE
        assert_eq!(
            bus.read_u32(address).unwrap() & 9,
            9,
            "USART CR1 at {address:#x}"
        );
        assert_ne!(
            bus.read_u32(address + 0x1c).unwrap() & (1 << 7),
            0,
            "USART ISR.TXE at {address:#x}"
        );
        bus.write_u32(address + 0x28, b'A' as u32 + i as u32)
            .unwrap();
        assert_eq!(
            sink.lock().unwrap().as_slice(),
            &b"ABCD"[..=i],
            "USART TDR at {address:#x}"
        );
    }
}

#[test]
fn f767_stock_arduino_prints_and_toggles_board_led() {
    let chip = chip();
    let system = manifest();
    let firmware = common::committed("tests/fixtures/stm32f767-stock-arduino.elf");
    let blobs = BlobMap::new();
    let mut built = build_machine(BuildRequest {
        chip: &chip,
        system: &system,
        firmware: FirmwareSource::Elf(&firmware),
        boot: BootMode::FastBoot,
        blobs: &blobs,
        options: BuildOptions::default(),
    })
    .unwrap();
    let mut previous = false;
    let mut transitions = 0;
    for _ in 0..150 {
        built
            .machine
            .advance(AdvanceRequest::run(Some(100_000)))
            .unwrap();
        let state = built.machine.bus_read_u32(0x4002_0414).unwrap() & 1 != 0;
        if state != previous {
            transitions += 1;
            previous = state;
        }
    }
    let serial = String::from_utf8_lossy(&built.uart.sink.lock().unwrap()).into_owned();
    assert!(serial.matches("F767_OK").count() >= 2, "serial: {serial:?}");
    assert!(transitions >= 4, "PB0 transitions: {transitions}");
}
