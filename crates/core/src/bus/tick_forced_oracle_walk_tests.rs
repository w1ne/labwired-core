use super::SystemBus;
use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

#[derive(Debug)]
struct OrderedTick {
    value: u32,
    scheduler: bool,
    order: Arc<Mutex<Vec<u32>>>,
}

impl Peripheral for OrderedTick {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        self.order.lock().unwrap().push(self.value);
        PeripheralTickResult::default()
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler
    }
}

#[derive(Debug)]
struct OneShotDynamic {
    active: bool,
    ticks: Arc<AtomicUsize>,
}

impl Peripheral for OneShotDynamic {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        self.ticks.fetch_add(1, Ordering::SeqCst);
        self.active = false;
        PeripheralTickResult::default()
    }

    fn legacy_tick_active(&self) -> bool {
        self.active
    }

    fn legacy_tick_dynamic(&self) -> bool {
        true
    }
}

#[test]
fn forced_walk_preserves_registration_order_across_drive_modes() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let mut bus = SystemBus::empty();
    bus.add_peripheral(
        "scheduler_first",
        0x1000,
        0x100,
        None,
        Box::new(OrderedTick {
            value: 1,
            scheduler: true,
            order: order.clone(),
        }),
    );
    bus.add_peripheral(
        "legacy_second",
        0x2000,
        0x100,
        None,
        Box::new(OrderedTick {
            value: 2,
            scheduler: false,
            order: order.clone(),
        }),
    );

    bus.tick_peripherals_fully_forced();

    assert_eq!(*order.lock().unwrap(), vec![1, 2]);
}

#[test]
fn forced_walk_advances_past_dynamic_entry_that_turns_inactive() {
    let ticks = Arc::new(AtomicUsize::new(0));
    let mut bus = SystemBus::empty();
    bus.add_peripheral(
        "one_shot",
        0x1000,
        0x100,
        None,
        Box::new(OneShotDynamic {
            active: true,
            ticks: ticks.clone(),
        }),
    );

    bus.tick_peripherals_fully_forced();

    assert_eq!(ticks.load(Ordering::SeqCst), 1);
}
