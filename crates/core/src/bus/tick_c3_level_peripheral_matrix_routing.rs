use crate::bus::SystemBus;
use crate::peripherals::esp32c3::apb_saradc::{Esp32c3ApbSarAdc, APB_SARADC_INTR_SOURCE_ID};
use crate::peripherals::esp32c3::spi::{Esp32c3Spi, SPI2_INTR_SOURCE_ID, TRANS_DONE};
use crate::Bus;
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
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip = ChipDescriptor::from_file(root.join("../../configs/chips/esp32c3.yaml"))
        .expect("load esp32c3 chip yaml");
    let manifest =
        SystemManifest::from_file(root.join("../../configs/systems/esp32c3-devkit.yaml"))
            .expect("load esp32c3-devkit system yaml");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build c3 devkit bus");
    bus.irq_fabric.esp32c3.routing = true;
    bus.refresh_peripheral_index();
    bus.write_u32(INTMATRIX + (source as u64) * 4, LINE)
        .unwrap();
    bus.write_u32(INTMATRIX + 0x114 + (LINE as u64) * 4, 1)
        .unwrap();
    bus.write_u32(INTMATRIX + 0x194, 1).unwrap();
    bus.write_u32(INTMATRIX + 0x104, 1 << LINE).unwrap();
    bus
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
