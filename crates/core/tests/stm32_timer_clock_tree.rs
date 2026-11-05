// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! RCC-to-timer timing regressions. RM0090 §7.2 and RM0008 §7.2 specify
//! TIMxCLK = PCLK for APB /1, and TIMxCLK = 2 * PCLK otherwise. The F4
//! cases cover F405/F407 and the default TIMPRE=0 mode of the shared model.
//! The simulated CPU frequency remains the configured HCLK timebase.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::rcc::{Rcc, RccRegisterLayout};
use labwired_core::peripherals::timer::{Timer, TimerOutputSnapshot};
use labwired_core::{Bus, CycleClock, Peripheral};

const RCC: u64 = 0x4002_3800;
const TIM1: u64 = 0x4001_0000;
const TIM2: u64 = 0x4000_0000;
const APB1_TIMERS: [u64; 4] = [TIM2, TIM2 + 0x400, TIM2 + 0x800, TIM2 + 0xC00];
const TIMER_DIVIDERS: [u32; 8] = [1, 1, 1, 1, 1, 2, 4, 8];

fn f407_bus() -> SystemBus {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let chip = ChipDescriptor::from_file(root.join("configs/chips/stm32f407.yaml")).unwrap();
    let manifest: SystemManifest =
        serde_yaml::from_str("name: timer-clock-tree\nchip: unused\nwalk_deleted: false\n")
            .unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    bus.write_u32(RCC + 0x40, 0xF).unwrap(); // TIM2..5EN
    bus.write_u32(RCC + 0x44, 1).unwrap(); // TIM1EN
    bus
}

fn start_timer(bus: &mut SystemBus, base: u64, prescaler: u32) {
    bus.write_u32(base + 0x28, prescaler).unwrap();
    bus.write_u32(base + 0x2C, 0xFFFF).unwrap();
    bus.write_u32(base, 1).unwrap();
}

fn advance_bus(bus: &mut SystemBus, cycles: u64) {
    if cfg!(feature = "event-scheduler") {
        bus.set_current_cycle(bus.current_cycle + cycles);
    } else {
        for _ in 0..cycles {
            bus.set_current_cycle(bus.current_cycle + 1);
            bus.tick_peripherals_fully();
        }
    }
}

fn tim2_snapshot(bus: &SystemBus) -> TimerOutputSnapshot {
    let idx = bus.find_peripheral_index_by_name("tim2").unwrap();
    bus.peripherals[idx]
        .dev
        .as_any()
        .unwrap()
        .downcast_ref::<Timer>()
        .unwrap()
        .output_snapshot()
}

#[test]
fn f1_and_f4_decode_both_apb_timer_domains() {
    for (layout, cfgr, apb1enr, apb2enr, ppre1_shift, ppre2_shift) in [
        (RccRegisterLayout::Stm32F1, 0x04, 0x1C, 0x18, 8, 11),
        (RccRegisterLayout::Stm32F4, 0x08, 0x40, 0x44, 10, 13),
    ] {
        let mut rcc = Rcc::new_with_layout(layout);
        for ppre1 in 0..8 {
            for ppre2 in 0..8 {
                rcc.write_u32(cfgr, (ppre1 << ppre1_shift) | (ppre2 << ppre2_shift))
                    .unwrap();
                assert_eq!(
                    rcc.timer_clock_divider(apb1enr),
                    Some(TIMER_DIVIDERS[ppre1 as usize]),
                );
                assert_eq!(
                    rcc.timer_clock_divider(apb2enr),
                    Some(TIMER_DIVIDERS[ppre2 as usize]),
                );
                assert_eq!(rcc.timer_clock_divider(0), None);
            }
        }
    }
}

#[test]
fn f407_production_descriptors_clock_every_timer_from_its_apb_domain() {
    for ppre1 in 0..8 {
        let ppre2 = 7 - ppre1;
        let mut bus = f407_bus();
        bus.write_u32(RCC + 0x08, (ppre1 << 10) | (ppre2 << 13))
            .unwrap();
        for base in [TIM1].into_iter().chain(APB1_TIMERS) {
            start_timer(&mut bus, base, 2);
        }
        advance_bus(&mut bus, 240);
        assert_eq!(
            bus.read_u32(TIM1 + 0x24).unwrap(),
            240 / (3 * TIMER_DIVIDERS[ppre2 as usize]),
            "TIM1 APB2 PPRE2={ppre2}",
        );
        for base in APB1_TIMERS {
            assert_eq!(
                bus.read_u32(base + 0x24).unwrap(),
                240 / (3 * TIMER_DIVIDERS[ppre1 as usize]),
                "timer {base:#x} APB1 PPRE1={ppre1}",
            );
        }
    }
}

#[test]
fn live_byte_halfword_and_word_prescaler_writes_preserve_cnt_and_psc_phase() {
    let mut bus = f407_bus();
    bus.write_u32(RCC + 0x08, 5 << 10).unwrap(); // APB /4, timer /2
    start_timer(&mut bus, TIM2, 2);
    advance_bus(&mut bus, 5); // two PSC ticks and one partial kernel tick
                              // No CNT read before the switch. The bus must synchronize the OLD clock.
    bus.write_u8(RCC + 0x09, 6 << 2).unwrap(); // APB /8, timer /4
    assert_eq!(bus.read_u32(TIM2 + 0x24).unwrap(), 0);
    assert_eq!(bus.read_u32(TIM2 + 0x28).unwrap(), 2);
    advance_bus(&mut bus, 3);
    assert_eq!(bus.read_u32(TIM2 + 0x24).unwrap(), 0);
    advance_bus(&mut bus, 1);
    assert_eq!(bus.read_u32(TIM2 + 0x24).unwrap(), 1);
    advance_bus(&mut bus, 24);
    assert_eq!(bus.read_u32(TIM2 + 0x24).unwrap(), 3);

    bus.write_u16(RCC + 0x08, 7 << 10).unwrap(); // APB /16, timer /8
    advance_bus(&mut bus, 48);
    assert_eq!(bus.read_u32(TIM2 + 0x24).unwrap(), 5);
    bus.write_u32(RCC + 0x08, 0).unwrap(); // APB /1, timer /1
    advance_bus(&mut bus, 3);
    assert_eq!(bus.read_u32(TIM2 + 0x24).unwrap(), 6);
}

#[test]
fn rcc_enable_gate_preserves_counter_and_both_divider_phases() {
    let mut bus = f407_bus();
    bus.write_u32(RCC + 0x08, 5 << 10).unwrap(); // timer /2
    start_timer(&mut bus, TIM2, 2);
    advance_bus(&mut bus, 11);
    let before = tim2_snapshot(&bus);
    assert_eq!((before.counter_ticks, before.prescaler_phase), (1, 5));
    bus.write_u32(RCC + 0x40, 0xE).unwrap(); // gate only TIM2
    advance_bus(&mut bus, 100);
    let stopped = tim2_snapshot(&bus);
    assert_eq!((stopped.counter_ticks, stopped.prescaler_phase), (1, 5));
    assert!(stopped.counter_frozen);
    assert_eq!(bus.read_u32(TIM2 + 0x24).unwrap(), 0); // gated MMIO
    bus.write_u32(RCC + 0x40, 0xF).unwrap();
    advance_bus(&mut bus, 1);
    let resumed = tim2_snapshot(&bus);
    assert_eq!((resumed.counter_ticks, resumed.prescaler_phase), (2, 0));
    assert!(!resumed.counter_frozen);
}

#[test]
fn diagnostic_gate_bypass_and_missing_clock_fault_refresh_timer_state() {
    let mut bus = f407_bus();
    start_timer(&mut bus, TIM2, 0);
    advance_bus(&mut bus, 3);
    bus.write_u32(RCC + 0x40, 0xE).unwrap();
    advance_bus(&mut bus, 10);
    assert_eq!(tim2_snapshot(&bus).counter_ticks, 3);
    bus.set_clock_gating_bypass(true);
    advance_bus(&mut bus, 5);
    assert_eq!(tim2_snapshot(&bus).counter_ticks, 8);
    bus.set_clock_gating_bypass(false);
    advance_bus(&mut bus, 5);
    assert_eq!(tim2_snapshot(&bus).counter_ticks, 8);
    bus.set_clock_gating_bypass(true);
    bus.inject_missing_clock("tim2").unwrap();
    assert_eq!(bus.missing_clock_suppressed("tim2"), 0);
    advance_bus(&mut bus, 4);
    assert_eq!(tim2_snapshot(&bus).counter_ticks, 8);
    assert_eq!(bus.read_u32(TIM2 + 0x24).unwrap(), 0);
    assert!(bus.missing_clock_suppressed("tim2") > 0);
}

fn advance_timer(timer: &mut Timer, clock: &CycleClock, cycle: u64) {
    clock.publish(cycle);
    if timer.uses_scheduler() {
        timer.sync_to(cycle);
    } else {
        timer.tick();
    }
}

#[test]
fn fractional_kernel_ticks_and_pwm_edges_agree_in_both_drive_modes() {
    for scheduler in [false, true] {
        let clock = CycleClock::default();
        let mut timer = Timer::new();
        if scheduler {
            timer.attach_cycle_clock(clock.clone());
        }
        timer.set_timer_clock(2, true);
        timer.output_lines_arc();
        timer.write_u32(0x28, 1).unwrap(); // /2 PSC, /4 combined
        timer.write_u32(0x2C, 9).unwrap();
        timer.write_u32(0x34, 3).unwrap();
        timer.write_u32(0x18, 0b110 << 4).unwrap();
        timer.write_u32(0x20, 1).unwrap();
        timer.write_u32(0x00, 1).unwrap();
        for cycle in 1..=5 {
            advance_timer(&mut timer, &clock, cycle);
            let snapshot = timer.output_snapshot();
            assert_eq!(snapshot.counter_ticks, (cycle / 4) as u32);
            assert_eq!(snapshot.prescaler_divisor, 4);
            assert_eq!(snapshot.prescaler_phase, (cycle % 4) as u32);
        }
        // This rate change extrapolates CNT=0 before CPU cycle zero. The
        // waveform must retain CNT=1 and restart just the kernel phase.
        assert!(timer.set_timer_clock(4, true));
        for cycle in 6..=165 {
            advance_timer(&mut timer, &clock, cycle);
            let count = (1 + (cycle - 5) / 8) % 10;
            assert_eq!(timer.read_u32(0x24).unwrap(), count as u32);
            assert_eq!(timer.output_level(0), count < 3, "cycle {cycle}");
        }
    }
}

#[test]
fn gated_pwm_holds_its_level_and_resumes_the_remaining_phase() {
    for scheduler in [false, true] {
        let clock = CycleClock::default();
        let mut timer = Timer::new();
        if scheduler {
            timer.attach_cycle_clock(clock.clone());
        }
        timer.set_timer_clock(2, true);
        timer.output_lines_arc();
        timer.write_u32(0x28, 1).unwrap();
        timer.write_u32(0x2C, 9).unwrap();
        timer.write_u32(0x34, 3).unwrap();
        timer.write_u32(0x18, 0b110 << 4).unwrap();
        timer.write_u32(0x20, 1).unwrap();
        timer.write_u32(0x00, 1).unwrap();
        let old_events = timer.take_scheduled_events();
        for cycle in 1..=5 {
            advance_timer(&mut timer, &clock, cycle);
        }
        timer.set_timer_clock(2, false);
        assert!(timer.take_scheduled_events().is_empty());
        for cycle in 6..=105 {
            advance_timer(&mut timer, &clock, cycle);
            let snapshot = timer.output_snapshot();
            assert_eq!((snapshot.counter_ticks, snapshot.prescaler_phase), (1, 1));
            assert!(timer.output_level(0));
        }
        timer.set_timer_clock(2, true);
        let new_events = timer.take_scheduled_events();
        if timer.uses_scheduler() {
            assert_eq!(new_events.len(), 1);
            assert_eq!(new_events[0].0, 6); // falling edge at cycle112
            assert_ne!(new_events[0].1, old_events[0].1);
        }
        for cycle in 106..=185 {
            advance_timer(&mut timer, &clock, cycle);
            let count = (1 + (1 + cycle - 105) / 4) % 10;
            assert_eq!(timer.read_u32(0x24).unwrap(), count as u32);
            assert_eq!(timer.output_level(0), count < 3, "cycle {cycle}");
        }
    }
}

#[test]
fn snapshot_keeps_kernel_phase_when_psc_is_reduced_below_its_live_phase() {
    for scheduler in [false, true] {
        let clock = CycleClock::default();
        let mut timer = Timer::new();
        if scheduler {
            timer.attach_cycle_clock(clock.clone());
        }
        timer.set_timer_clock(8, true);
        timer.write_u32(0x28, 3).unwrap();
        timer.write_u32(0, 1).unwrap();
        for cycle in 1..=19 {
            advance_timer(&mut timer, &clock, cycle);
        }
        timer.write_u32(0x28, 0).unwrap();
        let snapshot = timer.output_snapshot();
        assert_eq!(snapshot.prescaler_divisor, 8);
        assert_eq!(snapshot.prescaler_phase, 3);
        for cycle in 20..=23 {
            advance_timer(&mut timer, &clock, cycle);
            assert_eq!(timer.read_u32(0x24).unwrap(), 0);
        }
        advance_timer(&mut timer, &clock, 24);
        assert_eq!(timer.read_u32(0x24).unwrap(), 1);
    }
}

#[cfg(feature = "event-scheduler")]
#[test]
fn rcc_rate_change_rearms_irq_deadline_and_invalidates_the_old_event() {
    let mut bus = f407_bus();
    let timer_idx = bus.find_peripheral_index_by_name("tim2").unwrap();
    bus.write_u32(RCC + 0x08, 5 << 10).unwrap(); // timer /2
    start_timer(&mut bus, TIM2, 2);
    bus.write_u32(TIM2 + 0x2C, 9).unwrap(); // update after 60 CPU cycles
    bus.write_u32(TIM2 + 0x0C, 1).unwrap();
    let old = *bus.pending_schedule.last().unwrap();
    assert_eq!((old.0, old.1), (timer_idx, 60));
    bus.pending_schedule.clear();
    bus.set_current_cycle(5);
    bus.write_u32(RCC + 0x08, 6 << 10).unwrap(); // remaining 28 kernel ticks at /4
    let new = *bus.pending_schedule.last().unwrap();
    assert_eq!((new.0, new.1), (timer_idx, 117));
    assert_ne!(old.2, new.2);

    let mut scheduler = labwired_core::sched::EventScheduler::new();
    let mut event_bus = SystemBus::empty();
    scheduler.advance_to(old.1);
    let stale = bus.peripherals[timer_idx]
        .dev
        .on_event(old.2, &mut scheduler, &mut event_bus);
    assert!(!stale.raise_own_irq);
    assert_eq!(stale.reschedule_delay, None);
    scheduler.advance_to(new.1);
    bus.set_current_cycle(new.1);
    let fresh = bus.peripherals[timer_idx]
        .dev
        .on_event(new.2, &mut scheduler, &mut event_bus);
    assert!(fresh.raise_own_irq);
    assert_ne!(bus.read_u32(TIM2 + 0x10).unwrap() & 1, 0);
}

#[cfg(feature = "event-scheduler")]
#[test]
fn rcc_gate_retires_irq_deadlines_and_rearms_only_remaining_ticks() {
    let mut bus = f407_bus();
    let timer_idx = bus.find_peripheral_index_by_name("tim2").unwrap();
    bus.write_u32(RCC + 0x08, 5 << 10).unwrap();
    start_timer(&mut bus, TIM2, 2);
    bus.write_u32(TIM2 + 0x2C, 9).unwrap();
    bus.write_u32(TIM2 + 0x0C, 1).unwrap();
    let old = *bus.pending_schedule.last().unwrap();
    assert_eq!(old.1, 60);
    bus.pending_schedule.clear();
    bus.set_current_cycle(5);
    bus.write_u32(RCC + 0x40, 0xE).unwrap();
    assert!(bus.pending_schedule.is_empty());
    let mut scheduler = labwired_core::sched::EventScheduler::new();
    let mut event_bus = SystemBus::empty();
    scheduler.advance_to(60);
    bus.set_current_cycle(60);
    let stale = bus.peripherals[timer_idx]
        .dev
        .on_event(old.2, &mut scheduler, &mut event_bus);
    assert!(!stale.raise_own_irq);
    assert_eq!(stale.reschedule_delay, None);
    bus.set_current_cycle(105);
    bus.write_u32(RCC + 0x40, 0xF).unwrap();
    let resumed = *bus.pending_schedule.last().unwrap();
    assert_eq!(resumed.1, 160);
    bus.set_current_cycle(159);
    assert_eq!(bus.read_u32(TIM2 + 0x10).unwrap() & 1, 0);
    scheduler.advance_to(160);
    bus.set_current_cycle(160);
    let event = bus.peripherals[timer_idx]
        .dev
        .on_event(resumed.2, &mut scheduler, &mut event_bus);
    assert!(event.raise_own_irq);
}

#[cfg(feature = "event-scheduler")]
#[test]
fn capture_filter_and_remaining_deadline_follow_the_timer_kernel_clock() {
    let clock = CycleClock::default();
    let mut timer = Timer::new_with_width(32).ccer_np(true);
    timer.attach_cycle_clock(clock.clone());
    timer.set_timer_clock(2, true);
    timer.write_u32(0x18, 0x11).unwrap(); // CC1S=TI1, IC1F=2 kernel ticks
    timer.write_u32(0x20, 1).unwrap();
    timer.write_u32(0, 1).unwrap();
    clock.publish(10);
    timer.sync_to(10);
    timer.input_edge(0, true, 10); // accepted at 14 under old clock
    assert_eq!(timer.take_scheduled_events()[0].0, 3);
    clock.publish(12);
    timer.set_timer_clock(4, true); // one kernel tick remains, accepted at 16
    assert_eq!(timer.take_scheduled_events()[0].0, 3);
    clock.publish(15);
    timer.sync_to(15);
    assert_eq!(timer.read_u32(0x10).unwrap() & 2, 0);
    clock.publish(16);
    timer.sync_to(16);
    assert_eq!(timer.read_u32(0x10).unwrap() & 2, 2);
    assert_eq!(timer.read_u32(0x34).unwrap(), 7);
}

#[cfg(feature = "event-scheduler")]
#[test]
fn gated_capture_filter_resumes_its_remaining_samples() {
    let clock = CycleClock::default();
    let mut timer = Timer::new_with_width(32).ccer_np(true);
    timer.attach_cycle_clock(clock.clone());
    timer.set_timer_clock(2, true);
    timer.write_u32(0x18, 0x11).unwrap();
    timer.write_u32(0x20, 1).unwrap();
    timer.write_u32(0, 1).unwrap();
    clock.publish(10);
    timer.sync_to(10);
    timer.input_edge(0, true, 10);
    clock.publish(11);
    timer.set_timer_clock(2, false);
    assert!(timer.take_scheduled_events().is_empty());
    clock.publish(111);
    timer.sync_to(111);
    assert_eq!(timer.read_u32(0x10).unwrap() & 2, 0);
    timer.set_timer_clock(2, true);
    assert_eq!(timer.take_scheduled_events()[0].0, 2);
    clock.publish(113);
    timer.sync_to(113);
    assert_eq!(timer.read_u32(0x10).unwrap() & 2, 0);
    clock.publish(114);
    timer.sync_to(114);
    assert_eq!(timer.read_u32(0x10).unwrap() & 2, 2);
    assert_eq!(timer.read_u32(0x34).unwrap(), 7);
}

#[cfg(feature = "event-scheduler")]
#[test]
fn input_changes_while_gated_cancel_obsolete_edges_and_restart_filtering() {
    for (pending_rise, transitions, expect_capture) in [
        (true, vec![(50, false)], false),
        (false, vec![(50, true)], true),
        (false, vec![(50, true), (60, false)], false),
        (true, vec![(50, false), (60, true)], true),
    ] {
        let clock = CycleClock::default();
        let mut timer = Timer::new_with_width(32).ccer_np(true);
        timer.attach_cycle_clock(clock.clone());
        timer.set_timer_clock(2, true);
        timer.write_u32(0x18, 0x11).unwrap();
        timer.write_u32(0x20, 1).unwrap();
        timer.write_u32(0, 1).unwrap();
        clock.publish(10);
        timer.sync_to(10);
        if pending_rise {
            timer.input_edge(0, true, 10); // old acceptance cycle14
        }
        clock.publish(11);
        timer.set_timer_clock(2, false);
        for (cycle, level) in transitions {
            clock.publish(cycle);
            timer.sync_to(cycle);
            timer.input_edge(0, level, cycle);
        }
        clock.publish(111);
        timer.set_timer_clock(2, true);
        clock.publish(114);
        timer.sync_to(114);
        assert_eq!(timer.read_u32(0x10).unwrap() & 2, 0);
        clock.publish(115); // four fresh CPU cycles at /2
        timer.sync_to(115);
        assert_eq!(timer.read_u32(0x10).unwrap() & 2 != 0, expect_capture);
        if expect_capture {
            assert_eq!(timer.read_u32(0x34).unwrap(), 7);
        }
    }
}
