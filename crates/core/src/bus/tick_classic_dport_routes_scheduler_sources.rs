use super::*;
use crate::peripherals::esp32::dport::{Dport, DPORT_PRO_MAC_INTR_MAP_REG_OFFSET};

/// UART0 — the source the classic DPORT doc names, and the one the first
/// scheduler-driven classic peripheral will assert.
const SOURCE: u32 = 34; // ETS_UART0_INTR_SOURCE
const SLOT: u8 = 9;
/// A second source on a different core, to prove the union is per-source and
/// not "whichever aggregation ran last".
const WALK_SOURCE: u32 = 13;
const WALK_SLOT: u8 = 4;

/// Asserts `SOURCE` as a SCHEDULER-driven matrix level.
///
/// `uses_scheduler()` is true and `needs_legacy_walk()` false, so the walk
/// never ticks it — the only way its source can reach `pending_cpu_irqs` is
/// the event path. That is precisely the shape `Esp32Uart` takes once it
/// migrates, which is why this test exists before that migration rather than
/// after it.
#[derive(Debug, Default)]
struct SchedLevelSource {
    /// Shared with the test rather than reached through `as_any_mut`: a
    /// downcast here would need the fixture to implement the `Any` pair, which
    /// costs a `downcast_ratchet` entry for nothing a flag cannot do.
    asserting: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Peripheral for SchedLevelSource {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }
    fn uses_scheduler(&self) -> bool {
        true
    }
    fn needs_legacy_walk(&self) -> bool {
        false
    }
    fn matrix_irq_sources_into(&self, out: &mut Vec<u32>) {
        if self.asserting.load(std::sync::atomic::Ordering::Relaxed) {
            out.push(SOURCE);
        }
    }
}

/// Walk-driven, for the union case.
#[derive(Debug)]
struct WalkLevelSource;

impl Peripheral for WalkLevelSource {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }
    fn tick(&mut self) -> crate::PeripheralTickResult {
        crate::PeripheralTickResult {
            explicit_irqs: Some(vec![WALK_SOURCE]),
            ..Default::default()
        }
    }
}

fn flag(v: bool) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
    std::sync::Arc::new(std::sync::atomic::AtomicBool::new(v))
}

fn bind(bus: &mut SystemBus, source: u32, slot: u8) {
    bus.write_u32(
        Dport::BASE as u64 + (DPORT_PRO_MAC_INTR_MAP_REG_OFFSET + source * 4) as u64,
        slot as u32,
    )
    .expect("bind DPORT PRO map entry");
}

fn dport_bus() -> SystemBus {
    let mut bus = SystemBus::empty();
    bus.add_peripheral(
        "dport",
        Dport::BASE as u64,
        0x1000,
        None,
        Box::new(Dport::new()),
    );
    bind(&mut bus, SOURCE, SLOT);
    bind(&mut bus, WALK_SOURCE, WALK_SLOT);
    bus
}

/// THE POINT OF 3a. A scheduler-driven classic peripheral's matrix source must
/// reach `pending_cpu_irqs` through DPORT.
///
/// Before this, `deliver_scheduled_irq_levels()` returned false on a classic
/// bus, so the source fell through to `pend_irq_for_event` — which routes CPU
/// exception numbers, and would have taken source 34 for exception 34.
#[test]
fn a_scheduler_driven_source_routes_through_dport() {
    let asserting = flag(true);
    let mut bus = dport_bus();
    bus.add_peripheral(
        "sched",
        0x5000_0000,
        0x100,
        None,
        Box::new(SchedLevelSource {
            asserting: asserting.clone(),
        }),
    );

    assert!(
        bus.deliver_scheduled_irq_levels(),
        "a classic bus with DPORT must CLAIM delivery, or the matrix source \
         falls through to the NVIC path"
    );
    assert_eq!(
        bus.pending_cpu_irqs,
        [1u32 << SLOT, 0],
        "the scheduler-driven source must be routed by the DPORT map"
    );
}

/// The level must FALL, not latch — the defect `aggregate_esp32s3_explicit_irqs`
/// documents for the S3 (a stale routed bit re-firing an ISR forever).
#[test]
fn a_scheduler_driven_source_clears_when_it_stops_asserting() {
    let asserting = flag(true);
    let mut bus = dport_bus();
    bus.add_peripheral(
        "sched",
        0x5000_0000,
        0x100,
        None,
        Box::new(SchedLevelSource {
            asserting: asserting.clone(),
        }),
    );
    bus.deliver_scheduled_irq_levels();
    assert_ne!(bus.pending_cpu_irqs, [0, 0], "precondition: it asserted");

    asserting.store(false, std::sync::atomic::Ordering::Relaxed);

    bus.deliver_scheduled_irq_levels();
    assert_eq!(
        bus.pending_cpu_irqs,
        [0, 0],
        "a de-asserted level must clear its routed bit, not latch it"
    );
}

/// Walk and scheduler sources must UNION, not overwrite each other.
///
/// This is the property that makes a HYBRID classic bus correct — some
/// peripherals migrated, some not — which is exactly the state the tree will
/// be in between migrating `Esp32Uart` and `Esp32I2c`.
#[test]
fn walk_and_scheduler_sources_are_unioned() {
    let asserting = flag(true);
    let mut bus = dport_bus();
    bus.add_peripheral(
        "sched",
        0x5000_0000,
        0x100,
        None,
        Box::new(SchedLevelSource {
            asserting: asserting.clone(),
        }),
    );
    bus.add_peripheral("walk", 0x5000_1000, 0x100, None, Box::new(WalkLevelSource));

    // The walk aggregation runs first, then the event choke.
    bus.tick_peripherals_fully();
    bus.deliver_scheduled_irq_levels();

    assert_eq!(
        bus.pending_cpu_irqs,
        [(1u32 << SLOT) | (1u32 << WALK_SLOT), 0],
        "both sources must survive; routing only the last aggregation to run \
         is the bug this union exists to prevent"
    );
}

/// Anti-vacuity: without the scheduler-driven peripheral the test bus routes
/// NOTHING, so the assertions above cannot be passing on some pre-existing
/// state of `pending_cpu_irqs`.
#[test]
fn the_fixture_routes_nothing_on_its_own() {
    let mut bus = dport_bus();
    bus.deliver_scheduled_irq_levels();
    assert_eq!(bus.pending_cpu_irqs, [0, 0]);
}

/// And an active matrix fabric still takes ownership — 3a must not have
/// widened the classic path into one that writes over a C3/S3.
#[test]
fn an_active_matrix_fabric_still_owns_the_routed_output() {
    let asserting = flag(true);
    let mut bus = dport_bus();
    bus.add_peripheral(
        "sched",
        0x5000_0000,
        0x100,
        None,
        Box::new(SchedLevelSource {
            asserting: asserting.clone(),
        }),
    );
    bus.irq_fabric.esp32c3.routing = true;
    bus.recompute_esp32_classic_irq_lines();
    assert_eq!(
        bus.pending_cpu_irqs,
        [0, 0],
        "the C3 fabric owns pending_cpu_irqs; classic routing must defer"
    );
}
