// SPDX-License-Identifier: MIT
//! Timer DMA event truth, from RM_AT32F421 V2.05 sections9/14 and the
//! corresponding STM32 TIM DIER/CR2 contract. No physical timing claim.
use labwired_core::{
    peripherals::timer::Timer, Peripheral, DMA_SIGNAL_TIMER_CH1, DMA_SIGNAL_TIMER_CH2,
    DMA_SIGNAL_TIMER_COMMUTATION, DMA_SIGNAL_TIMER_TRIGGER, DMA_SIGNAL_TIMER_UPDATE,
};

fn w(t: &mut Timer, r: u64, v: u32) {
    t.write_u32(r, v).unwrap();
}
fn tick_signals(t: &mut Timer) -> Vec<u32> {
    t.tick().dma_signals.unwrap_or_default()
}

#[test]
fn each_capture_requests_dma_independently_of_its_sticky_status_flag() {
    let mut t = Timer::new().ccer_np(true);
    w(&mut t, 0x18, 1);
    w(&mut t, 0x20, 0xb);
    w(&mut t, 0x0c, 1 << 9);
    w(&mut t, 0, 1);
    for _ in 0..10 {
        assert!(tick_signals(&mut t).is_empty());
    }
    t.input_edge(0, true, 10);
    assert_eq!(tick_signals(&mut t), [DMA_SIGNAL_TIMER_CH1]);
    // No CCR/SR read or clear between captures; a DMA pulse is an event.
    t.input_edge(0, false, 11);
    assert_eq!(tick_signals(&mut t), [DMA_SIGNAL_TIMER_CH1]);
    assert_ne!(
        t.read_u32(0x10).unwrap() & (1 << 9),
        0,
        "overcapture remains observable"
    );
    assert_eq!(t.read_u32(0x34).unwrap(), 11);
    assert!(tick_signals(&mut t).is_empty(), "no sticky-flag replay");
}

#[test]
fn disabled_channel_prescaler_wrong_input_and_reset_do_not_create_requests() {
    let mut t = Timer::new().ccer_np(true);
    w(&mut t, 0x18, 1 | (1 << 2));
    w(&mut t, 0x20, 0xb);
    w(&mut t, 0x0c, 1 << 9);
    t.input_edge(1, true, 0);
    assert!(tick_signals(&mut t).is_empty());
    t.input_edge(0, true, 0);
    assert!(tick_signals(&mut t).is_empty());
    t.input_edge(0, false, 0);
    assert_eq!(tick_signals(&mut t), [DMA_SIGNAL_TIMER_CH1]);
    w(&mut t, 0x20, 0);
    t.input_edge(0, true, 0);
    assert!(tick_signals(&mut t).is_empty());
    w(&mut t, 0x20, 0xb);
    w(&mut t, 0x18, 1);
    w(&mut t, 0x0c, 0);
    t.input_edge(0, false, 0);
    w(&mut t, 0x0c, 1 << 9);
    assert!(
        tick_signals(&mut t).is_empty(),
        "enable must not replay an old flag"
    );
    t.input_edge(0, true, 0);
    t.on_node_reset();
    assert!(
        tick_signals(&mut t).is_empty(),
        "reset cancels pending requests"
    );
}

#[test]
fn output_compare_repeats_without_interrupt_enable_or_status_clear() {
    let mut t = Timer::new();
    w(&mut t, 0x2c, 5);
    w(&mut t, 0x34, 3);
    w(&mut t, 0x0c, 1 << 9);
    w(&mut t, 0, 1);
    let hits: Vec<_> = (1..=15)
        .filter_map(|cycle| {
            let r = t.tick();
            assert!(!r.irq);
            r.dma_signals.map(|signals| (cycle, signals))
        })
        .collect();
    assert_eq!(
        hits,
        vec![
            (3, vec![DMA_SIGNAL_TIMER_CH1]),
            (9, vec![DMA_SIGNAL_TIMER_CH1]),
            (15, vec![DMA_SIGNAL_TIMER_CH1])
        ]
    );
}

#[test]
fn channel_dma_source_selects_update_instead_of_capture_compare() {
    let mut t = Timer::new();
    w(&mut t, 0x2c, 4);
    w(&mut t, 0x34, 2);
    w(&mut t, 4, 1 << 3);
    w(&mut t, 0x0c, (1 << 8) | (1 << 9) | (1 << 10));
    w(&mut t, 0, 1);
    for _ in 0..4 {
        assert!(tick_signals(&mut t).is_empty());
    }
    assert_eq!(
        tick_signals(&mut t),
        [
            DMA_SIGNAL_TIMER_UPDATE,
            DMA_SIGNAL_TIMER_CH1,
            DMA_SIGNAL_TIMER_CH2
        ]
    );
}

#[test]
fn software_update_honors_urs_and_udis_but_natural_overflow_honors_urs() {
    for (cr1, expected) in [(0, true), (1 << 2, false), (1 << 1, false)] {
        let mut t = Timer::new().basic(true);
        w(&mut t, 0, cr1);
        w(&mut t, 0x0c, 1 << 8);
        w(&mut t, 0x14, 1);
        assert_eq!(!tick_signals(&mut t).is_empty(), expected, "CR1={cr1}");
    }
    for (cr1, expected) in [(1, true), (1 | (1 << 2), true), (1 | (1 << 1), false)] {
        let mut t = Timer::new().basic(true);
        w(&mut t, 0x2c, 1);
        w(&mut t, 0, cr1);
        w(&mut t, 0x0c, 1 << 8);
        assert!(tick_signals(&mut t).is_empty());
        assert_eq!(!tick_signals(&mut t).is_empty(), expected);
    }
}

#[test]
fn software_capture_trigger_and_commutation_use_their_own_requests() {
    let mut t = Timer::new_with_layout(16, true);
    w(&mut t, 0x0c, (1 << 9) | (1 << 13) | (1 << 14));
    w(&mut t, 0x14, (1 << 1) | (1 << 5) | (1 << 6));
    assert_eq!(
        tick_signals(&mut t),
        [
            DMA_SIGNAL_TIMER_CH1,
            DMA_SIGNAL_TIMER_COMMUTATION,
            DMA_SIGNAL_TIMER_TRIGGER
        ]
    );
    assert!(tick_signals(&mut t).is_empty());
}

#[cfg(feature = "event-scheduler")]
#[test]
fn scheduled_dma_wakes_without_irq_and_rearms_while_sr_stays_set() {
    use labwired_core::{bus::SystemBus, sched::EventScheduler, CycleClock};
    let clock = CycleClock::default();
    let mut t = Timer::new();
    t.attach_cycle_clock(clock.clone());
    w(&mut t, 0x2c, 5);
    w(&mut t, 0x34, 3);
    w(&mut t, 0x0c, 1 << 9);
    w(&mut t, 0, 1);
    let armed = t.take_scheduled_events();
    assert_eq!(armed.len(), 1);
    assert_eq!(armed[0].0, 2);
    let token = armed[0].1;
    let mut sched = EventScheduler::new();
    let mut bus = SystemBus::empty();
    for cycle in [3, 9, 15] {
        sched.advance_to(cycle);
        clock.publish(cycle);
        let event = t.on_event(token, &mut sched, &mut bus);
        assert_eq!(event.dma_signals, [DMA_SIGNAL_TIMER_CH1]);
        assert!(!event.raise_own_irq);
        assert_eq!(event.reschedule_delay, Some(6));
    }
}

#[cfg(feature = "event-scheduler")]
#[test]
fn scheduled_filtered_capture_routes_one_request_at_the_acceptance_cycle() {
    use labwired_core::{bus::SystemBus, sched::EventScheduler, CycleClock};
    let clock = CycleClock::default();
    let mut t = Timer::new().ccer_np(true);
    t.attach_cycle_clock(clock.clone());
    w(&mut t, 0x18, 1 | (1 << 4));
    w(&mut t, 0x20, 0xb);
    w(&mut t, 0x0c, 1 << 9);
    w(&mut t, 0, 1);
    clock.publish(10);
    t.sync_to(10);
    t.input_edge(0, true, 10);
    let armed = t.take_scheduled_events();
    assert_eq!(armed.len(), 1);
    let at = 11 + armed[0].0;
    assert_eq!(at, 12, "two-sample filter");
    let mut sched = EventScheduler::new();
    sched.advance_to(at);
    clock.publish(at);
    let event = t.on_event(armed[0].1, &mut sched, &mut SystemBus::empty());
    assert_eq!(event.dma_signals, [DMA_SIGNAL_TIMER_CH1]);
    assert_eq!(t.read_u32(0x34).unwrap(), 12);
    assert_eq!(event.reschedule_delay, None);
}
