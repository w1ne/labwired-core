use crate::bus::SystemBus;
use crate::peripherals::esp32s3::systimer::Systimer;
use crate::Bus;
use crate::Peripheral;
use labwired_config::{ChipDescriptor, SystemManifest};
use std::path::PathBuf;

const SYSTIMER_BASE: u64 = 0x6002_3000;
const INTMATRIX: u64 = 0x600C_2000;
const SYSTIMER_TARGET0_SOURCE: u64 = 37;
const LINE: u32 = 5;

/// Build a devkit C3 bus (real `interrupt_core0` → INTC cache), enable C3
/// routing, swap the declarative SYSTIMER stub for a real scheduler-driven
/// `Systimer`, and route SYSTIMER_TARGET0 (source 37) → CPU line 5.
fn setup() -> SystemBus {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip = ChipDescriptor::from_file(root.join("../../configs/chips/esp32c3.yaml"))
        .expect("load esp32c3 chip yaml");
    let manifest =
        SystemManifest::from_file(root.join("../../configs/systems/esp32c3-devkit.yaml"))
            .expect("load esp32c3-devkit system yaml");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build c3 devkit bus");

    // Enable the RISC-V interrupt routing (the ROM-boot path sets this; the
    // from_config bus does not) and rebuild the INTC cache.
    bus.irq_fabric.esp32c3.routing = true;
    bus.refresh_peripheral_index();

    // Swap the declarative SYSTIMER stub for the real scheduler model and
    // hand it the bus clock (as `add_peripheral` would).
    let idx = bus
        .find_peripheral_index_by_name("systimer")
        .expect("devkit bus carries a systimer");
    let mut dev = Systimer::new_with_source(160_000_000, SYSTIMER_TARGET0_SOURCE as u32);
    dev.attach_cycle_clock(bus.cycle_clock.clone());
    bus.peripherals[idx].dev = Box::new(dev);
    bus.refresh_peripheral_index();

    // Route source 37 → line 5, priority 1, threshold 1, line enabled.
    bus.write_u32(INTMATRIX + SYSTIMER_TARGET0_SOURCE * 4, LINE)
        .unwrap();
    bus.write_u32(INTMATRIX + 0x114 + (LINE as u64) * 4, 1)
        .unwrap();
    bus.write_u32(INTMATRIX + 0x194, 1).unwrap();
    bus.write_u32(INTMATRIX + 0x104, 1 << LINE).unwrap();

    // Arm TARGET0 in target mode at 3 SYSTIMER ticks, enable its IRQ.
    bus.write_u32(SYSTIMER_BASE + 0x64, 1).unwrap(); // INT_ENA bit0
    bus.write_u32(SYSTIMER_BASE + 0x1C, 0).unwrap(); // TARGET0_HI
    bus.write_u32(SYSTIMER_BASE + 0x20, 3).unwrap(); // TARGET0_LO
    bus.write_u32(SYSTIMER_BASE + 0x50, 1).unwrap(); // COMP0_LOAD
    let conf = bus.read_u32(SYSTIMER_BASE).unwrap();
    bus.write_u32(SYSTIMER_BASE, conf | (1 << 24)).unwrap(); // TARGET0_WORK_EN
    bus
}

fn systimer_mut(bus: &mut SystemBus) -> &mut Systimer {
    let idx = bus.find_peripheral_index_by_name("systimer").unwrap();
    bus.peripherals[idx]
        .dev
        .as_any_mut()
        .unwrap()
        .downcast_mut::<Systimer>()
        .unwrap()
}

/// The scheduler routing arm and the legacy walk aggregation produce the
/// SAME `irq_fabric.esp32c3.irq_lines` for the SAME SYSTIMER level.
#[test]
fn scheduler_routing_matches_walk_routing_for_same_level() {
    let mut bus = setup();

    // Advance the SYSTIMER past the target so the alarm latches
    // pending && int_ena → the model asserts matrix source 37.
    systimer_mut(&mut bus).sync_to(10_000);
    assert_eq!(
        systimer_mut(&mut bus).matrix_irq_sources(),
        vec![SYSTIMER_TARGET0_SOURCE as u32],
        "armed+fired SYSTIMER must assert matrix source 37"
    );

    // Scheduler routing arm (what `apply_event_result` runs on the C3 bus).
    bus.refresh_esp32c3_sched_sources();
    bus.recompute_esp32c3_irq_lines();
    let scheduler_lines = bus.irq_fabric.esp32c3.irq_lines;
    assert_eq!(
        scheduler_lines,
        1 << LINE,
        "scheduler routing must assert the routed CPU line for source 37"
    );

    // Legacy walk routing reference: source 37 re-emitted by the walk. Must
    // land on the identical line mask.
    bus.aggregate_esp32c3_irqs(&[SYSTIMER_TARGET0_SOURCE as u32]);
    assert_eq!(
        bus.irq_fabric.esp32c3.irq_lines, scheduler_lines,
        "walk aggregation and scheduler routing must produce identical esp32c3.irq_lines"
    );
}

/// Clearing the SYSTIMER level (INT_CLR) de-asserts the routed line on the
/// next re-derivation — same level semantics as the walk (which stops
/// re-emitting the source the tick after INT_CLR).
#[test]
fn clearing_level_deasserts_routed_line() {
    let mut bus = setup();
    systimer_mut(&mut bus).sync_to(10_000);
    bus.refresh_esp32c3_sched_sources();
    bus.recompute_esp32c3_irq_lines();
    assert_eq!(bus.irq_fabric.esp32c3.irq_lines, 1 << LINE);

    // INT_CLR bit0 clears the pending latch → level drops.
    bus.write_u32(SYSTIMER_BASE + 0x6C, 1).unwrap();
    assert!(
        systimer_mut(&mut bus).matrix_irq_sources().is_empty(),
        "after INT_CLR the SYSTIMER asserts no matrix source"
    );
    bus.refresh_esp32c3_sched_sources();
    bus.recompute_esp32c3_irq_lines();
    assert_eq!(
        bus.irq_fabric.esp32c3.irq_lines, 0,
        "routed line must de-assert once the SYSTIMER level clears"
    );
}
