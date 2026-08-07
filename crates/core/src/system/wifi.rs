//! Shared virtual-WiFi AP attach — the ONE source of truth every boot path uses
//! to make a diagram's `wifi-ap` component behave like a real Access Point.
//!
//! This is the "universal WiFi adapter" plumbing: drop a `wifi-ap` on any lab
//! and every *real* WiFi MAC on the bus associates to a modeled AP (802.11 →
//! DHCP → HTTP). It is universal by construction —
//!   * it operates on a bare [`SystemBus`], so the same call works from the
//!     RISC-V CLI, the browser, and any future host regardless of CPU type; and
//!   * it downcasts to every real WiFi MAC the simulator models in ONE place.
//!     Today that is the ESP32-C3 MAC (the S3 MAC is still thunked); when a
//!     second real MAC lands, teach THIS function and every host — CLI `test`,
//!     CLI `run`, the browser ctors — gets it for free, no drift.
//!
//! Absent a `wifi_ap` in the manifest this is inert, so non-WiFi boots stay
//! byte-identical to before.
//!
//! The station's eFuse MAC is NOT seeded here. It is not a property of the AP —
//! it is the die's factory identity, which every ESP32 has whether or not a
//! WiFi lab is asking. See [`crate::system::efuse`].

use crate::bus::SystemBus;
use crate::peripherals::esp32c3::virtual_wifi::{ApConfig, VirtualWifiBus};
use crate::peripherals::esp32c3::wifi_mac::Esp32c3WifiMac;
use labwired_config::SystemManifest;

/// If the manifest declares a `wifi_ap`, build a per-lab virtual-WiFi medium from
/// its config and attach every real WiFi MAC on the bus to it, keeping the MAC
/// resident so association completes. Absent a `wifi_ap` ⇒ no-op (honest "no AP
/// present": the MAC never associates).
///
/// Call this once, after the machine's bus is built and before the run loop.
pub fn attach_configured_wifi_ap(bus: &mut SystemBus, manifest: &SystemManifest) {
    let Some(ap) = manifest.wifi_ap.as_ref() else {
        return;
    };
    // Parse "a.b.c.d" → [u8;4]; parse failure falls back to the default AP IP.
    let ip = {
        let octets: Vec<u8> = ap
            .ip
            .split('.')
            .filter_map(|o| o.parse::<u8>().ok())
            .collect();
        (octets.len() == 4).then(|| [octets[0], octets[1], octets[2], octets[3]])
    };
    let cfg = ApConfig::from_parts(Some(ap.ssid.clone()), ip, Some(&ap.serves));
    let medium = VirtualWifiBus::with_config(cfg);
    let mut attached = false;
    for p in bus.peripherals.iter_mut() {
        let Some(any) = p.dev.as_any_mut() else {
            continue;
        };
        if let Some(mac) = any.downcast_mut::<Esp32c3WifiMac>() {
            mac.set_wifi_bus(medium.clone());
            mac.attach_to_medium();
            attached = true;
        }
    }
    // `attach_to_medium` flips `needs_bus_tick()` on but is a non-MMIO toggle, so
    // rebuild the tick index once to make the MAC resident (mirrors the CLI solo
    // attach loop). Only when we actually attached a MAC — a diagram may carry a
    // `wifi_ap` while the board has no real WiFi MAC (e.g. thunked S3), in which
    // case there is nothing to make resident.
    if attached {
        bus.refresh_peripheral_index();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::{PeripheralEntry, SystemBus};

    fn bus_with_c3_mac() -> SystemBus {
        let mut bus = SystemBus::new();
        bus.peripherals.push(PeripheralEntry {
            name: "wifi_mac".to_string(),
            base: 0x6000_3000,
            size: 0x1000,
            irq: None,
            dev: Box::new(Esp32c3WifiMac::new()),
            ticks_remaining: 0,
            clock_gate: None,
        });
        bus
    }

    fn mac_needs_bus_tick(bus: &mut SystemBus) -> bool {
        let idx = bus.find_peripheral_index_by_name("wifi_mac").unwrap();
        bus.peripherals[idx].dev.needs_bus_tick()
    }

    fn manifest(wifi_ap: bool) -> SystemManifest {
        let yaml = if wifi_ap {
            "name: t\nchip: esp32c3\nwifi_ap:\n  ssid: labwired-ap\n  ip: 192.168.4.1\n  serves: labwired-stats\n"
        } else {
            "name: t\nchip: esp32c3\n"
        };
        SystemManifest::from_yaml(yaml).expect("valid manifest")
    }

    #[test]
    fn attach_makes_c3_mac_resident_when_wifi_ap_present() {
        let mut bus = bus_with_c3_mac();
        // A fresh MAC is non-medium: no per-cycle bus tick required.
        assert!(!mac_needs_bus_tick(&mut bus), "fresh MAC should be idle");
        attach_configured_wifi_ap(&mut bus, &manifest(true));
        // After attach the MAC is medium-resident and must tick every cycle, or
        // association would never complete (the whole point of this fix).
        assert!(
            mac_needs_bus_tick(&mut bus),
            "attached MAC must be resident (needs_bus_tick)"
        );
    }

    #[test]
    fn attach_is_noop_without_wifi_ap() {
        let mut bus = bus_with_c3_mac();
        attach_configured_wifi_ap(&mut bus, &manifest(false));
        assert!(
            !mac_needs_bus_tick(&mut bus),
            "no wifi_ap ⇒ MAC stays idle (honest 'no AP present')"
        );
    }

    #[test]
    fn optional_password_parses_and_is_ignored_by_attach() {
        // Password is stored for people to match firmware credentials; attach
        // still succeeds (no WPA modelling). Empty/absent password remains open.
        let with_pw = SystemManifest::from_yaml(
            "name: t\nchip: esp32c3\nwifi_ap:\n  ssid: home\n  password: s3cret\n  ip: 192.168.4.1\n  serves: none\n",
        )
        .expect("valid manifest with password");
        assert_eq!(with_pw.wifi_ap.as_ref().unwrap().password, "s3cret");
        let mut bus = bus_with_c3_mac();
        attach_configured_wifi_ap(&mut bus, &with_pw);
        assert!(
            mac_needs_bus_tick(&mut bus),
            "password does not block attach"
        );

        let open = manifest(true);
        assert_eq!(open.wifi_ap.as_ref().unwrap().password, "");
    }
}
