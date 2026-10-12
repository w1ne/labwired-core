use labwired_core::{
    peripherals::armv7::{dw_uart::DwUart, gic::Gic},
    Peripheral,
};
use std::sync::{Arc, Mutex};
fn uart() -> DwUart {
    DwUart::new(Arc::new(Mutex::new(Gic::default())), 61)
}
#[test]
fn short_reply_below_high_trigger_gets_receiver_timeout() {
    let mut u = uart();
    u.write_u32(8, 0xc1).unwrap();
    u.write_u32(4, 1).unwrap();
    u.inject_rx(b"OK\r\n");
    assert_eq!(u.read_u32(8).unwrap(), 0xc1);
    u.advance_attached_device_time_us(3999);
    assert_eq!(u.read_u32(8).unwrap(), 0xc1);
    u.advance_attached_device_time_us(1);
    assert_eq!(u.read_u32(8).unwrap(), 0xcc);
    assert_eq!(u.irq_line_level(), Some(true));
}
#[test]
fn timeout_resets_on_byte_read_clear_and_external_arrival_not_dlab_read() {
    let mut u = uart();
    u.write_u32(8, 0xc1).unwrap();
    u.write_u32(4, 1).unwrap();
    u.inject_rx(b"ab");
    u.advance_attached_device_time_us(4000);
    u.write_u32(12, 128).unwrap();
    u.read_u32(0).unwrap();
    u.write_u32(12, 0).unwrap();
    assert_eq!(u.read_u32(8).unwrap(), 0xcc);
    assert_eq!(u.read_u32(0).unwrap(), b'a' as u32);
    assert_eq!(u.read_u32(8).unwrap(), 0xc1);
    u.advance_attached_device_time_us(4000);
    u.uart_rx_source().unwrap().lock().unwrap().push_back(b'c');
    assert_eq!(u.read_u32(8).unwrap(), 0xc1);
    u.advance_attached_device_time_us(u64::MAX);
    assert_eq!(u.read_u32(8).unwrap(), 0xcc);
    u.write_u32(8, 0xc3).unwrap();
    assert_eq!(u.read_u32(8).unwrap(), 0xc1);
}
use labwired_core::bus::SystemBus;
fn board() -> SystemBus {
    let mut b = SystemBus::new();
    b.cpu_hz = 24_000_000;
    b.configure_armv7_rv1103().unwrap();
    b
}
#[test]
fn esp_at_normal_kit_attaches_and_tx_rx_is_paced() {
    let mut b = board();
    let m: labwired_config::SystemManifest=serde_yaml::from_str("name: peer\nchip: rv1103\nexternal_devices:\n - id: basket:esp32\n   type: esp32c3_esp_at\n   connection: uart2\n").unwrap();
    b.attach_manifest_external_devices(&m).unwrap();
    let i = b.find_peripheral_index_by_name("uart2").unwrap();
    let u = &mut b.peripherals[i].dev;
    u.write_u32(8, 1).unwrap();
    u.write_u32(4, 1).unwrap();
    for &v in b"AT\r\n" {
        u.write_u32(0, v as u32).unwrap();
    }
    u.advance_attached_device_time_us(999);
    assert_eq!(u.read_u32(20).unwrap() & 1, 0);
    u.advance_attached_device_time_us(1);
    assert_eq!(u.read_u32(0).unwrap(), b'O' as u32);
    u.advance_attached_device_time_us(u64::MAX);
    assert_eq!(u.read_u32(0).unwrap(), b'K' as u32);
    assert_eq!(u.read_u32(20).unwrap() & 1, 0);
}
#[test]
fn timeout_preserves_mask_priority_trigger_and_zero_elapsed() {
    let mut u = uart();
    u.write_u32(8, 0xc1).unwrap();
    u.inject_rx(b"x");
    u.advance_attached_device_time_us(0);
    assert_eq!(u.read_u32(8).unwrap(), 0xc1);
    u.advance_attached_device_time_us(4000);
    assert_eq!(u.read_u32(8).unwrap(), 0xc1);
    u.write_u32(4, 3).unwrap();
    assert_eq!(u.read_u32(8).unwrap(), 0xcc);
    u.write_u32(8, 1).unwrap();
    assert_eq!(u.read_u32(8).unwrap(), 0xc4);
    u.write_u32(8, 0xc1).unwrap();
    assert_eq!(u.read_u32(8).unwrap(), 0xcc);
    u.inject_rx(&[1; 20]);
    u.write_u32(4, 7).unwrap();
    assert_eq!(u.read_u32(8).unwrap(), 0xc6);
    u.read_u32(20).unwrap();
    assert_eq!(u.read_u32(8).unwrap(), 0xc4);
}
