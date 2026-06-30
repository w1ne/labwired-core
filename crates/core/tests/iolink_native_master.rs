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

#[test]
fn real_master_exchanges_pd_with_one_stack_backed_device() {
    use labwired_core::peripherals::components::iolink_native::{
        NativeIolinkDevice, NativeIolinkMasterPort, NativeTickEvent,
    };

    let mut master = NativeIolinkMasterPort::new_type2_com3(1, 0);
    let mut device = NativeIolinkDevice::new_proximity(true);

    for tick in 0..600 {
        let now = tick * 10;
        master.tick(NativeTickEvent::CycleDue, now);
        let master_bytes = master.drain_tx();
        if !master_bytes.is_empty() {
            device.feed_master(&master_bytes);
        }
        let device_bytes = device.drain_tx();
        if !device_bytes.is_empty() {
            master.feed_rx(&device_bytes);
        }
        if master.state_name() == "operate" && master.latest_pd() == vec![0x01] {
            return;
        }
    }

    panic!(
        "master did not reach operate with PD=0x01; state={} pd={:02x?}",
        master.state_name(),
        master.latest_pd()
    );
}

#[test]
fn public_iolink_master_uses_native_backend_when_feature_enabled() {
    let master = labwired_core::peripherals::components::IolinkMaster::new(
        1,
        1,
        labwired_core::peripherals::components::IolinkComSpeed::Com3,
    );
    assert_eq!(master.backend_name_for_test(), "iolinki-master");
}

#[test]
fn four_port_station_reports_connected_profiles_without_sharing_state() {
    let mut station =
        labwired_core::peripherals::components::iolink_station::IolinkStation::new_4port();
    station.connect_proximity(1, true);
    station.connect_pressure(2, 6.25);
    station.connect_distance(3, 420);

    let ports = station.port_profiles();
    assert_eq!(
        ports,
        vec![
            "proximity:present",
            "pressure:6.25bar",
            "distance:420mm",
            "empty",
        ]
    );
}
