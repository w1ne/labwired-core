use crate::bus::SystemBus;
use crate::peripherals::esp32c3::ledc::{Esp32c3Ledc, LEDC_BASE, LEDC_INTR_SOURCE_ID};
use crate::Bus;
use crate::Peripheral;
use labwired_config::{ChipDescriptor, SystemManifest};
use std::path::PathBuf;

const INTMATRIX: u64 = 0x600C_2000;
const LINE: u32 = 9;

// LEDC register offsets (private in ledc.rs; mirrored for the test).
const TIMER0_CONF: u64 = 0xA0;
const INT_ENA: u64 = 0xC8;
const INT_CLR: u64 = 0xCC;
// TIMER0_CONF: DUTY_RES=4 (period 16), CLK_DIV integer part 1, running.
const TIMER0_RUN: u32 = 4 | ((1u32 << 8) << 4);
// Cycle comfortably past the first overflow (16 counts × divider 1).
const PAST_OVF: u64 = 20;

/// Build a devkit C3 bus (real `interrupt_core0` → INTC cache), enable C3
/// routing, route the LEDC source (23) → CPU line 9, and arm timer0 with its
/// OVF interrupt enabled — all through ordinary MMIO writes.
fn setup() -> SystemBus {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip = ChipDescriptor::from_file(root.join("../../configs/chips/esp32c3.yaml"))
        .expect("load esp32c3 chip yaml");
    let manifest =
        SystemManifest::from_file(root.join("../../configs/systems/esp32c3-devkit.yaml"))
            .expect("load esp32c3-devkit system yaml");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build c3 devkit bus");
    bus.irq_fabric.esp32c3.routing = true;
    bus.refresh_peripheral_index();

    // Route source 23 → line 9, priority 1, threshold 1, line enabled.
    bus.write_u32(INTMATRIX + LEDC_INTR_SOURCE_ID as u64 * 4, LINE)
        .unwrap();
    bus.write_u32(INTMATRIX + 0x114 + (LINE as u64) * 4, 1)
        .unwrap();
    bus.write_u32(INTMATRIX + 0x194, 1).unwrap();
    bus.write_u32(INTMATRIX + 0x104, 1 << LINE).unwrap();

    // Arm timer0 (period 16, divider 1) and enable its LSTIMER0_OVF int.
    bus.write_u32(LEDC_BASE as u64 + INT_ENA, 1).unwrap();
    bus.write_u32(LEDC_BASE as u64 + TIMER0_CONF, TIMER0_RUN)
        .unwrap();
    bus
}

fn ledc_mut(bus: &mut SystemBus) -> &mut Esp32c3Ledc {
    let idx = bus.find_peripheral_index_by_name("ledc").unwrap();
    bus.peripherals[idx]
        .dev
        .as_any_mut()
        .unwrap()
        .downcast_mut::<Esp32c3Ledc>()
        .unwrap()
}

/// The scheduler routing arm and the legacy walk aggregation produce the
/// SAME `irq_fabric.esp32c3.irq_lines` for the SAME LEDC overflow level.
#[test]
fn scheduler_routing_matches_walk_routing_for_overflow() {
    let mut bus = setup();

    // Advance the LEDC past its first overflow (publishes the clock so the
    // scheduler-driven model's lazy counters materialise LSTIMER0_OVF).
    bus.set_current_cycle(PAST_OVF);
    assert_eq!(
        ledc_mut(&mut bus).matrix_irq_sources(),
        vec![LEDC_INTR_SOURCE_ID],
        "an overflowed+enabled LEDC must assert matrix source 23"
    );

    // Scheduler routing arm (what `apply_event_result` runs on the C3 bus).
    bus.refresh_esp32c3_sched_sources();
    bus.recompute_esp32c3_irq_lines();
    let scheduler_lines = bus.irq_fabric.esp32c3.irq_lines;
    assert_eq!(
        scheduler_lines,
        1 << LINE,
        "scheduler routing must assert the routed CPU line for source 23"
    );

    // Legacy walk routing reference: source 23 re-emitted by the walk. Must
    // land on the identical line mask.
    bus.aggregate_esp32c3_irqs(&[LEDC_INTR_SOURCE_ID]);
    assert_eq!(
        bus.irq_fabric.esp32c3.irq_lines, scheduler_lines,
        "walk aggregation and scheduler routing must produce identical esp32c3.irq_lines"
    );
}

/// A LEDC pinned back onto the per-cycle walk (`force_legacy_walk`) re-emits
/// the SAME matrix source from its `tick()` — the level the scheduler path
/// exports via `matrix_irq_sources` — so both drive modes deliver the OVF IRQ
/// identically.
#[test]
fn force_legacy_walk_reemits_same_source() {
    let mut bus = setup();
    let ledc = ledc_mut(&mut bus);
    ledc.force_legacy_walk();
    assert!(
        !ledc.uses_scheduler(),
        "force_legacy_walk must pin LEDC back onto the per-cycle walk"
    );
    // Drive the walk past the overflow; the level source must re-emit.
    let mut irqs = None;
    for _ in 0..PAST_OVF {
        irqs = ledc.tick().explicit_irqs;
    }
    assert_eq!(
        irqs,
        Some(vec![LEDC_INTR_SOURCE_ID]),
        "a walk-pinned LEDC must re-emit its OVF level source from the walk tick"
    );
}

/// Clearing the LEDC overflow (INT_CLR) de-asserts the routed line on the
/// next re-derivation — same level semantics as the walk.
#[test]
fn clearing_overflow_deasserts_routed_line() {
    let mut bus = setup();
    bus.set_current_cycle(PAST_OVF);
    bus.refresh_esp32c3_sched_sources();
    bus.recompute_esp32c3_irq_lines();
    assert_eq!(bus.irq_fabric.esp32c3.irq_lines, 1 << LINE);

    // INT_CLR bit0 clears the LSTIMER0_OVF latch → level drops.
    bus.write_u32(LEDC_BASE as u64 + INT_CLR, 1).unwrap();
    assert!(
        ledc_mut(&mut bus).matrix_irq_sources().is_empty(),
        "after INT_CLR the LEDC asserts no matrix source"
    );
    bus.refresh_esp32c3_sched_sources();
    bus.recompute_esp32c3_irq_lines();
    assert_eq!(
        bus.irq_fabric.esp32c3.irq_lines, 0,
        "routed line must de-assert once the LEDC overflow level clears"
    );
}
