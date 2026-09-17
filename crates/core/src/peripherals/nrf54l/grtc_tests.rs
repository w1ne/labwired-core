use super::*;

/// SYSCOUNTER[0].SYSCOUNTERL / .SYSCOUNTERH.
const SYSCOUNTERL: u64 = OFF_SYSCOUNTER0;
const SYSCOUNTERH: u64 = OFF_SYSCOUNTER0 + 4;

fn cc_reg(i: u64, reg: u64) -> u64 {
    OFF_CC0 + i * CC_STRIDE + reg
}

#[test]
fn syscounter_starts_at_zero_and_needs_a_start() {
    let mut g = Nrf54lGrtc::new_fast();
    assert_eq!(g.read_u32(SYSCOUNTERL).unwrap(), 0);
    for _ in 0..10 {
        g.tick();
    }
    assert_eq!(
        g.read_u32(SYSCOUNTERL).unwrap(),
        0,
        "SYSCOUNTER must not advance before it is started"
    );
}

#[test]
fn syscounter_advances_after_tasks_start_and_freezes_on_stop() {
    let mut g = Nrf54lGrtc::new_fast();
    g.write_u32(OFF_TASKS_START, 1).unwrap();
    for _ in 0..10 {
        g.tick();
    }
    assert_eq!(g.read_u32(SYSCOUNTERL).unwrap(), 10);

    g.write_u32(OFF_TASKS_STOP, 1).unwrap();
    for _ in 0..10 {
        g.tick();
    }
    assert_eq!(
        g.read_u32(SYSCOUNTERL).unwrap(),
        10,
        "SYSCOUNTER must freeze while stopped"
    );
}

#[test]
fn mode_syscounteren_also_starts_the_counter() {
    // nrfx starts the SYSCOUNTER through MODE, never TASKS_START.
    let mut g = Nrf54lGrtc::new_fast();
    g.write_u32(OFF_MODE, MODE_SYSCOUNTEREN).unwrap();
    for _ in 0..5 {
        g.tick();
    }
    assert_eq!(g.read_u32(SYSCOUNTERL).unwrap(), 5);
    assert_eq!(g.read_u32(OFF_MODE).unwrap(), MODE_SYSCOUNTEREN);
}

#[test]
fn tasks_clear_zeroes_the_counter() {
    let mut g = Nrf54lGrtc::new_fast();
    g.write_u32(OFF_TASKS_START, 1).unwrap();
    for _ in 0..7 {
        g.tick();
    }
    assert_eq!(g.read_u32(SYSCOUNTERL).unwrap(), 7);
    g.write_u32(OFF_TASKS_CLEAR, 1).unwrap();
    assert_eq!(g.read_u32(SYSCOUNTERL).unwrap(), 0);
}

#[test]
fn low_read_latches_high_across_a_32_bit_rollover() {
    let mut g = Nrf54lGrtc::new_fast();
    g.write_u32(OFF_TASKS_START, 1).unwrap();
    // One tick short of the 32-bit boundary, high word still 0.
    g.set_counter(0xFFFF_FFFF);

    let low = g.read_u32(SYSCOUNTERL).unwrap();
    assert_eq!(low, 0xFFFF_FFFF);

    // The counter rolls into the high word between the paired reads.
    g.tick();
    assert_eq!(g.counter.get(), 0x1_0000_0000);

    let high = g.read_u32(SYSCOUNTERH).unwrap() & SYSCOUNTERH_VALUE_MASK;
    assert_eq!(
        high, 0,
        "SYSCOUNTERH must return the value latched by the SYSCOUNTERL \
             read, otherwise the 52-bit value tears to 0x1_FFFF_FFFF"
    );

    // A fresh pair sees the new high word.
    assert_eq!(g.read_u32(SYSCOUNTERL).unwrap(), 0);
    assert_eq!(g.read_u32(SYSCOUNTERH).unwrap() & SYSCOUNTERH_VALUE_MASK, 1);
}

#[test]
fn syscounter_high_reports_overflow_after_a_rollover() {
    let mut g = Nrf54lGrtc::new_fast();
    g.write_u32(OFF_TASKS_START, 1).unwrap();
    g.set_counter(0xFFFF_FFFF);

    g.read_u32(SYSCOUNTERL).unwrap();
    assert_eq!(
        g.read_u32(SYSCOUNTERH).unwrap() & SYSCOUNTERH_OVERFLOW_BIT,
        0
    );

    g.tick(); // rolls into the high word
    g.read_u32(SYSCOUNTERL).unwrap();
    assert_ne!(
        g.read_u32(SYSCOUNTERH).unwrap() & SYSCOUNTERH_OVERFLOW_BIT,
        0,
        "OVERFLOW must flag a low-word rollover between reads"
    );
}

#[test]
fn syscounter_high_never_reports_busy() {
    let g = Nrf54lGrtc::new();
    assert_eq!(g.read_u32(SYSCOUNTERH).unwrap() & SYSCOUNTERH_BUSY_BIT, 0);
}

#[test]
fn all_four_domain_views_alias_one_counter() {
    let mut g = Nrf54lGrtc::new_fast();
    g.write_u32(OFF_TASKS_START, 1).unwrap();
    for _ in 0..4 {
        g.tick();
    }
    for view in 0..NUM_SYSCOUNTER_VIEWS {
        let off = OFF_SYSCOUNTER0 + view * SYSCOUNTER_STRIDE;
        assert_eq!(g.read_u32(off).unwrap(), 4, "SYSCOUNTER[{view}] view");
    }
}

#[test]
fn compare_fires_event_and_intpend_when_enabled() {
    let mut g = Nrf54lGrtc::new_fast();
    g.write_u32(cc_reg(0, 0x0), 5).unwrap(); // CC[0].CCL = 5
    g.write_u32(cc_reg(0, 0xC), CCEN_ACTIVE).unwrap(); // CC[0].CCEN
    g.write_u32(OFF_INTEN0 + 0x4, 1 << 0).unwrap(); // INTENSET0.COMPARE0
    g.write_u32(OFF_TASKS_START, 1).unwrap();

    let mut irqs = 0;
    for _ in 0..12 {
        if let Some(lines) = g.tick().explicit_irqs {
            // Group 0's compare pends GRTC_0 = irq_base (226 by default).
            assert_eq!(lines, vec![GRTC_IRQ_BASE_DEFAULT]);
            irqs += 1;
        }
    }
    assert_eq!(g.read_u32(OFF_EVENTS_COMPARE0).unwrap(), 1);
    assert_eq!(irqs, 1, "IRQ must be raised once, on the event's 0→1 edge");
    assert_eq!(
        g.read_u32(OFF_INTEN0 + 0xC).unwrap(),
        1 << 0,
        "INTPEND0 must show the enabled, latched compare"
    );
}

#[test]
fn compare_pends_the_group_that_enabled_it() {
    // nrfx on the secure app core enables the kernel-tick compare in INTEN
    // group 2 and expects it on GRTC_2 = irq_base + 2. A compare enabled in
    // group g must pend exactly line irq_base + g — not a collapsed GRTC_0.
    let mut g = Nrf54lGrtc::new_with_cc_and_irq(12, GRTC_IRQ_BASE_DEFAULT);
    // Arm CC[6] the way nrfx does: CCL then CCH (the CCH write arms it).
    g.write_u32(cc_reg(6, 0x0), 5).unwrap();
    g.write_u32(cc_reg(6, 0x4), 0).unwrap();
    // INTENSET2.COMPARE6.
    g.write_u32(OFF_INTEN0 + 2 * INT_GROUP_STRIDE + 0x4, 1 << 6)
        .unwrap();
    g.write_u32(OFF_MODE, MODE_SYSCOUNTEREN).unwrap();

    let mut pended = None;
    for _ in 0..(6 * CYCLES_PER_SYSCOUNTER_TICK) {
        if let Some(lines) = g.tick().explicit_irqs {
            pended = Some(lines);
            break;
        }
    }
    assert_eq!(
        pended,
        Some(vec![GRTC_IRQ_BASE_DEFAULT + 2]),
        "a group-2 compare must pend GRTC_2 = irq_base + 2"
    );
}

#[test]
fn writing_cch_arms_and_writing_ccl_disarms() {
    // The arm bit is a write side effect of CCL/CCH, exactly how nrfx's
    // `nrf_grtc_sys_counter_cc_set` (CCL then CCH, never CCEN) arms the
    // system-timer compare.
    let mut g = Nrf54lGrtc::new_fast();
    g.write_u32(cc_reg(3, 0x0), 4).unwrap(); // CCL → disarmed
    assert_eq!(g.read_u32(cc_reg(3, 0xC)).unwrap(), 0, "CCL write disarms");
    g.write_u32(cc_reg(3, 0x4), 0).unwrap(); // CCH → armed
    assert_eq!(
        g.read_u32(cc_reg(3, 0xC)).unwrap(),
        CCEN_ACTIVE,
        "CCH write arms the channel"
    );
    g.write_u32(OFF_INTEN0 + 0x4, 1 << 3).unwrap();
    g.write_u32(OFF_TASKS_START, 1).unwrap();
    let mut fired = false;
    for _ in 0..8 {
        if g.tick().explicit_irqs.is_some() {
            fired = true;
        }
    }
    assert!(fired, "a CCH-armed compare must fire");
}

#[test]
fn fired_compare_is_one_shot_and_does_not_refire_after_event_clear() {
    // After a compare fires the hardware clears CCEN.ACTIVE, so clearing
    // EVENTS_COMPARE (as the ISR does) must NOT immediately re-fire — that
    // would double-tick the kernel.
    let mut g = Nrf54lGrtc::new_fast();
    g.write_u32(cc_reg(0, 0x0), 3).unwrap();
    g.write_u32(cc_reg(0, 0x4), 0).unwrap(); // arm CC[0] at 3
    g.write_u32(OFF_INTEN0 + 0x4, 1 << 0).unwrap();
    g.write_u32(OFF_TASKS_START, 1).unwrap();

    let mut irqs = 0;
    for _ in 0..5 {
        if g.tick().explicit_irqs.is_some() {
            irqs += 1;
        }
    }
    assert_eq!(irqs, 1);
    assert_eq!(
        g.read_u32(cc_reg(0, 0xC)).unwrap(),
        0,
        "CCEN.ACTIVE must self-clear on fire (one-shot)"
    );
    // ISR clears the event; the counter is still past the stale CC.
    g.write_u32(OFF_EVENTS_COMPARE0, 0).unwrap();
    for _ in 0..5 {
        assert!(
            g.tick().explicit_irqs.is_none(),
            "a one-shot compare must not re-fire after event clear"
        );
    }
}

#[test]
fn compare_does_not_interrupt_when_disabled() {
    let mut g = Nrf54lGrtc::new_fast();
    g.write_u32(cc_reg(0, 0x0), 5).unwrap();
    g.write_u32(cc_reg(0, 0xC), CCEN_ACTIVE).unwrap();
    // INTEN left clear.
    g.write_u32(OFF_TASKS_START, 1).unwrap();

    let mut irqs = 0;
    for _ in 0..12 {
        if g.tick().explicit_irqs.is_some() {
            irqs += 1;
        }
    }
    assert_eq!(irqs, 0, "a masked compare must not raise the IRQ");
    assert_eq!(
        g.read_u32(OFF_EVENTS_COMPARE0).unwrap(),
        1,
        "the event still latches while the interrupt is masked"
    );
    assert_eq!(g.read_u32(OFF_INTEN0 + 0xC).unwrap(), 0, "INTPEND0 empty");
}

#[test]
fn inactive_cc_channel_never_compares() {
    let mut g = Nrf54lGrtc::new_fast();
    g.write_u32(cc_reg(0, 0x0), 3).unwrap();
    // CCEN.ACTIVE left clear.
    g.write_u32(OFF_INTEN0 + 0x4, 1 << 0).unwrap();
    g.write_u32(OFF_TASKS_START, 1).unwrap();
    for _ in 0..10 {
        assert!(g.tick().explicit_irqs.is_none());
    }
    assert_eq!(g.read_u32(OFF_EVENTS_COMPARE0).unwrap(), 0);
}

#[test]
fn compare_uses_the_full_52_bit_value() {
    let mut g = Nrf54lGrtc::new_fast();
    // CC[1] = 0x2_0000_0005 — needs both CCL and CCH.
    g.write_u32(cc_reg(1, 0x0), 5).unwrap();
    g.write_u32(cc_reg(1, 0x4), 2).unwrap();
    g.write_u32(cc_reg(1, 0xC), CCEN_ACTIVE).unwrap();
    g.write_u32(OFF_TASKS_START, 1).unwrap();

    g.set_counter(0x1_FFFF_FFFF);
    g.tick(); // → 0x2_0000_0000, still below the compare
    assert_eq!(g.read_u32(OFF_EVENTS_COMPARE0 + 4).unwrap(), 0);
    g.set_counter(0x2_0000_0004);
    g.tick(); // → 0x2_0000_0005
    assert_eq!(g.read_u32(OFF_EVENTS_COMPARE0 + 4).unwrap(), 1);
}

#[test]
fn events_write_one_ignored_write_zero_clears() {
    let mut g = Nrf54lGrtc::new_fast();
    g.write_u32(OFF_EVENTS_COMPARE0, 1).unwrap();
    assert_eq!(
        g.read_u32(OFF_EVENTS_COMPARE0).unwrap(),
        0,
        "EVENTS_COMPARE write-1 must be a no-op"
    );

    g.write_u32(cc_reg(0, 0xC), CCEN_ACTIVE).unwrap();
    g.write_u32(OFF_TASKS_START, 1).unwrap();
    g.tick();
    assert_eq!(g.read_u32(OFF_EVENTS_COMPARE0).unwrap(), 1);
    g.write_u32(OFF_EVENTS_COMPARE0, 0).unwrap();
    assert_eq!(
        g.read_u32(OFF_EVENTS_COMPARE0).unwrap(),
        0,
        "write-0 must clear the event"
    );
}

#[test]
fn tasks_capture_snapshots_the_counter_into_cc() {
    let mut g = Nrf54lGrtc::new_fast();
    g.write_u32(OFF_TASKS_START, 1).unwrap();
    g.set_counter(0x3_0000_0009);
    g.write_u32(OFF_TASKS_CAPTURE0 + 4 * 2, 1).unwrap(); // TASKS_CAPTURE[2]
    assert_eq!(g.read_u32(cc_reg(2, 0x0)).unwrap(), 0x0000_0009);
    assert_eq!(g.read_u32(cc_reg(2, 0x4)).unwrap(), 0x3);
}

#[test]
fn ccadd_adds_to_the_syscounter_or_the_cc() {
    let mut g = Nrf54lGrtc::new_fast();
    g.set_counter(100);
    // REFERENCE=SYSCOUNTER (bit 31 clear): CC[0] = counter + 50.
    g.write_u32(cc_reg(0, 0x8), 50).unwrap();
    assert_eq!(g.read_u32(cc_reg(0, 0x0)).unwrap(), 150);
    // REFERENCE=CC (bit 31 set): CC[0] += 25.
    g.write_u32(cc_reg(0, 0x8), CCADD_REFERENCE_CC | 25)
        .unwrap();
    assert_eq!(g.read_u32(cc_reg(0, 0x0)).unwrap(), 175);
    // CCADD is write-only.
    assert_eq!(g.read_u32(cc_reg(0, 0x8)).unwrap(), 0);
}

#[test]
fn out_of_range_cc_channel_is_ignored_not_panicking() {
    // A six-channel instance: CC[6..11] and their events are absent.
    let mut g = Nrf54lGrtc::new_with_cc(6);
    g.write_u32(cc_reg(9, 0x0), 0xDEAD_BEEF).unwrap();
    g.write_u32(cc_reg(9, 0x4), 0xF).unwrap();
    g.write_u32(cc_reg(9, 0xC), CCEN_ACTIVE).unwrap();
    assert_eq!(g.read_u32(cc_reg(9, 0x0)).unwrap(), 0);
    assert_eq!(g.read_u32(cc_reg(9, 0xC)).unwrap(), 0);
    assert_eq!(g.read_u32(OFF_EVENTS_COMPARE0 + 4 * 9).unwrap(), 0);
    g.write_u32(OFF_TASKS_CAPTURE0 + 4 * 9, 1).unwrap();
    g.write_u32(OFF_EVENTS_COMPARE0 + 4 * 9, 0).unwrap();

    // The last real channel still works.
    g.write_u32(cc_reg(5, 0x0), 0x1234).unwrap();
    assert_eq!(g.read_u32(cc_reg(5, 0x0)).unwrap(), 0x1234);
}

#[test]
fn inten_masked_to_num_cc() {
    let mut g = Nrf54lGrtc::new(); // 12 channels
    g.write_u32(OFF_INTEN0 + 0x4, 0xFFFF_FFFF).unwrap();
    assert_eq!(g.read_u32(OFF_INTEN0).unwrap(), 0x0000_0FFF);

    let mut g6 = Nrf54lGrtc::new_with_cc(6);
    g6.write_u32(OFF_INTEN0 + 0x4, 0xFFFF_FFFF).unwrap();
    assert_eq!(g6.read_u32(OFF_INTEN0).unwrap(), 0x0000_003F);
}

#[test]
fn intenclr_clears_and_all_four_groups_are_independent() {
    let mut g = Nrf54lGrtc::new();
    for group in 0..NUM_INT_GROUPS as u64 {
        let base = OFF_INTEN0 + group * INT_GROUP_STRIDE;
        g.write_u32(base + 0x4, 1 << group).unwrap();
    }
    for group in 0..NUM_INT_GROUPS as u64 {
        let base = OFF_INTEN0 + group * INT_GROUP_STRIDE;
        assert_eq!(g.read_u32(base).unwrap(), 1 << group);
    }
    g.write_u32(OFF_INTEN0 + 0x8, 1 << 0).unwrap(); // INTENCLR0
    assert_eq!(g.read_u32(OFF_INTEN0).unwrap(), 0);
    assert_eq!(
        g.read_u32(OFF_INTEN0 + INT_GROUP_STRIDE).unwrap(),
        1 << 1,
        "clearing group 0 must not touch group 1"
    );
}

#[test]
fn intpend_is_read_only() {
    let mut g = Nrf54lGrtc::new();
    g.write_u32(OFF_INTEN0 + 0xC, 0xFFFF_FFFF).unwrap();
    assert_eq!(g.read_u32(OFF_INTEN0 + 0xC).unwrap(), 0);
}

#[test]
fn status_registers_read_ready() {
    let g = Nrf54lGrtc::new();
    assert_eq!(g.read_u32(OFF_STATUS_LFTIMER).unwrap(), STATUS_READY);
    assert_eq!(g.read_u32(OFF_STATUS_PWM).unwrap(), STATUS_READY);
    assert_eq!(g.read_u32(OFF_STATUS_CLKOUT).unwrap(), STATUS_READY);
}

#[test]
fn clkcfg_holds_its_reset_value_and_reads_back_writes() {
    let mut g = Nrf54lGrtc::new();
    assert_eq!(g.read_u32(OFF_CLKCFG).unwrap(), CLKCFG_RESET_VALUE);
    g.write_u32(OFF_CLKCFG, 0x0002_0004).unwrap();
    assert_eq!(g.read_u32(OFF_CLKCFG).unwrap(), 0x0002_0004);
}

#[test]
fn syscounter_registers_are_read_only_but_active_is_writable() {
    let mut g = Nrf54lGrtc::new_fast();
    g.write_u32(OFF_TASKS_START, 1).unwrap();
    g.tick();
    g.write_u32(SYSCOUNTERL, 0xFFFF_FFFF).unwrap();
    g.write_u32(SYSCOUNTERH, 0xFFFF_FFFF).unwrap();
    assert_eq!(g.read_u32(SYSCOUNTERL).unwrap(), 1);

    g.write_u32(OFF_SYSCOUNTER0 + 0x8, 1).unwrap();
    assert_eq!(g.read_u32(OFF_SYSCOUNTER0 + 0x8).unwrap(), 1);
}

#[test]
fn unimplemented_offsets_read_zero_without_faulting() {
    let g = Nrf54lGrtc::new();
    for off in [
        0x030u64,
        0x0C0,
        0x160,
        0x1F0,
        0x204,
        0x400 + 0x40,
        0x500,
        0xFFC,
    ] {
        assert_eq!(g.read_u32(off).unwrap(), 0, "offset {off:#05x}");
    }
}

#[test]
fn tick_elapsed_matches_repeated_ticks() {
    let mut repeated = Nrf54lGrtc::new();
    let mut elapsed = Nrf54lGrtc::new();
    repeated.write_u32(OFF_TASKS_START, 1).unwrap();
    elapsed.write_u32(OFF_TASKS_START, 1).unwrap();

    let cycles = 3 * CYCLES_PER_SYSCOUNTER_TICK as u64 + 7;
    for _ in 0..cycles {
        repeated.tick();
    }
    elapsed.tick_elapsed(cycles);

    assert_eq!(repeated.read_u32(SYSCOUNTERL).unwrap(), 3);
    assert_eq!(
        elapsed.read_u32(SYSCOUNTERL).unwrap(),
        repeated.read_u32(SYSCOUNTERL).unwrap()
    );
}

#[test]
fn syscounter_runs_at_one_megahertz_against_the_cpu_clock() {
    // 128 CPU cycles at 128 MHz is exactly 1 µs, i.e. one SYSCOUNTER tick.
    let mut g = Nrf54lGrtc::new();
    g.write_u32(OFF_TASKS_START, 1).unwrap();
    g.tick_elapsed(CPU_HZ_DEFAULT as u64 / 1000); // 1 ms of CPU cycles
    assert_eq!(
        g.read_u32(SYSCOUNTERL).unwrap(),
        SYSCOUNTER_HZ / 1000,
        "1 ms of CPU time must be 1000 SYSCOUNTER ticks"
    );
}

#[test]
fn legacy_tick_charges_zero_cost() {
    // Tick-cost normalization: a free-running counter consumes no core
    // cycles, so the walk must charge zero (the scheduler path can't
    // reproduce a per-tick cost, and total_cycles must agree across modes).
    let mut g = Nrf54lGrtc::new();
    g.write_u32(OFF_TASKS_START, 1).unwrap();
    assert_eq!(g.tick().cycles, 0);
    assert_eq!(g.tick_elapsed(256).cycles, 0);
}

#[test]
fn without_clock_stays_on_legacy_tick_path() {
    let g = Nrf54lGrtc::new();
    assert!(
        !g.uses_scheduler(),
        "no cycle clock attached → the model must stay on the legacy walk"
    );
    assert!(g.needs_legacy_walk());
}

#[cfg(feature = "event-scheduler")]
mod scheduler_mode {
    use super::*;

    fn armed_scheduler() -> (Nrf54lGrtc, CycleClock) {
        let clock = CycleClock::default();
        let mut g = Nrf54lGrtc::new_fast(); // 1 CPU cycle per SYSCOUNTER tick
        g.attach_cycle_clock(clock.clone());
        (g, clock)
    }

    #[test]
    fn clock_attach_flips_to_scheduler_and_walk_tick_is_inert() {
        let (mut g, _clock) = armed_scheduler();
        assert!(g.uses_scheduler(), "clock attached → walk-independent");
        assert!(!g.needs_legacy_walk());
        g.write_u32(OFF_MODE, MODE_SYSCOUNTEREN).unwrap();
        // A stray walk tick must not double-count against the lazy anchor.
        let r = g.tick();
        assert!(r.explicit_irqs.is_none());
        assert_eq!(
            g.read_u32(SYSCOUNTERL).unwrap(),
            0,
            "tick inert in scheduler mode; counter derives from the clock"
        );
    }

    #[test]
    fn lazy_syscounter_read_tracks_published_clock_exactly() {
        let (mut g, clock) = armed_scheduler();
        g.write_u32(OFF_MODE, MODE_SYSCOUNTEREN).unwrap();
        g.sync_to(0);
        clock.publish(5);
        assert_eq!(g.read_u32(SYSCOUNTERL).unwrap(), 5);
        clock.publish(9);
        assert_eq!(g.read_u32(SYSCOUNTERL).unwrap(), 9);
    }

    #[test]
    fn arming_write_schedules_the_exact_compare_deadline() {
        let (mut g, _clock) = armed_scheduler();
        g.write_u32(OFF_MODE, MODE_SYSCOUNTEREN).unwrap();
        g.sync_to(0);
        // CC[0] = 100; from counter 0 the fire lands 100 ticks after the
        // synced state; the bus adds current_cycle + 1, so the peripheral
        // hands out d - 1 = 99.
        g.write_u32(cc_reg(0, 0x0), 100).unwrap();
        g.write_u32(cc_reg(0, 0x4), 0).unwrap(); // CCH arms
        let evs = g.take_scheduled_events();
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].0, 99, "delay must be cycles-to-fire minus one");
    }

    #[test]
    fn on_event_pends_the_group_irq_and_reschedules() {
        let (mut g, clock) = armed_scheduler();
        g.write_u32(OFF_MODE, MODE_SYSCOUNTEREN).unwrap();
        g.sync_to(0);
        g.write_u32(cc_reg(0, 0x0), 100).unwrap();
        g.write_u32(cc_reg(0, 0x4), 0).unwrap();
        g.write_u32(OFF_INTEN0 + 0x4, 1 << 0).unwrap(); // group 0 enables COMPARE0
        let token = g.take_scheduled_events()[0].1;

        clock.publish(100); // drain at the exact fire cycle
        let mut sched = crate::sched::EventScheduler::new();
        sched.advance_to(100);
        let mut bus = crate::bus::SystemBus::new();
        let res = g.on_event(token, &mut sched, &mut bus);
        assert_eq!(
            res.explicit_irqs,
            vec![GRTC_IRQ_BASE_DEFAULT],
            "group-0 compare pends GRTC_0"
        );
        assert_eq!(res.fired_events, vec![OFF_EVENTS_COMPARE0 as u32]);
        assert_eq!(g.read_u32(OFF_EVENTS_COMPARE0).unwrap(), 1);
        // One-shot: no further compare armed → nothing to reschedule.
        assert_eq!(res.reschedule_delay, None);

        // A second drain at the same cycle claims nothing.
        let res2 = g.on_event(token, &mut sched, &mut bus);
        assert!(res2.explicit_irqs.is_empty(), "no double-claim");
    }

    #[test]
    fn stale_event_chain_dies_on_token_mismatch() {
        let (mut g, clock) = armed_scheduler();
        g.write_u32(OFF_MODE, MODE_SYSCOUNTEREN).unwrap();
        g.sync_to(0);
        g.write_u32(cc_reg(0, 0x0), 100).unwrap();
        g.write_u32(cc_reg(0, 0x4), 0).unwrap();
        let old_token = g.take_scheduled_events()[0].1;
        // Re-arm (kills the old chain).
        g.write_u32(cc_reg(0, 0x0), 50).unwrap();
        g.write_u32(cc_reg(0, 0x4), 0).unwrap();
        let new_token = g.take_scheduled_events()[0].1;
        assert_ne!(old_token, new_token);

        clock.publish(500);
        let mut sched = crate::sched::EventScheduler::new();
        sched.advance_to(500);
        let mut bus = crate::bus::SystemBus::new();
        let res = g.on_event(old_token, &mut sched, &mut bus);
        assert!(res.explicit_irqs.is_empty(), "stale chain must be inert");
        assert_eq!(res.reschedule_delay, None, "stale chain must not respawn");
    }

    #[test]
    fn masked_compare_still_latches_but_schedules_no_irq() {
        let (mut g, clock) = armed_scheduler();
        g.write_u32(OFF_MODE, MODE_SYSCOUNTEREN).unwrap();
        g.sync_to(0);
        g.write_u32(cc_reg(0, 0x0), 10).unwrap();
        g.write_u32(cc_reg(0, 0x4), 0).unwrap(); // armed, INTEN clear
        let token = g.take_scheduled_events()[0].1;

        clock.publish(10);
        let mut sched = crate::sched::EventScheduler::new();
        sched.advance_to(10);
        let mut bus = crate::bus::SystemBus::new();
        let res = g.on_event(token, &mut sched, &mut bus);
        assert!(
            res.explicit_irqs.is_empty(),
            "a masked compare must not pend an IRQ"
        );
        assert_eq!(
            g.read_u32(OFF_EVENTS_COMPARE0).unwrap(),
            1,
            "the event still latches while the interrupt is masked"
        );
    }

    #[test]
    fn cc0_interval_reload_reschedules_the_next_period() {
        let (mut g, clock) = armed_scheduler();
        g.write_u32(OFF_MODE, MODE_SYSCOUNTEREN).unwrap();
        g.write_u32(OFF_INTERVAL, 20).unwrap();
        g.sync_to(0);
        g.write_u32(cc_reg(0, 0x0), 20).unwrap();
        g.write_u32(cc_reg(0, 0x4), 0).unwrap();
        g.write_u32(OFF_INTEN0 + 0x4, 1 << 0).unwrap();
        let token = g.take_scheduled_events()[0].1;

        clock.publish(20);
        let mut sched = crate::sched::EventScheduler::new();
        sched.advance_to(20);
        let mut bus = crate::bus::SystemBus::new();
        let res = g.on_event(token, &mut sched, &mut bus);
        assert_eq!(res.explicit_irqs, vec![GRTC_IRQ_BASE_DEFAULT]);
        // CC0 auto-reloads by INTERVAL and stays armed (deadline → 40).
        assert_eq!(g.read_u32(cc_reg(0, 0x0)).unwrap(), 40);
        assert_eq!(g.read_u32(cc_reg(0, 0xC)).unwrap(), CCEN_ACTIVE);
        // But EVENTS_COMPARE[0] is still latched, so — exactly like the
        // legacy walk's `&& events_compare == 0` guard — the channel cannot
        // re-fire and nothing is rescheduled until the ISR clears it.
        assert_eq!(res.reschedule_delay, None);

        // ISR clears the event; the arming-recompute (which the bus runs on
        // that write) now schedules the next period at 40 (20 ticks away).
        g.write_u32(OFF_EVENTS_COMPARE0, 0).unwrap();
        g.sync_to(20);
        let next = g.take_scheduled_events();
        assert_eq!(
            next.first().map(|e| e.0),
            Some(19),
            "next period at cycle 40"
        );
    }
}
