use crate::bus::SystemBus;
use crate::peripherals::esp32c3::wifi_mac::Esp32c3WifiMac;
use crate::Bus;
use crate::Peripheral;
use labwired_config::{ChipDescriptor, SystemManifest};
use std::path::PathBuf;

const MAC_BASE: u64 = 0x6003_3000;
const INTMATRIX: u64 = 0x600C_2000;
const LINE: u32 = 6;
/// WiFi MAC interrupt-matrix source (MAC_INTR_MAP @ offset 0).
const MAC_SOURCE: u32 = 0;

// wifi_mac register offsets (private in wifi_mac.rs; mirrored for the test).
const EVENT_GET: u64 = 0xC3C; // MAC event word (HW-set; read by the ISR)
const EVENT_CLR: u64 = 0xC40; // W1C acknowledge
const EVENT_RX_DONE: u32 = 0x0100_4000;

/// Build a devkit C3 bus (real `interrupt_core0` → INTC cache), enable C3
/// routing, swap the declarative `wifi_mac` stub for the real behavioral
/// model, and route the MAC source (0) → CPU line 6. `scheduler` selects the
/// drive mode: with the bus cycle clock attached the model is
/// scheduler-driven (walk-skipped, level exported via `matrix_irq_sources`);
/// without it the model stays on the legacy per-cycle walk.
fn setup(scheduler: bool) -> SystemBus {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip = ChipDescriptor::from_file(root.join("../../configs/chips/esp32c3.yaml"))
        .expect("load esp32c3 chip yaml");
    let manifest =
        SystemManifest::from_file(root.join("../../configs/systems/esp32c3-devkit.yaml"))
            .expect("load esp32c3-devkit system yaml");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build c3 devkit bus");
    bus.irq_fabric.esp32c3.routing = true;

    // Swap the declarative wifi_mac for the real behavioral model at its base.
    let idx = bus
        .find_peripheral_index_by_name("wifi_mac")
        .expect("devkit bus carries a wifi_mac");
    let mut dev = Esp32c3WifiMac::new();
    if scheduler {
        dev.attach_cycle_clock(bus.cycle_clock.clone());
    }
    bus.peripherals[idx].dev = Box::new(dev);
    bus.refresh_peripheral_index();

    // Route source 0 → line 6, priority 1, threshold 1, line enabled.
    bus.write_u32(INTMATRIX + MAC_SOURCE as u64 * 4, LINE)
        .unwrap();
    bus.write_u32(INTMATRIX + 0x114 + (LINE as u64) * 4, 1)
        .unwrap();
    bus.write_u32(INTMATRIX + 0x194, 1).unwrap();
    bus.write_u32(INTMATRIX + 0x104, 1 << LINE).unwrap();
    bus
}

fn mac_mut(bus: &mut SystemBus) -> &mut Esp32c3WifiMac {
    let idx = bus.find_peripheral_index_by_name("wifi_mac").unwrap();
    bus.peripherals[idx]
        .dev
        .as_any_mut()
        .unwrap()
        .downcast_mut::<Esp32c3WifiMac>()
        .unwrap()
}

/// The scheduler routing arm and the legacy walk aggregation produce the
/// SAME `irq_fabric.esp32c3.irq_lines` for the SAME pending MAC event level.
#[test]
fn scheduler_routing_matches_walk_routing_for_mac_event() {
    let mut bus = setup(true);

    // Arm the MAC event (HW normally sets it; the ISR reads it). The level
    // asserts source 0 while the event word is non-zero.
    bus.write_u32(MAC_BASE + EVENT_GET, EVENT_RX_DONE).unwrap();
    assert_eq!(
        mac_mut(&mut bus).matrix_irq_sources(),
        vec![MAC_SOURCE],
        "a MAC with a pending event must assert matrix source 0"
    );

    // Scheduler routing arm (what `apply_event_result` runs on the C3 bus).
    bus.refresh_esp32c3_sched_sources();
    bus.recompute_esp32c3_irq_lines();
    let scheduler_lines = bus.irq_fabric.esp32c3.irq_lines;
    assert_eq!(
        scheduler_lines,
        1 << LINE,
        "scheduler routing must assert the routed CPU line for source 0"
    );

    // Legacy walk routing reference: source 0 re-emitted by the walk. Must
    // land on the identical line mask.
    bus.aggregate_esp32c3_irqs(&[MAC_SOURCE]);
    assert_eq!(
        bus.irq_fabric.esp32c3.irq_lines, scheduler_lines,
        "walk aggregation and scheduler routing must produce identical esp32c3.irq_lines"
    );
}

/// A wifi_mac pinned back onto the per-cycle walk (`force_legacy_walk`)
/// re-emits the SAME matrix source from its `tick()` — the level the
/// scheduler path exports via `matrix_irq_sources` — so both drive modes
/// deliver the MAC IRQ identically.
#[test]
fn force_legacy_walk_reemits_same_source() {
    let mut bus = setup(false);
    bus.write_u32(MAC_BASE + EVENT_GET, EVENT_RX_DONE).unwrap();
    let mac = mac_mut(&mut bus);
    assert!(
        !mac.uses_scheduler(),
        "no clock attached → wifi_mac stays on the per-cycle walk"
    );
    assert_eq!(
        mac.tick().explicit_irqs,
        Some(vec![MAC_SOURCE]),
        "a walk-driven wifi_mac must re-emit its MAC level source from the walk tick"
    );
}

/// THE write-choke proof: on a fully walk-DELETED bus (no per-cycle walk to
/// re-derive the level), the `EVENT_CLR` acknowledge must de-assert the
/// routed line AT THE WRITE — otherwise the MAC level latches forever and
/// re-enters its ISR. This is the one legitimate bus addition the last
/// walker needs (`sync_esp32c3_irq_cache_write`).
#[test]
fn event_clr_deasserts_on_walk_deleted_bus_at_the_write() {
    let mut bus = setup(true);
    // Delete the walk: with wifi_mac scheduler-driven and every other model
    // inert/scheduler, the devkit bus derives walk-deletion.
    bus.legacy_walk_disabled = bus.derive_walk_deletable();
    assert!(
        bus.legacy_walk_disabled,
        "scheduler-driven wifi_mac must let the devkit bus derive walk-deletion"
    );

    // Arm the MAC level and route it at the write choke (writing EVENT_GET is
    // a MAC-window write → the choke re-derives the scheduler level).
    bus.write_u32(MAC_BASE + EVENT_GET, EVENT_RX_DONE).unwrap();
    assert_eq!(
        bus.irq_fabric.esp32c3.irq_lines,
        1 << LINE,
        "MAC event must route to the CPU line at the write, with NO walk tick"
    );

    // Acknowledge via EVENT_CLR (W1C). On a walk-deleted bus the ONLY thing
    // that can de-assert the level is the write choke re-derivation.
    bus.write_u32(MAC_BASE + EVENT_CLR, EVENT_RX_DONE).unwrap();
    assert!(
        mac_mut(&mut bus).matrix_irq_sources().is_empty(),
        "after EVENT_CLR the MAC asserts no matrix source"
    );
    assert_eq!(
        bus.irq_fabric.esp32c3.irq_lines, 0,
        "EVENT_CLR must de-assert the routed line at the write on a walk-deleted bus"
    );
}
