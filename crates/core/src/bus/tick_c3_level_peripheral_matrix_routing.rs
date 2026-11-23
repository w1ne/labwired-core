use crate::bus::SystemBus;
use crate::peripherals::esp32c3::apb_saradc::{Esp32c3ApbSarAdc, APB_SARADC_INTR_SOURCE_ID};
use crate::peripherals::esp32c3::ledc::{Esp32c3Ledc, LEDC_BASE, LEDC_INTR_SOURCE_ID};
use crate::peripherals::esp32c3::spi::{Esp32c3Spi, SPI2_INTR_SOURCE_ID, TRANS_DONE};
use crate::peripherals::esp32c3::wifi_mac::Esp32c3WifiMac;
use crate::peripherals::esp32s3::systimer::Systimer;
use crate::Bus;
use crate::Peripheral;
use labwired_config::{ChipDescriptor, SystemManifest};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

const INTMATRIX: u64 = 0x600C_2000;
const LINE: u32 = 7;

// spi2 register offsets + bits (private in spi.rs; mirrored for the test).
const SPI_CMD: u64 = 0x00;
const SPI_DMA_INT_ENA: u64 = 0x34;
const SPI_DMA_INT_CLR: u64 = 0x38;
const SPI_USR_BIT: u32 = 1 << 24;

struct ExternalCanPoller {
    polls: Arc<AtomicUsize>,
    attached: bool,
}
impl crate::peripherals::spi::SpiDevice for ExternalCanPoller {
    fn needs_external_bus_poll(&self) -> bool {
        self.attached
    }
    fn component_id(&self) -> Option<&str> {
        Some("external-can")
    }
    fn poll_external_bus(&mut self) {
        self.polls.fetch_add(1, Ordering::SeqCst);
    }
    fn attach_can_bus(
        &mut self,
        _tx: Sender<crate::network::CanFrame>,
        _rx: Receiver<crate::network::CanFrame>,
    ) -> anyhow::Result<()> {
        self.attached = true;
        Ok(())
    }
    fn transfer(&mut self, _mosi: u8) -> u8 {
        0
    }
    fn cs_pin(&self) -> &str {
        "GPIO10"
    }
}

#[test]
fn idle_nested_can_keeps_c3_spi_in_real_legacy_walk() {
    let polls = Arc::new(AtomicUsize::new(0));
    let mut bus = routed_bus(SPI2_INTR_SOURCE_ID);
    bus.attach_spi_device(
        "spi2",
        Box::new(ExternalCanPoller {
            polls: polls.clone(),
            attached: false,
        }),
    )
    .unwrap();
    pin_to_walk(&mut bus, "spi2");
    assert!(
        bus.legacy_tick_entry_descriptors()
            .iter()
            .all(|(name, _, _)| name != "spi2"),
        "unattached nested CAN must not keep SPI walk-active"
    );

    let (tx, _outbound) = std::sync::mpsc::channel();
    let (_inbound, rx) = std::sync::mpsc::channel();
    bus.attach_can_endpoint_by_id("external-can", tx, rx)
        .unwrap();
    assert!(
        bus.legacy_tick_entry_descriptors()
            .iter()
            .any(|(name, _, _)| name == "spi2"),
        "nested attach must refresh owning SPI walk eligibility"
    );
    Bus::tick_peripherals(&mut bus);
    assert_eq!(polls.load(Ordering::SeqCst), 1);

    let mut ordinary = routed_bus(SPI2_INTR_SOURCE_ID);
    pin_to_walk(&mut ordinary, "spi2");
    assert!(
        ordinary
            .legacy_tick_entry_descriptors()
            .iter()
            .all(|(name, _, _)| name != "spi2"),
        "ordinary idle C3 SPI stays outside the legacy walk"
    );
}

#[test]
fn nested_can_attach_arms_existing_c3_scheduler_controller() {
    let mut bus = routed_bus(SPI2_INTR_SOURCE_ID);
    bus.attach_spi_device(
        "spi2",
        Box::new(ExternalCanPoller {
            polls: Arc::new(AtomicUsize::new(0)),
            attached: false,
        }),
    )
    .unwrap();
    let before = bus.pending_schedule.len();

    let (tx, _outbound) = std::sync::mpsc::channel();
    let (_inbound, rx) = std::sync::mpsc::channel();
    bus.attach_can_endpoint_by_id("external-can", tx, rx)
        .unwrap();

    assert_eq!(
        bus.pending_schedule.len(),
        before + 1,
        "nested attach must arm the owning scheduler controller exactly once"
    );
}

// apb_saradc register offsets + bits (private in apb_saradc.rs; mirrored).
const SARADC_ONETIME_SAMPLE: u64 = 0x20;
const SARADC_INT_ENA: u64 = 0x40;
const SARADC_INT_CLR: u64 = 0x4C;
const SAR1_DONE_INT: u32 = 1 << 31;
const SAR1_ONETIME_SAMPLE: u32 = 1 << 31;
const ONETIME_START: u32 = 1 << 29;

/// Build a devkit C3 bus (real `interrupt_core0` → INTC cache), enable C3
/// routing, and route `source` → CPU line 7 (priority 1, threshold 1,
/// enabled) — the same wiring the SYSTIMER routing gate uses.
fn routed_bus(source: u32) -> SystemBus {
    let mut bus = devkit_bus();
    bus.refresh_peripheral_index();
    route_source(&mut bus, source, LINE);
    bus
}

/// A devkit C3 bus (real `interrupt_core0` → INTC cache) with C3 routing
/// enabled. The ROM-boot path sets `routing`; `from_config` does not.
fn devkit_bus() -> SystemBus {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip = ChipDescriptor::from_file(root.join("../../configs/chips/esp32c3.yaml"))
        .expect("load esp32c3 chip yaml");
    let manifest =
        SystemManifest::from_file(root.join("../../configs/systems/esp32c3-devkit.yaml"))
            .expect("load esp32c3-devkit system yaml");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build c3 devkit bus");
    bus.irq_fabric.esp32c3.routing = true;
    bus
}

/// Route matrix `source` → CPU `line`, priority 1, threshold 1, line enabled.
fn route_source(bus: &mut SystemBus, source: u32, line: u32) {
    bus.write_u32(INTMATRIX + (source as u64) * 4, line)
        .unwrap();
    bus.write_u32(INTMATRIX + 0x114 + (line as u64) * 4, 1)
        .unwrap();
    bus.write_u32(INTMATRIX + 0x194, 1).unwrap();
    bus.write_u32(INTMATRIX + 0x104, 1 << line).unwrap();
}

fn idx(bus: &SystemBus, name: &str) -> usize {
    bus.find_peripheral_index_by_name(name)
        .unwrap_or_else(|| panic!("c3 devkit bus carries {name}"))
}

/// Drive the level directly on the peripheral's own register write path (the
/// same effect an MMIO write has on the device), bypassing any bus-side
/// clock gate so the test is about the routing, not RCC bring-up. Refreshes
/// the legacy walk tick-set membership afterwards exactly as the real MMIO
/// write choke (`SystemBus::write_u32`) does, so a walk-pinned peripheral
/// that just asserted its level is actually ticked.
fn write_dev(bus: &mut SystemBus, name: &str, off: u64, val: u32) {
    let i = idx(bus, name);
    bus.peripherals[i].dev.write_u32(off, val).unwrap();
    bus.refresh_legacy_tick_index(i);
}

fn arm_spi2(bus: &mut SystemBus) {
    write_dev(bus, "spi2", SPI_DMA_INT_ENA, TRANS_DONE); // enable TRANS_DONE
    write_dev(bus, "spi2", SPI_CMD, SPI_USR_BIT); // launch → latches TRANS_DONE
}

fn arm_saradc(bus: &mut SystemBus) {
    write_dev(bus, "apb_saradc", SARADC_INT_ENA, SAR1_DONE_INT); // enable
    write_dev(
        bus,
        "apb_saradc",
        SARADC_ONETIME_SAMPLE,
        SAR1_ONETIME_SAMPLE | ONETIME_START | (3 << 25), // one-shot ch3 → DONE
    );
}

fn pin_to_walk(bus: &mut SystemBus, name: &str) {
    let i = idx(bus, name);
    let dev = bus.peripherals[i].dev.as_any_mut().unwrap();
    if let Some(spi) = dev.downcast_mut::<Esp32c3Spi>() {
        spi.force_legacy_walk();
    } else if let Some(adc) = dev.downcast_mut::<Esp32c3ApbSarAdc>() {
        adc.force_legacy_walk();
    } else {
        panic!("{name} is not a known C3 level peripheral");
    }
    // Flipping uses_scheduler false changes walk-set membership; refresh it
    // so an already-armed peripheral joins the walk (the arm's own refresh
    // ran while it was still scheduler-driven and thus excluded).
    // Re-derive walk-deletion: once every C3 timer/level model migrated off
    // the walk (the LEDC timer port emptied the last real pinner on this
    // no-wifi_mac devkit bus), `from_config` builds the bus walk-DELETED, so
    // the per-cycle walk is globally skipped. Pinning a peripheral back onto
    // the walk (`force_legacy_walk` → `needs_legacy_walk() == true`) makes it
    // no longer deletable; recompute the flag so the walk path this gate
    // exercises actually runs.
    bus.legacy_walk_disabled = bus.derive_walk_deletable();
    bus.refresh_legacy_tick_index(i);
}

/// Shared body: arm `name` on a scheduler bus and a walk bus, tick each, and
/// assert the routed CPU line is delivered identically; then clear the level
/// and assert both de-assert. `source` is the peripheral's matrix source ID,
/// `int_clr`/`clr_bit` the W1C acknowledge.
fn assert_walk_scheduler_identical(
    name: &str,
    source: u32,
    int_clr: u64,
    clr_bit: u32,
    arm: fn(&mut SystemBus),
) {
    let mut sched = routed_bus(source);
    let mut walk = routed_bus(source);
    arm(&mut sched);
    arm(&mut walk);
    pin_to_walk(&mut walk, name);

    // Scheduler bus: the peripheral is walk-skipped and exports its level.
    let si = idx(&sched, name);
    assert!(
        sched.peripherals[si].dev.uses_scheduler(),
        "{name} must be scheduler-driven once the bus attached its clock"
    );
    assert_eq!(
        sched.peripherals[si].dev.matrix_irq_sources(),
        vec![source],
        "{name} must export its matrix source while armed+enabled"
    );
    // Walk bus: pinned back, so it re-emits via the walk (`tick`), not the
    // scheduler export. (`matrix_irq_sources` still reflects the raw level,
    // but the bus never polls it for a non-`uses_scheduler` peripheral.)
    let wi = idx(&walk, name);
    assert!(
        !walk.peripherals[wi].dev.uses_scheduler(),
        "force_legacy_walk must pin {name} back onto the per-cycle walk"
    );
    assert_eq!(
        walk.peripherals[wi].dev.tick().explicit_irqs,
        Some(vec![source]),
        "a walk-pinned {name} must re-emit its level source from the walk tick"
    );

    // One walk tick on each aggregates the routed line mask.
    sched.tick_peripherals_with_costs();
    walk.tick_peripherals_with_costs();
    assert_eq!(
        sched.irq_fabric.esp32c3.irq_lines,
        1 << LINE,
        "scheduler path must route {name} source {source} to CPU line {LINE}"
    );
    assert_eq!(
        walk.irq_fabric.esp32c3.irq_lines,
        1 << LINE,
        "walk path must route {name} source {source} to CPU line {LINE}"
    );
    assert_eq!(
        sched.irq_fabric.esp32c3.irq_lines, walk.irq_fabric.esp32c3.irq_lines,
        "walk vs scheduler IRQ delivery for {name} must be byte-identical"
    );

    // Acknowledge the level on both; the routed line must de-assert on the
    // next tick — same level semantics on either path.
    write_dev(&mut sched, name, int_clr, clr_bit);
    write_dev(&mut walk, name, int_clr, clr_bit);
    sched.tick_peripherals_with_costs();
    walk.tick_peripherals_with_costs();
    assert_eq!(
        sched.irq_fabric.esp32c3.irq_lines, 0,
        "scheduler path must de-assert {name} after INT_CLR"
    );
    assert_eq!(
        walk.irq_fabric.esp32c3.irq_lines, 0,
        "walk path must de-assert {name} after INT_CLR"
    );
}

#[test]
fn spi2_irq_delivered_identically_walk_and_scheduler() {
    assert_walk_scheduler_identical(
        "spi2",
        SPI2_INTR_SOURCE_ID,
        SPI_DMA_INT_CLR,
        TRANS_DONE,
        arm_spi2,
    );
}

#[test]
fn apb_saradc_irq_delivered_identically_walk_and_scheduler() {
    assert_walk_scheduler_identical(
        "apb_saradc",
        APB_SARADC_INTR_SOURCE_ID,
        SARADC_INT_CLR,
        SAR1_DONE_INT,
        arm_saradc,
    );
}

// ---------------------------------------------------------------------------
// Walk-free C3 SYSTIMER / LEDC / WiFi-MAC batches — the same routing identity
// for peripherals whose level is not write-armed through one INT_ENA/launch
// pair. They differ only in how the bus is prepared, how the level is raised
// and how it is cleared, so each is a row of `ROUTED` and each contract is one
// test looping every row it applies to.
// ---------------------------------------------------------------------------

const SYSTIMER_BASE: u64 = 0x6002_3000;
const SYSTIMER_TARGET0_SOURCE: u32 = 37;

// LEDC register offsets (private in ledc.rs; mirrored for the test).
const LEDC_TIMER0_CONF: u64 = 0xA0;
const LEDC_INT_ENA: u64 = 0xC8;
const LEDC_INT_CLR: u64 = 0xCC;
// TIMER0_CONF: DUTY_RES=4 (period 16), CLK_DIV integer part 1, running.
const LEDC_TIMER0_RUN: u32 = 4 | ((1u32 << 8) << 4);
// Cycle comfortably past the first overflow (16 counts × divider 1).
const LEDC_PAST_OVF: u64 = 20;

const MAC_BASE: u64 = 0x6003_3000;
/// WiFi MAC interrupt-matrix source (MAC_INTR_MAP @ offset 0).
const MAC_SOURCE: u32 = 0;
// wifi_mac register offsets (private in wifi_mac.rs; mirrored for the test).
const MAC_EVENT_GET: u64 = 0xC3C; // MAC event word (HW-set; read by the ISR)
const MAC_EVENT_CLR: u64 = 0xC40; // W1C acknowledge
const MAC_EVENT_RX_DONE: u32 = 0x0100_4000;

fn ledc_mut(bus: &mut SystemBus) -> &mut Esp32c3Ledc {
    let i = bus.find_peripheral_index_by_name("ledc").unwrap();
    bus.peripherals[i]
        .dev
        .as_any_mut()
        .unwrap()
        .downcast_mut::<Esp32c3Ledc>()
        .unwrap()
}

fn systimer_mut(bus: &mut SystemBus) -> &mut Systimer {
    let i = bus.find_peripheral_index_by_name("systimer").unwrap();
    bus.peripherals[i]
        .dev
        .as_any_mut()
        .unwrap()
        .downcast_mut::<Systimer>()
        .unwrap()
}

fn mac_mut(bus: &mut SystemBus) -> &mut Esp32c3WifiMac {
    let i = bus.find_peripheral_index_by_name("wifi_mac").unwrap();
    bus.peripherals[i]
        .dev
        .as_any_mut()
        .unwrap()
        .downcast_mut::<Esp32c3WifiMac>()
        .unwrap()
}

/// Swap the declarative SYSTIMER stub for a real scheduler-driven `Systimer`
/// and hand it the bus clock (as `add_peripheral` would).
fn prep_systimer(bus: &mut SystemBus, _scheduler: bool) {
    bus.refresh_peripheral_index();
    let i = bus
        .find_peripheral_index_by_name("systimer")
        .expect("devkit bus carries a systimer");
    let mut dev = Systimer::new_with_source(160_000_000, SYSTIMER_TARGET0_SOURCE);
    dev.attach_cycle_clock(bus.cycle_clock.clone());
    bus.peripherals[i].dev = Box::new(dev);
    bus.refresh_peripheral_index();
}

/// Arm TARGET0 in target mode at 3 SYSTIMER ticks, enable its IRQ.
fn arm_systimer(bus: &mut SystemBus) {
    bus.write_u32(SYSTIMER_BASE + 0x64, 1).unwrap(); // INT_ENA bit0
    bus.write_u32(SYSTIMER_BASE + 0x1C, 0).unwrap(); // TARGET0_HI
    bus.write_u32(SYSTIMER_BASE + 0x20, 3).unwrap(); // TARGET0_LO
    bus.write_u32(SYSTIMER_BASE + 0x50, 1).unwrap(); // COMP0_LOAD
    let conf = bus.read_u32(SYSTIMER_BASE).unwrap();
    bus.write_u32(SYSTIMER_BASE, conf | (1 << 24)).unwrap(); // TARGET0_WORK_EN
}

/// Advance the SYSTIMER past the target so the alarm latches
/// pending && int_ena → the model asserts matrix source 37.
fn raise_systimer(bus: &mut SystemBus) {
    systimer_mut(bus).sync_to(10_000);
}

/// INT_CLR bit0 clears the pending latch → level drops.
fn clear_systimer(bus: &mut SystemBus) {
    bus.write_u32(SYSTIMER_BASE + 0x6C, 1).unwrap();
}

fn prep_ledc(bus: &mut SystemBus, _scheduler: bool) {
    bus.refresh_peripheral_index();
}

/// Arm timer0 (period 16, divider 1) and enable its LSTIMER0_OVF int.
fn arm_ledc(bus: &mut SystemBus) {
    bus.write_u32(LEDC_BASE as u64 + LEDC_INT_ENA, 1).unwrap();
    bus.write_u32(LEDC_BASE as u64 + LEDC_TIMER0_CONF, LEDC_TIMER0_RUN)
        .unwrap();
}

/// Advance the LEDC past its first overflow (publishes the clock so the
/// scheduler-driven model's lazy counters materialise LSTIMER0_OVF).
fn raise_ledc(bus: &mut SystemBus) {
    bus.set_current_cycle(LEDC_PAST_OVF);
}

/// INT_CLR bit0 clears the LSTIMER0_OVF latch → level drops.
fn clear_ledc(bus: &mut SystemBus) {
    bus.write_u32(LEDC_BASE as u64 + LEDC_INT_CLR, 1).unwrap();
}

/// Pin the LEDC back onto the per-cycle walk, drive it past the overflow and
/// return what the walk re-emitted.
fn walk_reemit_ledc(bus: &mut SystemBus) -> Option<Vec<u32>> {
    let ledc = ledc_mut(bus);
    ledc.force_legacy_walk();
    assert!(
        !ledc.uses_scheduler(),
        "force_legacy_walk must pin LEDC back onto the per-cycle walk"
    );
    let mut irqs = None;
    for _ in 0..LEDC_PAST_OVF {
        irqs = ledc.tick().explicit_irqs;
    }
    irqs
}

/// Swap the declarative `wifi_mac` for the real behavioral model at its base.
/// With the bus cycle clock attached the model is scheduler-driven
/// (walk-skipped, level exported via `matrix_irq_sources`); without it the
/// model stays on the legacy per-cycle walk.
fn prep_mac(bus: &mut SystemBus, scheduler: bool) {
    let i = bus
        .find_peripheral_index_by_name("wifi_mac")
        .expect("devkit bus carries a wifi_mac");
    let mut dev = Esp32c3WifiMac::new();
    if scheduler {
        dev.attach_cycle_clock(bus.cycle_clock.clone());
    }
    bus.peripherals[i].dev = Box::new(dev);
    bus.refresh_peripheral_index();
}

fn no_arm(_bus: &mut SystemBus) {}

/// Arm the MAC event (HW normally sets it; the ISR reads it). The level
/// asserts source 0 while the event word is non-zero.
fn raise_mac(bus: &mut SystemBus) {
    bus.write_u32(MAC_BASE + MAC_EVENT_GET, MAC_EVENT_RX_DONE)
        .unwrap();
}

/// No clock attached → wifi_mac stays on the per-cycle walk; the walk tick
/// must re-emit the MAC level.
fn walk_reemit_mac(bus: &mut SystemBus) -> Option<Vec<u32>> {
    raise_mac(bus);
    let mac = mac_mut(bus);
    assert!(
        !mac.uses_scheduler(),
        "no clock attached → wifi_mac stays on the per-cycle walk"
    );
    mac.tick().explicit_irqs
}

/// Pin to / run on the walk and return what `tick()` re-emitted.
type WalkReemit = fn(&mut SystemBus) -> Option<Vec<u32>>;

struct Routed {
    name: &'static str,
    /// Interrupt-matrix source the peripheral asserts.
    source: u32,
    /// CPU line the source is routed to.
    line: u32,
    /// Swap/prepare the peripheral on the devkit bus (`scheduler`: attach the
    /// bus clock, for models whose drive mode depends on it).
    prep: fn(&mut SystemBus, bool),
    /// Arm the peripheral through ordinary MMIO writes at setup.
    arm: fn(&mut SystemBus),
    /// Make the level assert.
    raise: fn(&mut SystemBus),
    /// Acknowledge the level (None: the peripheral's clear is proven by its
    /// own bespoke test).
    clear: Option<fn(&mut SystemBus)>,
    /// Contract 2: pin to / run on the walk and return what `tick()`
    /// re-emitted, with `bool` = whether setup attaches the bus clock
    /// (None: no such test existed for this peripheral).
    walk_reemit: Option<(bool, WalkReemit)>,
}

const ROUTED: &[Routed] = &[
    Routed {
        name: "systimer",
        source: SYSTIMER_TARGET0_SOURCE,
        line: 5,
        prep: prep_systimer,
        arm: arm_systimer,
        raise: raise_systimer,
        clear: Some(clear_systimer),
        walk_reemit: None,
    },
    Routed {
        name: "ledc",
        source: LEDC_INTR_SOURCE_ID,
        line: 9,
        prep: prep_ledc,
        arm: arm_ledc,
        raise: raise_ledc,
        clear: Some(clear_ledc),
        walk_reemit: Some((true, walk_reemit_ledc)),
    },
    Routed {
        name: "wifi_mac",
        source: MAC_SOURCE,
        line: 6,
        prep: prep_mac,
        arm: no_arm,
        raise: raise_mac,
        // The EVENT_CLR acknowledge is proven on a walk-DELETED bus by
        // `event_clr_deasserts_on_walk_deleted_bus_at_the_write`.
        clear: None,
        walk_reemit: Some((false, walk_reemit_mac)),
    },
];

/// Build a devkit C3 bus, prepare the peripheral, route its source → `line`
/// (priority 1, threshold 1, line enabled) and arm it.
fn setup_routed(row: &Routed, scheduler: bool) -> SystemBus {
    let mut bus = devkit_bus();
    (row.prep)(&mut bus, scheduler);
    route_source(&mut bus, row.source, row.line);
    (row.arm)(&mut bus);
    bus
}

/// The scheduler routing arm and the legacy walk aggregation produce the
/// SAME `irq_fabric.esp32c3.irq_lines` for the SAME level.
#[test]
fn every_c3_routed_peripheral_scheduler_routing_matches_walk_routing() {
    for row in ROUTED {
        let Routed { name, source, .. } = *row;
        let mut bus = setup_routed(row, true);
        (row.raise)(&mut bus);
        let i = idx(&bus, name);
        assert_eq!(
            bus.peripherals[i].dev.matrix_irq_sources(),
            vec![source],
            "{name}: an armed+raised peripheral must assert matrix source {source}"
        );

        // Scheduler routing arm (what `apply_event_result` runs on the C3 bus).
        bus.refresh_esp32c3_sched_sources();
        bus.recompute_esp32c3_irq_lines();
        let scheduler_lines = bus.irq_fabric.esp32c3.irq_lines;
        assert_eq!(
            scheduler_lines,
            1 << row.line,
            "{name}: scheduler routing must assert the routed CPU line for source {source}"
        );

        // Legacy walk routing reference: the source re-emitted by the walk.
        // Must land on the identical line mask.
        bus.aggregate_esp32c3_irqs(&[source]);
        assert_eq!(
            bus.irq_fabric.esp32c3.irq_lines, scheduler_lines,
            "{name}: walk aggregation and scheduler routing must produce identical esp32c3.irq_lines"
        );
    }
}

/// A peripheral on the per-cycle walk re-emits the SAME matrix source from its
/// `tick()` — the level the scheduler path exports via `matrix_irq_sources` —
/// so both drive modes deliver the IRQ identically.
#[test]
fn every_c3_routed_peripheral_force_legacy_walk_reemits_same_source() {
    for row in ROUTED {
        let Some((scheduler, walk_reemit)) = row.walk_reemit else {
            continue;
        };
        let mut bus = setup_routed(row, scheduler);
        assert_eq!(
            walk_reemit(&mut bus),
            Some(vec![row.source]),
            "{}: a walk-driven peripheral must re-emit its level source from the walk tick",
            row.name
        );
    }
}

/// Clearing the level (INT_CLR) de-asserts the routed line on the next
/// re-derivation — same level semantics as the walk (which stops re-emitting
/// the source the tick after the clear).
#[test]
fn every_c3_routed_peripheral_clearing_level_deasserts_routed_line() {
    for row in ROUTED {
        let Some(clear) = row.clear else {
            continue;
        };
        let name = row.name;
        let mut bus = setup_routed(row, true);
        (row.raise)(&mut bus);
        bus.refresh_esp32c3_sched_sources();
        bus.recompute_esp32c3_irq_lines();
        assert_eq!(
            bus.irq_fabric.esp32c3.irq_lines,
            1 << row.line,
            "{name}: raised level must route to its CPU line before the clear"
        );

        clear(&mut bus);
        let i = idx(&bus, name);
        assert!(
            bus.peripherals[i].dev.matrix_irq_sources().is_empty(),
            "{name}: after INT_CLR the peripheral asserts no matrix source"
        );
        bus.refresh_esp32c3_sched_sources();
        bus.recompute_esp32c3_irq_lines();
        assert_eq!(
            bus.irq_fabric.esp32c3.irq_lines, 0,
            "{name}: routed line must de-assert once the level clears"
        );
    }
}

/// THE write-choke proof: on a fully walk-DELETED bus (no per-cycle walk to
/// re-derive the level), the `EVENT_CLR` acknowledge must de-assert the
/// routed line AT THE WRITE — otherwise the MAC level latches forever and
/// re-enters its ISR. This is the one legitimate bus addition the last
/// walker needs (`sync_esp32c3_irq_cache_write`).
#[test]
fn event_clr_deasserts_on_walk_deleted_bus_at_the_write() {
    let row = ROUTED
        .iter()
        .find(|r| r.name == "wifi_mac")
        .expect("wifi_mac row");
    let mut bus = setup_routed(row, true);
    // Delete the walk: with wifi_mac scheduler-driven and every other model
    // inert/scheduler, the devkit bus derives walk-deletion.
    bus.legacy_walk_disabled = bus.derive_walk_deletable();
    assert!(
        bus.legacy_walk_disabled,
        "scheduler-driven wifi_mac must let the devkit bus derive walk-deletion"
    );

    // Arm the MAC level and route it at the write choke (writing EVENT_GET is
    // a MAC-window write → the choke re-derives the scheduler level).
    raise_mac(&mut bus);
    assert_eq!(
        bus.irq_fabric.esp32c3.irq_lines,
        1 << row.line,
        "MAC event must route to the CPU line at the write, with NO walk tick"
    );

    // Acknowledge via EVENT_CLR (W1C). On a walk-deleted bus the ONLY thing
    // that can de-assert the level is the write choke re-derivation.
    bus.write_u32(MAC_BASE + MAC_EVENT_CLR, MAC_EVENT_RX_DONE)
        .unwrap();
    assert!(
        mac_mut(&mut bus).matrix_irq_sources().is_empty(),
        "after EVENT_CLR the MAC asserts no matrix source"
    );
    assert_eq!(
        bus.irq_fabric.esp32c3.irq_lines, 0,
        "EVENT_CLR must de-assert the routed line at the write on a walk-deleted bus"
    );
}
