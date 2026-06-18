#![cfg(feature = "iolink-native")]

#[test]
fn native_bridge_reports_real_master_backend() {
    assert_eq!(
        labwired_core::peripherals::components::iolink_native::backend_name(),
        "iolinki-master"
    );
}

#[test]
fn real_master_reaches_operate_with_minimal_type0_response() {
    use labwired_core::peripherals::components::iolink_native::{
        NativeIolinkMasterPort, NativeTickEvent,
    };

    let mut port = NativeIolinkMasterPort::new_type2_com3(1, 0);

    port.tick(NativeTickEvent::CycleDue, 100);
    assert_eq!(port.drain_tx(), vec![0x55]);

    port.tick(NativeTickEvent::CycleDue, 120);
    let startup = port.drain_tx();
    assert!(
        startup.len() >= 2,
        "expected real startup frame after wakeup, got {startup:02x?}"
    );

    port.feed_rx(&[0x00, 0x24]);
    port.tick(NativeTickEvent::None, 121);
    assert!(port.state_name() == "preoperate" || port.state_name() == "operate");
}
