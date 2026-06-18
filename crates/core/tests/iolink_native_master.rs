#![cfg(feature = "iolink-native")]

#[test]
fn native_bridge_reports_real_master_backend() {
    assert_eq!(
        labwired_core::peripherals::components::iolink_native::backend_name(),
        "iolinki-master"
    );
}
