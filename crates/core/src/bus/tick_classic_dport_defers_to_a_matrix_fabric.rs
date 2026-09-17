use super::*;
use crate::peripherals::esp32::dport::{Dport, DPORT_PRO_MAC_INTR_MAP_REG_OFFSET};

/// A matrix source the DPORT PRO map binds to a CPU slot below.
const SOURCE: u32 = 34; // ETS_UART0_INTR_SOURCE
const SLOT: u8 = 9;

/// Emits `SOURCE` as an `explicit_irqs` level on every tick.
#[derive(Debug)]
struct LevelSource;

impl Peripheral for LevelSource {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }
    fn tick(&mut self) -> crate::PeripheralTickResult {
        crate::PeripheralTickResult {
            explicit_irqs: Some(vec![SOURCE]),
            ..Default::default()
        }
    }
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
    bus.add_peripheral("level", 0x5000_0000, 0x100, None, Box::new(LevelSource));
    // TRM §7: PRO map base 0x104, source `s` at `base + s*4`, low 5 bits =
    // the CPU interrupt slot the source is bound to on that core.
    bus.write_u32(
        Dport::BASE as u64 + (DPORT_PRO_MAC_INTR_MAP_REG_OFFSET + SOURCE * 4) as u64,
        SLOT as u32,
    )
    .expect("bind DPORT PRO map entry");
    bus
}

#[test]
fn dport_routes_when_no_matrix_fabric_is_active() {
    let mut bus = dport_bus();
    assert!(!bus.irq_fabric.matrix_owns_cpu_irqs());
    bus.tick_peripherals_fully();
    assert_eq!(
        bus.pending_cpu_irqs,
        [1u32 << SLOT, 0],
        "a classic bus with no matrix fabric must route DPORT sources itself"
    );
}

#[test]
fn an_active_c3_fabric_takes_ownership_of_pending_cpu_irqs() {
    let mut bus = dport_bus();
    bus.irq_fabric.esp32c3.routing = true;
    assert!(bus.irq_fabric.matrix_owns_cpu_irqs());
    bus.tick_peripherals_fully();
    assert_eq!(
        bus.pending_cpu_irqs,
        [0, 0],
        "the C3 fabric owns the routed output; DPORT must not write over it"
    );
}

#[test]
fn an_active_s3_fabric_takes_ownership_of_pending_cpu_irqs() {
    let mut bus = dport_bus();
    // Set AFTER the last `add_peripheral`: `rebuild_peripheral_ranges`
    // derives this flag from the presence of the S3 intmatrix model.
    bus.irq_fabric.esp32s3.routing = true;
    assert!(bus.irq_fabric.matrix_owns_cpu_irqs());
    bus.tick_peripherals_fully();
    assert_eq!(
        bus.pending_cpu_irqs,
        [0, 0],
        "the S3 fabric owns the routed output; DPORT must not write over it"
    );
}
