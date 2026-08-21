// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! BRD2709A BLE, through the chip descriptor and across two machines.
//!
//! The controller's own behaviour is covered by the unit tests in
//! `peripherals::virtual_ble`. What those cannot prove is the wiring: that the
//! chip yaml maps the device at the address firmware uses, that the factory
//! builds it for the type name the yaml declares, and that the bus hands it the
//! core clock so an interval in milliseconds means milliseconds. Each of those
//! is a place where the model can be perfect and the board still deaf.
//!
//! ⚠️ This device is NOT silicon — see `peripherals/virtual_ble.rs`. These
//! tests assert the twin's behaviour, and the twin's only.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;
use std::path::PathBuf;

/// Where the chip yaml maps the controller.
const BLE: u64 = 0x4F00_0000;

const ID: u64 = 0x00;
const CTRL: u64 = 0x04;
const STATUS: u64 = 0x08;
const CHANNEL: u64 = 0x0C;
const ADVINTERVAL: u64 = 0x18;
const TXLEN: u64 = 0x24;
const RXCMD: u64 = 0x2C;
const RXLEN: u64 = 0x30;
const RXCHANNEL: u64 = 0x34;
const TXBUF: u64 = 0x100;
const RXBUF: u64 = 0x200;

const ADV_EN: u32 = 1 << 0;
const SCAN_EN: u32 = 1 << 1;
const RX_AVAIL: u32 = 1 << 0;

/// `"LWBL"`.
const LWBL_MAGIC: u32 = 0x4C42_574C;

/// EFR32MG26 `cpu_hz`, from the chip descriptor.
const MG26_HZ: u64 = 78_000_000;

fn root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// A pair of buses on a **private** air.
///
/// Not the process-global air the ordinary factory mints onto: `cargo test`
/// runs these tests in parallel threads of one process, so a shared air makes
/// every test hear every other test's advertisements. That is not a test
/// artefact — it is the same reason a real multi-lab worker has to hand each
/// lab its own air, which is what `attach_lab_air` exists for.
fn lab(nodes: usize) -> Vec<SystemBus> {
    let air = labwired_core::peripherals::ble_air::BleAirBus::new();
    (0..nodes)
        .map(|i| {
            let mut b = bus();
            b.attach_lab_air(
                &format!("node{i}"),
                labwired_core::peripherals::nrf52::radio::VirtualAirBus::new(),
                air.clone(),
                labwired_core::network::SimMqttFabric::new(),
            );
            b
        })
        .collect()
}

fn bus() -> SystemBus {
    let abs = root("configs/chips/efr32mg26.yaml");
    let chip = ChipDescriptor::from_file(&abs).expect("load chip descriptor");
    let manifest = SystemManifest {
        parts: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "efr32mg26-ble".to_string(),
        chip: abs.to_string_lossy().to_string(),
        external_devices: vec![],
        cosim_models: Vec::new(),
        motor_models: Vec::new(),
        board_io: vec![],
        debug_uart: None,
        wifi_ap: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
        cpu_hz: None,
    };
    SystemBus::from_config(&chip, &manifest).expect("build bus")
}

/// A legacy `ADV_NONCONN_IND` with a one-byte tag in its manufacturer data —
/// the same shape `firmware-mg26-ble` transmits.
fn beacon(tag: u8) -> Vec<u8> {
    let mut pdu = vec![0x02, 0x0C];
    pdu.extend_from_slice(&[0x02, 0x09, 0x26, 0x00, 0x27, 0x09]);
    pdu.extend_from_slice(&[0x05, 0xFF, 0xE5, 0x02, tag, 0x01]);
    pdu
}

fn stage(bus: &mut SystemBus, pdu: &[u8]) {
    for (i, chunk) in pdu.chunks(4).enumerate() {
        let mut w = 0u32;
        for (j, b) in chunk.iter().enumerate() {
            w |= (*b as u32) << (j * 8);
        }
        bus.write_u32(BLE + TXBUF + (i * 4) as u64, w).unwrap();
    }
    bus.write_u32(BLE + TXLEN, pdu.len() as u32).unwrap();
}

fn pop(bus: &mut SystemBus) -> Vec<u8> {
    bus.write_u32(BLE + RXCMD, 1).unwrap();
    let len = bus.read_u32(BLE + RXLEN).unwrap() as usize;
    (0..len)
        .map(|i| bus.read_u8(BLE + RXBUF + i as u64).unwrap())
        .collect()
}

/// Advance every peripheral on the bus by exactly `cycles`.
///
/// The bus hands `tick_elapsed` its configured `peripheral_tick_interval`, so
/// the interval IS the step size. Setting it per call keeps the advance exact
/// at any granularity — which the interval test needs, since it checks the
/// boundary one cycle either side and a rounded step would prove nothing.
fn advance(bus: &mut SystemBus, cycles: u64) {
    let mut left = cycles;
    while left > 0 {
        let step = left.min(1 << 20);
        bus.config.peripheral_tick_interval = step as u32;
        bus.tick_peripherals_fully();
        left -= step;
    }
}

#[test]
fn the_chip_maps_the_controller_where_firmware_looks_for_it() {
    let bus = bus();
    assert_eq!(
        bus.read_u32(BLE + ID).unwrap(),
        LWBL_MAGIC,
        "0x4F00_0000 must answer with the LabWired id; a wrong base address \
         reads 0 and firmware refuses to advertise"
    );
}

#[test]
fn one_board_advertises_and_another_receives_the_same_pdu() {
    let mut nodes = lab(2);
    let mut scan = nodes.pop().unwrap();
    let mut adv = nodes.pop().unwrap();

    scan.write_u32(BLE + CHANNEL, 37).unwrap();
    scan.write_u32(BLE + CTRL, SCAN_EN).unwrap();

    let pdu = beacon(0x26);
    stage(&mut adv, &pdu);
    adv.write_u32(BLE + CTRL, ADV_EN).unwrap();

    advance(&mut scan, 1);
    assert_eq!(
        scan.read_u32(BLE + STATUS).unwrap() & RX_AVAIL,
        RX_AVAIL,
        "the scanner heard nothing"
    );
    assert_eq!(pop(&mut scan), pdu, "the PDU crossed unchanged");
    assert_eq!(scan.read_u32(BLE + RXCHANNEL).unwrap(), 37);
}

/// The bus must hand the controller the descriptor's `cpu_hz`, or an interval
/// expressed in 625 µs units means the wrong wall time. With the placeholder
/// the model is constructed with (1 MHz) a 100 ms interval would come out 78×
/// short, so this is two-sided: too early fails, and so does never.
#[test]
fn the_advertising_interval_is_measured_against_the_chips_own_clock() {
    let mut nodes = lab(2);
    let mut scan = nodes.pop().unwrap();
    let mut adv = nodes.pop().unwrap();
    scan.write_u32(BLE + CHANNEL, 37).unwrap();
    scan.write_u32(BLE + CTRL, SCAN_EN).unwrap();

    stage(&mut adv, &beacon(0x01));
    adv.write_u32(BLE + ADVINTERVAL, 160).unwrap(); // 160 × 625 µs = 100 ms
    adv.write_u32(BLE + CTRL, ADV_EN).unwrap(); // one burst, on enable

    // 100 ms at 78 MHz is 7_800_000 cycles. One cycle short: still one burst.
    let period = MG26_HZ / 10;
    advance(&mut adv, period - 1);
    advance(&mut scan, 1);
    let mut heard = 0;
    while scan.read_u32(BLE + STATUS).unwrap() & RX_AVAIL != 0 {
        pop(&mut scan);
        heard += 1;
    }
    assert_eq!(
        heard, 1,
        "a second burst arrived before the interval elapsed"
    );

    // Cross the boundary: exactly one more.
    advance(&mut adv, 1);
    advance(&mut scan, 1);
    heard = 0;
    while scan.read_u32(BLE + STATUS).unwrap() & RX_AVAIL != 0 {
        pop(&mut scan);
        heard += 1;
    }
    assert_eq!(heard, 1, "the interval elapsed and no burst arrived");
}

/// Two boards can advertise and scan at once and each hears only the other —
/// the shape every "find my peer" lab has.
#[test]
fn two_boards_advertising_and_scanning_hear_each_other_and_not_themselves() {
    let mut nodes = lab(2);
    let mut b = nodes.pop().unwrap();
    let mut a = nodes.pop().unwrap();

    for (bus_ref, tag) in [(&mut a, 0xAA_u8), (&mut b, 0xBB)] {
        bus_ref.write_u32(BLE + CHANNEL, 37).unwrap();
        stage(bus_ref, &beacon(tag));
        bus_ref.write_u32(BLE + CTRL, ADV_EN | SCAN_EN).unwrap();
    }

    advance(&mut a, 1);
    advance(&mut b, 1);

    let from_b = pop(&mut a);
    let from_a = pop(&mut b);
    assert_eq!(from_b[12], 0xBB, "A must hear B's tag, not its own");
    assert_eq!(from_a[12], 0xAA, "B must hear A's tag, not its own");
    assert!(
        pop(&mut a).is_empty(),
        "A queued more than one frame, so it heard itself"
    );
}
