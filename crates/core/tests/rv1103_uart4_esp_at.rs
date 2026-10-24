use labwired_core::{bus::SystemBus, Bus};
fn board() -> SystemBus {
    let mut b = SystemBus::empty();
    b.cpu_hz = 24_000_000;
    b.configure_armv7_rv1103().unwrap();
    b
}
#[test]
fn uart4_installs_atomically_at_board_address() {
    let mut b = SystemBus::new();
    let initial = b.peripherals.len();
    assert!(b.configure_armv7_rv1103_uart4().is_err());
    assert_eq!(b.peripherals.len(), initial);
    b = board();
    let n = b.peripherals.len();
    b.configure_armv7_rv1103_uart4().unwrap();
    assert_eq!(b.peripherals.len(), n + 1);
    let i = b.find_peripheral_index_by_name("uart4").unwrap();
    assert_eq!(b.peripherals[i].base, 0xff4e0000);
    assert_eq!(b.peripherals[i].size, 0x100);
    assert!(b.configure_armv7_rv1103_uart4().is_err());
    assert_eq!(b.peripherals.len(), n + 1);
}
fn basket(config: &str) -> SystemBus {
    let mut b = board();
    b.configure_armv7_rv1103_i2c3().unwrap();
    b.configure_armv7_rv1103_uart4().unwrap();
    b.register_supply_domain("external3v3", 2.7).unwrap();
    let m:labwired_config::SystemManifest=serde_yaml::from_str(&format!("name: basket\nchip: rv1103\nexternal_devices:\n - id: basket:esp32\n   type: esp32c3_esp_at\n   connection: uart4\n   config: {{{config}}}\n - id: basket:adxl\n   type: adxl345\n   connection: i2c3\n   config: {{supply_domain: external3v3, supply_min_v: 2.7}}\n")).unwrap();
    b.attach_manifest_external_devices(&m).unwrap();
    b
}
fn send(b: &mut SystemBus, cmd: &[u8]) {
    for &v in cmd {
        b.write_u32(0xff4e0000, v as u32).unwrap();
    }
}
fn drain(b: &mut SystemBus) -> Vec<u8> {
    let i = b.find_peripheral_index_by_name("uart4").unwrap();
    let u = &mut b.peripherals[i].dev;
    let mut out = vec![];
    for _ in 0..1100 {
        u.advance_attached_device_time_us(1000);
        if u.read_u32(20).unwrap() & 1 != 0 {
            out.push(u.read_u32(0).unwrap() as u8);
        }
    }
    out
}
#[test]
fn esp_commands_framing_overflow_bounds_and_real_trace() {
    let mut b = basket("");
    b.write_u32(0xff4e0008, 1).unwrap();
    for (cmd, reply) in [
        ("AT\r\n", "OK\r\n"),
        (
            "AT+GMR\r\n",
            "LabWired behavioral ESP32-C3 ESP-AT mock\r\nOK\r\n",
        ),
        (
            "AT+LABMOCKSTATUS?\r\n",
            "+LABMOCKSTATUS:DISCONNECTED\r\nOK\r\n",
        ),
        ("AT+LABMOCKCONNECT\r\n", "OK\r\n"),
        (
            "AT+LABMOCKSTATUS?\r\n",
            "+LABMOCKSTATUS:CONNECTED\r\nOK\r\n",
        ),
        ("AT+LABMOCKDISCONNECT\r\n", "OK\r\n"),
        ("AT+NOPE\r\n", "ERROR\r\n"),
        ("AT\n\r\n", "ERROR\r\n"),
    ] {
        send(&mut b, cmd.as_bytes());
        assert_eq!(drain(&mut b), reply.as_bytes());
    }
    let mut long = vec![b'x'; 257];
    long.extend_from_slice(b"\r\n");
    send(&mut b, &long);
    assert_eq!(drain(&mut b), b"ERROR\r\n");
    send(&mut b, b"AT\r\n");
    assert_eq!(drain(&mut b), b"OK\r\n");
    let i = b.find_peripheral_index_by_name("uart4").unwrap();
    assert!(b.peripherals[i].dev.bus_trace_handle().is_some());
    assert!(!b.peripherals[i].dev.logs().is_empty());
}
#[test]
fn cut_restore_discards_host_fifo_partial_command_and_mock_connection() {
    let mut b = basket("supply_domain: external3v3, supply_min_v: 2.7");
    b.write_u32(0xff4e0008, 0xc1).unwrap();
    b.write_u32(0xff4e0004, 1).unwrap();
    send(&mut b, b"AT\r\n");
    assert!(drain(&mut b).is_empty());
    b.update_supply_domain("external3v3", 3.3).unwrap();
    send(&mut b, b"AT+LABMOCKCONNECT\r\n");
    drain(&mut b);
    send(&mut b, b"AT\r\n");
    let i = b.find_peripheral_index_by_name("uart4").unwrap();
    b.peripherals[i].dev.advance_attached_device_time_us(1000);
    assert_eq!(b.peripherals[i].dev.peek(0), Some(b'O'));
    send(&mut b, b"AT+GM");
    b.update_supply_domain("external3v3", 0.0).unwrap();
    b.update_supply_domain("external3v3", 3.3).unwrap();
    assert_eq!(b.read_u32(0xff4e0014).unwrap() & 1, 0);
    send(&mut b, b"AT\r\n");
    assert_eq!(drain(&mut b), b"OK\r\n");
    send(&mut b, b"AT+LABMOCKSTATUS?\r\n");
    assert_eq!(drain(&mut b), b"+LABMOCKSTATUS:DISCONNECTED\r\nOK\r\n");
}
#[test]
fn explicit_powered_false_remains_off_on_healthy_domain() {
    let mut b = basket("supply_domain: external3v3, supply_min_v: 2.7, powered: false");
    b.update_supply_domain("external3v3", 3.3).unwrap();
    b.write_u32(0xff4e0008, 1).unwrap();
    send(&mut b, b"AT\r\n");
    assert!(drain(&mut b).is_empty());
}
#[test]
fn unknown_supply_and_peer_conflicts_are_rejected() {
    let mut b = board();
    let m:labwired_config::SystemManifest=serde_yaml::from_str("name: peer\nchip: rv1103\nexternal_devices:\n - id: basket:esp32\n   type: esp32c3_esp_at\n   connection: uart2\n   config: {supply_domain: missing, supply_min_v: 2.7}\n").unwrap();
    assert!(b.attach_manifest_external_devices(&m).is_err());
    let mut b = basket("supply_domain: external3v3, supply_min_v: 2.7");
    let m:labwired_config::SystemManifest=serde_yaml::from_str("name: peer\nchip: rv1103\nexternal_devices:\n - id: second\n   type: esp32c3_esp_at\n   connection: uart4\n").unwrap();
    assert!(b.attach_manifest_external_devices(&m).is_err());
}
fn i2c_write(b: &mut SystemBus, payload: &[u8]) {
    let mut bytes = vec![0xa6];
    bytes.extend_from_slice(payload);
    b.write_u32(0xff460000, 9).unwrap();
    let mut word = [0; 4];
    word[..bytes.len()].copy_from_slice(&bytes);
    b.write_u32(0xff460100, u32::from_le_bytes(word)).unwrap();
    b.write_u32(0xff460010, bytes.len() as u32).unwrap();
    b.write_u32(0xff460000, 17).unwrap();
}
fn i2c_read(b: &mut SystemBus, reg: u8, len: usize) -> Vec<u8> {
    b.write_u32(0xff46001c, 0x7f).unwrap();
    b.write_u32(0xff460000, 11).unwrap();
    b.write_u32(0xff460008, 0x010000a6).unwrap();
    b.write_u32(0xff46000c, 0x01000000 | reg as u32).unwrap();
    b.write_u32(0xff460014, len as u32).unwrap();
    let mut out = vec![];
    for offset in (0..len).step_by(4) {
        out.extend(
            b.read_u32(0xff460200 + offset as u64)
                .unwrap()
                .to_le_bytes(),
        );
    }
    out.truncate(len);
    b.write_u32(0xff460000, 19).unwrap();
    out
}
#[test]
fn same_domain_machine_inputs_inspection_sensor_retention_and_no_off_trace() {
    let b = basket("supply_domain: external3v3, supply_min_v: 2.7");
    let mut m = labwired_core::Machine::new(labwired_core::cpu::cortex_m::CortexM::new(), b);
    assert!(m
        .list_inputs()
        .iter()
        .any(|(id, ch)| id == "basket:esp32" && ch.key == "mock_connected"));
    m.set_input_on("basket:adxl", "x", 1.0).unwrap();
    assert_eq!(i2c_read(&mut m.bus, 0, 1), [0]);
    assert_eq!(m.bus.read_u32(0xff46001c).unwrap() & 0x40, 0x40);
    m.bus.update_supply_domain("external3v3", 3.3).unwrap();
    assert_eq!(i2c_read(&mut m.bus, 0, 1), [0xe5]);
    i2c_write(&mut m.bus, &[0x2d, 8]);
    i2c_write(&mut m.bus, &[0x31, 8]);
    assert_eq!(i2c_read(&mut m.bus, 0x32, 2), [0, 1]);
    let inspected = m.inspect(None, &Default::default());
    assert!(inspected.devices.iter().any(|d| d.id == "basket:esp32"
        && d.declared
        && d.device_type.as_deref() == Some("esp32c3_esp_at")));
    let i = m.bus.find_peripheral_index_by_name("uart4").unwrap();
    let trace = m.bus.peripherals[i].dev.bus_trace_handle().unwrap();
    let before = trace.snapshot().len();
    m.bus.update_supply_domain("external3v3", 0.0).unwrap();
    m.set_input_on("basket:adxl", "x", -0.5).unwrap();
    i2c_read(&mut m.bus, 0x32, 2);
    assert_eq!(trace.snapshot().len(), before);
    let inspected = m.inspect(None, &Default::default());
    let e = inspected
        .devices
        .iter()
        .find(|d| d.id == "basket:esp32")
        .unwrap();
    assert_eq!(e.artifacts[0].meta["powered"], false);
    m.bus.update_supply_domain("external3v3", 3.3).unwrap();
    assert_eq!(i2c_read(&mut m.bus, 0x32, 2), [0x80, 0xff]);
    m.bus.write_u32(0xff4e0008, 1).unwrap();
    send(&mut m.bus, b"AT\r\n");
    assert_eq!(drain(&mut m.bus), b"OK\r\n");
    use labwired_core::bus::bus_trace::{BusDir, BusPayload};
    let events = trace.snapshot();
    assert!(events.iter().any(|e| matches!(
        e.payload,
        BusPayload::Uart {
            direction: BusDir::Tx,
            byte: b'A'
        }
    )));
    assert!(events.iter().any(|e| matches!(
        e.payload,
        BusPayload::Uart {
            direction: BusDir::Rx,
            byte: b'O'
        }
    )));
}
#[test]
fn uart4_rejects_name_window_and_cpu_memory_conflicts() {
    use labwired_core::{
        memory::LinearMemory,
        peripherals::armv7::{dw_uart::DwUart, gic::Gic},
    };
    use std::sync::{Arc, Mutex};
    for (name, base, size) in [
        ("uart4", 0xfe000000, 0x100),
        ("overlap", 0xff4dffff, 2),
        ("overlap", 0xff4e00ff, 2),
    ] {
        let mut b = board();
        b.add_peripheral(
            name,
            base,
            size,
            None,
            Box::new(DwUart::new(Arc::new(Mutex::new(Gic::default())), 61)),
        );
        let n = b.peripherals.len();
        assert!(b.configure_armv7_rv1103_uart4().is_err());
        assert_eq!(b.peripherals.len(), n);
    }
    for region in 0..3 {
        let mut b = board();
        let mem = LinearMemory::new(2, 0xff4dffff);
        match region {
            0 => b.flash = mem,
            1 => b.ram = mem,
            _ => b.extra_mem.push(mem),
        };
        let n = b.peripherals.len();
        assert!(b.configure_armv7_rv1103_uart4().is_err());
        assert_eq!(b.peripherals.len(), n);
    }
}
#[test]
fn peer_and_inspection_peeks_do_not_consume_bytes_or_restart_timeout() {
    let mut b = basket("supply_domain: external3v3, supply_min_v: 2.7");
    b.update_supply_domain("external3v3", 3.3).unwrap();
    b.write_u32(0xff4e0008, 0xc1).unwrap();
    b.write_u32(0xff4e0004, 1).unwrap();
    send(&mut b, b"AT\r\n");
    let i = b.find_peripheral_index_by_name("uart4").unwrap();
    let u = &mut b.peripherals[i].dev;
    for _ in 0..4 {
        u.advance_attached_device_time_us(1000);
    }
    u.advance_attached_device_time_us(4000);
    for _ in 0..3 {
        assert_eq!(u.peek(0), Some(b'O'));
        assert_eq!(u.peek(8), Some(0xcc));
        u.inspect(0xff4e0000, "uart4", &Default::default());
    }
    assert_eq!(u.read_u32(8).unwrap(), 0xcc);
    assert_eq!(u.read_u32(0).unwrap(), b'O' as u32);
    assert_eq!(u.read_u32(8).unwrap(), 0xc1);
}
#[test]
fn paced_long_reply_delivers_without_fifo_overrun_under_normal_service() {
    let mut b = basket("");
    b.write_u32(0xff4e0008, 0xc1).unwrap();
    send(&mut b, b"AT+GMR\r\n");
    let i = b.find_peripheral_index_by_name("uart4").unwrap();
    let u = &mut b.peripherals[i].dev;
    let mut bytes = vec![];
    for _ in 0..70 {
        u.advance_attached_device_time_us(1000);
        let lsr = u.read_u32(20).unwrap();
        assert_eq!(lsr & 2, 0);
        if lsr & 1 != 0 {
            bytes.push(u.read_u32(0).unwrap() as u8);
        }
    }
    assert_eq!(bytes, b"LabWired behavioral ESP32-C3 ESP-AT mock\r\nOK\r\n");
}
#[test]
fn command_and_reply_memory_stays_bounded_and_saturation_does_not_change_mock_state() {
    let mut b = basket("");
    send(&mut b, &[b'x'; 256]);
    send(&mut b, b"\r");
    let i = b.find_peripheral_index_by_name("uart4").unwrap();
    let pi = b.peripherals[i]
        .dev
        .inspect(0xff4e0000, "uart4", &Default::default());
    assert!(pi.artifacts[0].meta["command_bytes"].as_u64().unwrap() <= 256);
    send(&mut b, b"\n");
    b.write_u32(0xff4e0008, 1).unwrap();
    assert_eq!(drain(&mut b), b"ERROR\r\n");
    for _ in 0..300 {
        send(&mut b, b"AT+GMR\r\n");
    }
    send(&mut b, b"AT+LABMOCKCONNECT\r\n");
    let pi = b.peripherals[i]
        .dev
        .inspect(0xff4e0000, "uart4", &Default::default());
    assert!(pi.artifacts[0].meta["reply_bytes"].as_u64().unwrap() <= 1024);
    assert_eq!(pi.artifacts[0].meta["mock_connected"], false);
    assert!(b.peripherals[i].dev.logs()[0].entries.len() <= 128);
}
#[test]
fn uart4_short_peer_reply_routes_timeout_to_gic_intid61() {
    let mut b = basket("");
    b.write_u32(0xff4e0008, 0xc1).unwrap();
    b.write_u32(0xff4e0004, 1).unwrap();
    send(&mut b, b"AT\r\n");
    let i = b.find_peripheral_index_by_name("uart4").unwrap();
    for _ in 0..4 {
        b.peripherals[i].dev.advance_attached_device_time_us(1000);
    }
    assert_eq!(b.read_u32(0xff1f1204).unwrap() & (1 << 29), 0);
    b.peripherals[i].dev.advance_attached_device_time_us(4000);
    assert_eq!(b.read_u32(0xff4e0008).unwrap(), 0xcc);
    assert_eq!(b.read_u32(0xff1f1204).unwrap() & (1 << 29), 1 << 29);
}
#[test]
fn stale_peer_epoch_peek_suppresses_discarded_overrun_without_side_effects() {
    let mut b = basket("supply_domain: external3v3, supply_min_v: 2.7");
    b.update_supply_domain("external3v3", 3.3).unwrap();
    b.write_u32(0xff4e0008, 1).unwrap();
    b.write_u32(0xff4e0004, 4).unwrap();
    send(&mut b, b"AT+GMR\r\n");
    let i = b.find_peripheral_index_by_name("uart4").unwrap();
    for _ in 0..20 {
        b.peripherals[i].dev.advance_attached_device_time_us(1000);
    }
    assert_eq!(b.peripherals[i].dev.peek(8), Some(0xc6));
    b.update_supply_domain("external3v3", 0.0).unwrap();
    b.update_supply_domain("external3v3", 3.3).unwrap();
    let u = &b.peripherals[i].dev;
    assert_eq!(u.peek(20), Some(0x60));
    assert_eq!(u.peek(8), Some(0xc1));
    assert_eq!(u.read_u32(8).unwrap(), 0xc1);
}
#[test]
fn arm_irq_sampling_reconciles_uart4_epoch_without_mmio_or_peer_poll() {
    let mut b = basket("supply_domain: external3v3, supply_min_v: 2.7");
    b.update_supply_domain("external3v3", 3.3).unwrap();
    b.write_u32(0xff4e0008, 1).unwrap();
    b.write_u32(0xff4e0004, 1).unwrap();
    send(&mut b, b"AT\r\n");
    let i = b.find_peripheral_index_by_name("uart4").unwrap();
    b.peripherals[i].dev.advance_attached_device_time_us(1000);
    assert_eq!(b.read_u32(0xff1f1204).unwrap() & (1 << 29), 1 << 29);
    b.update_supply_domain("external3v3", 0.0).unwrap();
    b.update_supply_domain("external3v3", 3.3).unwrap();
    assert!(b.arm_irq_fiq_levels().is_some());
    assert_eq!(b.read_u32(0xff1f1204).unwrap() & (1 << 29), 0);
}

struct LegacyAttachmentPeer {
    id: &'static str,
    tx: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    rx: Option<u8>,
}
impl labwired_core::peripherals::device::UartStreamDevice for LegacyAttachmentPeer {
    fn device_id(&self) -> Option<&str> {
        Some(self.id)
    }
    fn on_tx_byte(&mut self, byte: u8) {
        self.tx.lock().unwrap().push(byte);
    }
    fn poll(&mut self, _: u32) -> Option<u8> {
        self.rx.take()
    }
}
fn exclusive_attachment_remains_usable(b: &mut SystemBus) {
    let i = b.find_peripheral_index_by_name("uart4").unwrap();
    let host = b.peripherals[i].dev.as_uart_stream_host().unwrap();
    assert_eq!(host.peer_ids(), ["basket:esp32"]);
    assert!(!host.accepts_stream_attachment());
    assert!(!host.attach_console_sink(None, false));
    b.update_supply_domain("external3v3", 3.3).unwrap();
    b.write_u32(0xff4e0008, 1).unwrap();
    send(b, b"AT\r\n");
    assert_eq!(drain(b), b"OK\r\n");
}
#[test]
fn normal_attachment_bus_conflict_returns_error_and_keeps_exclusive_host_usable() {
    let mut b = basket("supply_domain: external3v3, supply_min_v: 2.7");
    let tx = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
    let result = b.attach_uart_stream_by_id(
        "uart4",
        Box::new(LegacyAttachmentPeer {
            id: "rejected",
            tx: tx.clone(),
            rx: Some(b'x'),
        }),
    );
    assert!(result.unwrap_err().to_string().contains("exclusive"));
    exclusive_attachment_remains_usable(&mut b);
    assert!(tx.lock().unwrap().is_empty());
}
#[test]
fn normal_attachment_bt201_conflict_returns_error_and_keeps_exclusive_host_usable() {
    use labwired_core::peripherals::components::{
        esp32c3_esp_at::Esp32c3EspAt, supply_domain::SupplyGate,
    };
    let mut b = board();
    b.configure_armv7_rv1103_uart4().unwrap();
    let domain = b.register_supply_domain("external3v3", 2.7).unwrap();
    let i = b.find_peripheral_index_by_name("uart4").unwrap();
    b.peripherals[i]
        .dev
        .as_uart_stream_host()
        .unwrap()
        .attach_exclusive_stream_device(Box::new(Esp32c3EspAt::new(
            "basket:esp32".into(),
            SupplyGate::routed(domain, true),
        )))
        .unwrap();
    let m: labwired_config::SystemManifest = serde_yaml::from_str(
        "name: conflict\nchip: rv1103\nexternal_devices:\n - id: rejected-bt\n   type: bt201\n   connection: uart4\n",
    )
    .unwrap();
    let result = b.attach_manifest_external_devices(&m);
    assert!(format!("{:#}", result.unwrap_err()).contains("exclusive"));
    exclusive_attachment_remains_usable(&mut b);
}
#[test]
fn normal_attachment_epoch_peer_error_preserves_console_and_empty_host() {
    use labwired_core::peripherals::components::{
        esp32c3_esp_at::Esp32c3EspAt, supply_domain::SupplyGate,
    };
    let mut b = board();
    b.configure_armv7_rv1103_uart4().unwrap();
    let domain = b.register_supply_domain("external3v3", 2.7).unwrap();
    let capture = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
    let i = b.find_peripheral_index_by_name("uart4").unwrap();
    assert!(b.peripherals[i]
        .dev
        .as_uart_stream_host()
        .unwrap()
        .attach_console_sink(Some(capture.clone()), false));
    send(&mut b, b"before");
    let result = b.attach_uart_stream_by_id(
        "uart4",
        Box::new(Esp32c3EspAt::new(
            "rejected-epoch".into(),
            SupplyGate::routed(domain, true),
        )),
    );
    assert!(result.unwrap_err().to_string().contains("exclusive"));
    let host = b.peripherals[i].dev.as_uart_stream_host().unwrap();
    assert!(host.accepts_stream_attachment());
    assert!(host.peer_ids().is_empty());
    assert!(!host.hosts_protocol_peer());
    send(&mut b, b"after");
    assert_eq!(*capture.lock().unwrap(), b"beforeafter");
    assert_eq!(b.read_u32(0xff4e0014).unwrap() & 1, 0);
}
#[test]
fn normal_attachment_legacy_multipeer_and_console_remain_supported() {
    let mut b = board();
    b.configure_armv7_rv1103_uart4().unwrap();
    b.write_u32(0xff4e0008, 1).unwrap();
    let i = b.find_peripheral_index_by_name("uart4").unwrap();
    let capture = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
    assert!(b.peripherals[i]
        .dev
        .as_uart_stream_host()
        .unwrap()
        .attach_console_sink(Some(capture.clone()), false));
    send(&mut b, b"C");
    let tx_a = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
    let tx_b = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
    for (id, tx, rx) in [("a", tx_a.clone(), b'a'), ("b", tx_b.clone(), b'b')] {
        b.attach_uart_stream_by_id(
            "uart4",
            Box::new(LegacyAttachmentPeer {
                id,
                tx,
                rx: Some(rx),
            }),
        )
        .unwrap();
    }
    let host = b.peripherals[i].dev.as_uart_stream_host().unwrap();
    assert_eq!(host.peer_ids(), ["a", "b"]);
    assert!(host.accepts_stream_attachment());
    send(&mut b, b"X");
    assert_eq!(drain(&mut b), b"ab");
    assert_eq!(*capture.lock().unwrap(), b"C"); // Normal bus attach detached capture.
    assert!(b.peripherals[i]
        .dev
        .as_uart_stream_host()
        .unwrap()
        .attach_console_sink(Some(capture.clone()), false));
    send(&mut b, b"Y");
    assert_eq!(*capture.lock().unwrap(), b"CY");
    assert_eq!(*tx_a.lock().unwrap(), b"XY");
    assert_eq!(*tx_b.lock().unwrap(), b"XY");
}

#[test]
fn machine_idle_windows_advance_peer_pacing_and_rx_timeout() {
    use labwired_core::{
        cpu::cortex_m::CortexM, memory::LinearMemory, AdvanceRequest, Cpu, Machine,
    };
    // A real WFI instruction exercises the generic Machine idle boundary.
    // This is a host boundary regression, not native Linux acceptance.
    let mut b = basket("");
    b.ram = LinearMemory::new(0x10000, 0);
    b.write_u16(0, 0xbf30).unwrap(); // WFI
    b.write_u16(2, 0xe7fe).unwrap(); // B .: bounded PC after WFI in scheduler-OFF mode.
    let mut cpu = CortexM::new();
    cpu.set_pc(0);
    cpu.set_sp(0x8000);
    let mut machine = Machine::new(cpu, b);
    machine.config.idle_fast_forward_enabled = true;
    let entered = machine.advance(AdvanceRequest::run(Some(1))).unwrap();
    assert_eq!(entered.primary_steps, 1);
    machine.bus.write_u32(0xff4e0008, 0xc1).unwrap(); // FIFO, trigger14
    machine.bus.write_u32(0xff4e0004, 1).unwrap(); // RX interrupts
    send(&mut machine.bus, b"AT\r\n");
    let before = machine.total_cycles;
    let scheduler = machine.bus.cycle_clock.scheduler_enabled();
    let mut elapsed = 0;
    // Same bounded 4096-cycle windows used by the actual basket runner.
    // 11ms covers four >=1ms peer polls and the functional4ms RX timeout.
    for _ in 0..64 {
        let advanced = machine.advance(AdvanceRequest::run(Some(4096))).unwrap();
        if scheduler {
            assert_eq!(advanced.primary_steps, 0);
            assert_eq!(advanced.idle_cycles, 4096);
            assert_eq!(advanced.elapsed_cycles, 4096);
        } else {
            assert!(advanced.primary_steps > 0);
            assert_eq!(advanced.idle_cycles, 0);
            assert!(advanced.elapsed_cycles > 0);
        }
        elapsed += advanced.elapsed_cycles;
    }
    assert_eq!(machine.total_cycles - before, elapsed);
    if scheduler {
        assert_eq!(elapsed, 64 * 4096);
    }
    let trace = machine.bus.bus_trace.snapshot();
    use labwired_core::bus::bus_trace::{BusDir, BusPayload};
    let received: Vec<_> = trace
        .iter()
        .filter_map(|e| match e.payload {
            BusPayload::Uart {
                direction: BusDir::Rx,
                byte,
            } if e.bus == "uart4" => Some((e.cycle, byte)),
            _ => None,
        })
        .collect();
    assert_eq!(
        received.iter().map(|(_, b)| *b).collect::<Vec<_>>(),
        b"OK\r\n",
        "architectural idle must not starve the same persistent UART peer clock"
    );
    assert!(received.windows(2).all(|w| w[1].0 > w[0].0));
    assert_eq!(machine.bus.read_u32(0xff4e0008).unwrap(), 0xcc);
    assert_eq!(
        machine.bus.read_u32(0xff1f1204).unwrap() & (1 << 29),
        1 << 29
    );
    let bytes: Vec<_> = (0..4)
        .map(|_| machine.bus.read_u32(0xff4e0000).unwrap() as u8)
        .collect();
    assert_eq!(bytes, b"OK\r\n");
    assert_eq!(machine.bus.read_u32(0xff4e0008).unwrap(), 0xc1);
}
