// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//! Opt-in probe of an immutable physical HAL image using existing core APIs.
//! This controlled peer is not the physical three-node station.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::CortexM;
use labwired_core::network::timed_uart::{
    cycles_to_ps, DigitalDrive, LineFormat, Parity, TimedUartNet,
};
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Cpu, Machine};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::Path;
const HZ: u64 = 80_000_000;
const ELF_SHA: &str = "0e6cfe0df8a355dcf0f7508ba9ffc46e655edd5e82a1fb31227aa45a6bde8508";
fn value(m: &Machine<CortexM>, bytes: &[u8], name: &str) -> u32 {
    let addr = labwired_loader::resolve_symbol_in_elf(bytes, name)
        .unwrap_or_else(|| panic!("missing real ELF symbol {name}"));
    m.bus.read_u32(u64::from(addr)).unwrap()
}
fn state(m: &Machine<CortexM>, bytes: &[u8]) -> serde_json::Value {
    let names = [
        "g_startup_error",
        "g_wake_events",
        "g_overload_edges",
        "g_uart_errors",
        "g_rx_overflows",
        "SystemCoreClock",
    ];
    let values: serde_json::Map<String, serde_json::Value> = names
        .iter()
        .map(|name| ((*name).into(), json!(value(m, bytes, name))))
        .collect();
    json!({"cycle":m.total_cycles,"pc":m.cpu.get_pc(),"symbols":values,"device_state":value(m,bytes,"g_device_state")&255,
        "uart_cr1":m.bus.read_u32(0x40013800).unwrap(),"uart_cr2":m.bus.read_u32(0x40013804).unwrap(),"uart_brr":m.bus.read_u32(0x4001380c).unwrap(),
        "gpioa_idr":m.bus.read_u32(0x48000010).unwrap(),"gpioc_odr":m.bus.read_u32(0x48000814).unwrap(),
        "exti_imr":m.bus.read_u32(0x40010400).unwrap(),"exti_ftsr":m.bus.read_u32(0x4001040c).unwrap(),"exti_pr":m.bus.read_u32(0x40010414).unwrap()})
}
fn save(stages: &[serde_json::Value], reply: &[u8], sha: &str) {
    let path =
        std::env::var("IOLINK_L476_DEVICE_REPORT").expect("IOLINK_L476_DEVICE_REPORT required");
    std::fs::write(path,serde_json::to_vec_pretty(&json!({
        "firmwareSha256":sha,"cpuHz":HZ,"scope":"Unchanged physical device ELF and controlled digital master line; not physical master firmware or the complete station",
        "stages":stages,"reply":reply
    })).unwrap()).unwrap();
}
fn step_to(m: &mut Machine<CortexM>, end: u64) {
    while m.total_cycles < end {
        m.step().unwrap();
    }
}
fn checksum(bytes: &[u8]) -> u8 {
    let x = bytes.iter().fold(0x52, |x, b| x ^ b);
    let b = |n: u8| (x >> n) & 1u8;
    ((b(7) ^ b(5) ^ b(3) ^ b(1)) << 5)
        | ((b(6) ^ b(4) ^ b(2) ^ b(0)) << 4)
        | ((b(7) ^ b(6)) << 3)
        | ((b(5) ^ b(4)) << 2)
        | ((b(3) ^ b(2)) << 1)
        | (b(1) ^ b(0))
}
#[test]
#[ignore = "requires immutable physical IO-Link device ELF and explicit report path"]
fn unchanged_hal_image_wakes_through_ol_exti_and_answers_a_real_type0_request() {
    let elf = std::env::var("IOLINK_L476_DEVICE_ELF").expect("IOLINK_L476_DEVICE_ELF required");
    let bytes = std::fs::read(&elf).unwrap();
    let sha = format!("{:x}", Sha256::digest(&bytes));
    assert_eq!(sha, ELF_SHA);
    let chip = ChipDescriptor::from_file(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../configs/chips/stm32l476.yaml"
    ))
    .unwrap();
    let manifest:SystemManifest=serde_yaml::from_str("name: physical-iod-probe\nchip: stm32l476\ncpu_hz: 80000000\nexternal_devices:\n- id: phy\n  type: l6362a\n  connection: uart1\n  config: {en: PC7, in1: low, in2: PA9, outiq: PA10, diag: PA4, ol: PA6}\n").unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    bus.config.peripheral_tick_interval = 1;
    let net = TimedUartNet::new();
    let (_, device, peer) = net.add_link(
        ("physical-device", "uart1", HZ),
        ("controlled-peer", "cq", HZ),
        0,
        0,
        1,
    );
    bus.attach_timed_uart_by_id("uart1", device).unwrap();
    let (cpu, _) = configure_cortex_m(&mut bus);
    let mut m = Machine::new(cpu, bus);
    m.load_firmware(&labwired_loader::load_elf(Path::new(&elf)).unwrap())
        .unwrap();
    for _ in 0..2_000_000 {
        m.step().unwrap();
    }
    let mut stages = vec![json!({"stage":"boot","state":state(&m,&bytes)})];
    save(&stages, &[], &sha);
    assert_eq!(value(&m, &bytes, "g_startup_error"), 0);
    assert_eq!(value(&m, &bytes, "SystemCoreClock"), HZ as u32);
    assert_eq!(value(&m, &bytes, "g_device_state") & 255, 0);
    let wake_start = m.total_cycles + 1;
    peer.drive_digital(wake_start, DigitalDrive::High).unwrap();
    peer.drive_digital(wake_start + 80 * 80, DigitalDrive::HighImpedance)
        .unwrap();
    step_to(&mut m, wake_start + 2000 * 80);
    stages.push(json!({"stage":"wake-release-and-guard","wakeStartCycle":wake_start,"pulseCycles":6400,"state":state(&m,&bytes)}));
    save(&stages, &[], &sha);
    assert_eq!(
        value(&m, &bytes, "g_wake_events"),
        1,
        "real EXTI handler must deliver one wake"
    );
    assert_eq!(value(&m, &bytes, "g_device_state") & 255, 1);
    assert_eq!(m.bus.read_u32(0x4001380c).unwrap(), 2083);
    assert_eq!(
        m.bus.read_u32(0x40013804).unwrap() & ((1 << 16) | (1 << 17)),
        (1 << 16) | (1 << 17)
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
    let start = m.total_cycles + 1;
    let request = [0xa2, checksum(&[0xa2, 0])];
    for (i, byte) in request.into_iter().enumerate() {
        peer.transmit(start + i as u64 * 2083 * 11, u16::from(byte), fmt);
    }
    let mut reply = Vec::new();
    let end = start + 4000 * 80;
    while m.total_cycles < end {
        m.step().unwrap();
        let (got, lost) = peer.receive_due(m.total_cycles, &fmt, false);
        assert_eq!(lost, 0);
        if let Some(got) = got {
            assert!(!got.parity_error && !got.framing_error);
            reply.push(got.value as u8);
        }
    }
    stages.push(json!({"stage":"type0-min-cycle-response","request":request,"state":state(&m,&bytes),"network":net.report(0,cycles_to_ps(m.total_cycles,HZ))}));
    save(&stages, &reply, &sha);
    assert_eq!(reply.len(), 2, "TYPE0 data plus checksum");
    assert_eq!(
        reply[0], 0x49,
        "the pinned physical station's10ms MinCycleTime"
    );
    assert_eq!(reply[1] & 0x3f, checksum(&[reply[0], reply[1] & 0xc0]));
    assert_eq!(value(&m, &bytes, "g_uart_errors"), 0);
    assert_eq!(value(&m, &bytes, "g_rx_overflows"), 0);
}
