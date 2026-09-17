use super::*;

/// Silicon capture 2026-08-02: the whole window reads `00000000` at
/// `reset halt` (clock-gated), so an untouched model must too — everywhere
/// except the two hardwired identity words, which are deliberately always
/// readable (see the note in `read_u32`).
#[test]
fn gated_window_reads_zero() {
    let bt = Esp32c3Bt::new();
    for off in [0x000u64, 0x01C, 0x020, 0x024, 0x204, 0x2C4, 0x370, 0x530] {
        assert_eq!(bt.read_u32(off).unwrap(), 0, "offset {off:#05x} at reset");
    }
}

/// `RWBLECNTL` bit31 is a self-clearing command bit: the controller writes
/// the control word, then writes it again with bit31 set as a kick, and
/// spins until the bit reads back clear. Silicon reads `+0x000` as
/// `0x0010_070f` (bit31 clear) on a live part whose last write was
/// `0x8010_070f`. Regression for the stall that pinned the twin on the
/// instruction after that store.
#[test]
fn rwblecntl_command_bit_self_clears() {
    let mut bt = Esp32c3Bt::new();
    bt.write_u32(RWBLECNTL, 0x0010_070f).unwrap();
    assert_eq!(bt.read_u32(RWBLECNTL).unwrap(), 0x0010_070f);
    bt.write_u32(RWBLECNTL, 0x8010_070f).unwrap();
    assert_eq!(
        bt.read_u32(RWBLECNTL).unwrap(),
        0x0010_070f,
        "bit31 must be consumed, not stored — otherwise the controller \
             spins forever waiting for its own kick to clear"
    );
}

/// **The two abort REQUEST bits read back clear, like bit31.** Silicon
/// reads `+0x000 = 0x0010_070f` on a live advertising part whose own
/// firmware writes bit 25 (`r_lld_adv_stop` `0x4001_8A2C`,
/// `r_lld_per_adv_stop` `0x4002_3E48`, `r_lld_rpa_renew_evt_start_cbk`
/// `0x4001_FF06`) and bit 24 (`r_lld_scan_end` `0x4002_4634`,
/// `r_lld_rpa_renew_evt_start_cbk` `0x4001_FF18`). No ROM site ever writes
/// a zero into either, and none reads either back, so nothing in software
/// could clear them.
///
/// The values here are the exact ones off the two sides: what the C3's
/// firmware stores, and what OpenOCD reads back afterwards.
#[test]
fn rwblecntl_abort_requests_read_back_clear() {
    for request in [0x0210_070fu32, 0x0110_070f, 0x0310_070f] {
        let mut bt = Esp32c3Bt::new();
        bt.write_u32(RWBLECNTL, 0x0010_070f).unwrap();
        bt.write_u32(RWBLECNTL, request).unwrap();
        assert_eq!(
            bt.read_u32(RWBLECNTL).unwrap(),
            0x0010_070f,
            "wrote {request:#010x}; every live dump of an advertising C3 \
                 reads +0x000 back as 0x0010_070f, abort requests consumed"
        );
    }
}

/// **The firmware's OWN next control word proves it**, which is what makes
/// this a silicon check rather than a restatement of the line above.
///
/// `r_lld_rpa_renew_evt_start_cbk` (`0x4001_FEE0`) aborts both activities
/// back to back, and the second store is computed from what the first one
/// read BACK:
///
/// ```text
/// 4001ff06: lw   a5,0(a4)      ; read RWBLECNTL
/// 4001ff0e: and  a5,a5,a3      ; a3 = 0xfdffffff
/// 4001ff14: or   a5,a5,a3      ; a3 = 0x02000000   -> set bit 25
/// 4001ff16: sw   a5,0(a4)
/// 4001ff18: lw   a5,0(a4)      ; read it BACK
/// 4001ff20: and  a5,a5,a3      ; a3 = 0xfeffffff
/// 4001ff26: or   a5,a5,a3      ; a3 = 0x01000000   -> set bit 24
/// 4001ff28: sw   a5,0(a4)
/// ```
///
/// So the second word is `(read_back & 0xFEFF_FFFF) | 0x0100_0000`. On
/// silicon the read-back has bit 25 already gone, so that is
/// `0x0110_070f`. A model that latches the request makes the same
/// firmware compute `0x0310_070f` — which is exactly what the twin's
/// trace showed at CLKN 37 on both BLE Pong nodes, and a word the real
/// part can never produce.
#[test]
fn the_rpa_renew_sequence_computes_the_silicon_control_word() {
    let mut bt = Esp32c3Bt::new();
    bt.write_u32(RWBLECNTL, 0x0010_070f).unwrap();

    // Replay `r_lld_rpa_renew_evt_start_cbk` instruction for instruction.
    let first = (bt.read_u32(RWBLECNTL).unwrap() & 0xFDFF_FFFF) | 0x0200_0000;
    assert_eq!(first, 0x0210_070f, "the abort-advertising store");
    bt.write_u32(RWBLECNTL, first).unwrap();

    let second = (bt.read_u32(RWBLECNTL).unwrap() & 0xFEFF_FFFF) | 0x0100_0000;
    assert_eq!(
        second, 0x0110_070f,
        "the abort-scanning store the firmware computes from its own \
             read-back. 0x0310_070f means bit 25 was still there to be read, \
             which is the twin latching a request the core consumes"
    );
}

/// The controller validates `VERSION` during `lld` bring-up and asserts on
/// a mismatch, quoting the value it wants:
/// `assert lld.c 318, param 00000000 09001b00`. Regression for that stop.
#[test]
fn hardware_identity_words_read_their_silicon_values() {
    let mut bt = Esp32c3Bt::new();
    assert_eq!(bt.read_u32(0x004).unwrap(), 0x0900_1b00, "VERSION");
    assert_eq!(bt.read_u32(0x008).unwrap(), 0x0f22_d0b0, "RWBLECONF");
    // Read-only: a stray write must not be able to break the assert.
    bt.write_u32(0x004, 0xdead_beef).unwrap();
    bt.write_u32(0x008, 0xdead_beef).unwrap();
    assert_eq!(bt.read_u32(0x004).unwrap(), 0x0900_1b00, "VERSION is RO");
    assert_eq!(bt.read_u32(0x008).unwrap(), 0x0f22_d0b0, "RWBLECONF is RO");
}

/// The register-backed majority: BLE bring-up is read-modify-write, so a
/// written value must read straight back. Values are real ones from the
/// silicon write trace.
#[test]
fn window_is_register_backed() {
    let mut bt = Esp32c3Bt::new();
    for (off, val) in [
        (0x204u64, 0x0002_9725u32), // ROM patch/veneer table entry 0
        (0x2c4, 0x07fe_01ff),       // patch-enable mask, fully populated
        (0x0e0, 0x0190_012c),       // advertising interval pair
        (0x530, 0x0000_0001),
    ] {
        bt.write_u32(off, val).unwrap();
        assert_eq!(bt.read_u32(off).unwrap(), val, "offset {off:#05x}");
    }
}

/// `CLKN` is read/write asymmetric: the BT ROM writes a comparator target
/// with the arm bit and immediately re-reads the *clock*. If the write were
/// stored the scheduler would read its own deadline back as "now" and
/// re-arm the same instant forever.
#[test]
fn clkn_write_arms_comparator_and_does_not_shadow_the_clock() {
    let mut bt = Esp32c3Bt::new();
    let clock = CycleClock::default();
    bt.attach_cycle_clock(clock.clone());
    clock.publish(0);
    bt.write_u32(CLKN, 0x8000_e8f7).unwrap(); // a real traced value
    assert_eq!(bt.armed_event_target(), Some(0x0000_e8f7));
    assert_eq!(
        bt.read_u32(CLKN).unwrap(),
        0,
        "CLKN read must be the clock, not the armed target"
    );
    // A write without the arm bit disarms.
    bt.write_u32(CLKN, 0x0000_1234).unwrap();
    assert_eq!(bt.armed_event_target(), None);
}

/// CLKN advances at the Bluetooth native rate (312.5 µs / 3200 Hz), and the
/// fine counter wraps `0..=624` once per CLKN tick.
#[test]
fn timebase_advances_at_the_bluetooth_native_rate() {
    let mut bt = Esp32c3Bt::new();
    let clock = CycleClock::default();
    bt.attach_cycle_clock(clock.clone());
    clock.publish(1_000); // un-gate at a non-zero cycle
    bt.write_u32(0x000, 0x0010_060f).unwrap();
    assert_eq!(bt.read_u32(CLKN).unwrap(), 0, "CLKN starts at the un-gate");

    // One second of device time at 160 MHz = 3200 CLKN ticks.
    clock.publish(1_000 + 160_000_000);
    assert_eq!(bt.read_u32(CLKN).unwrap(), 3200);

    // Fine counter: half-µs ticks, counting DOWN 624 -> 0 once per CLKN
    // tick (the direction `r_rwip_time_get`'s `624 - FINETIMECNT` and
    // `r_rwip_timer_hus_set`'s `624 - hus` both require).
    clock.publish(1_000);
    assert_eq!(bt.read_u32(CLKN_FINE).unwrap(), 624, "starts full");
    clock.publish(1_000 + CYCLES_PER_FINE_TICK * (624 - 566));
    assert_eq!(bt.read_u32(CLKN_FINE).unwrap(), 566); // max value seen on silicon
    assert_eq!(bt.read_u32(CLKN).unwrap(), 0, "still inside the first tick");
    clock.publish(1_000 + CYCLES_PER_CLKN_TICK);
    assert_eq!(bt.read_u32(CLKN_FINE).unwrap(), 624, "reloads with CLKN");
    assert_eq!(bt.read_u32(CLKN).unwrap(), 1);

    // Never leaves the range silicon showed.
    for n in 0..2_000u64 {
        clock.publish(1_000 + n * 137);
        assert!(bt.read_u32(CLKN_FINE).unwrap() < FINE_TICKS_PER_CLKN as u32);
    }
}

/// Bring a model up to the point a live advertising part is at: block
/// un-gated, the enable word silicon reads, and the hus comparator armed
/// the way `r_rwip_timer_hus_set` arms it.
fn advertising_part(clock: &CycleClock) -> Esp32c3Bt {
    let mut bt = Esp32c3Bt::new();
    bt.attach_cycle_clock(clock.clone());
    clock.publish(0);
    bt.write_u32(RWBLECNTL, 0x0010_070f).unwrap(); // un-gate
    bt.write_u32(INTCNTL, 0x0064_0b66).unwrap(); // silicon enable word
    bt
}

/// Silicon capture 2026-08-02, board `38:44:be:42:f5:58`: `INTSTAT` is
/// `INTRAWSTAT & INTCNTL`, not a stored register. Both measured pairs.
#[test]
fn int_status_is_raw_and_enable() {
    let clock = CycleClock::default();
    let mut bt = advertising_part(&clock);
    assert_eq!(bt.read_u32(INTCNTL).unwrap(), 0x0064_0b66);

    bt.int_raw.set(0x0000_0011); // measured raw at one halt
    assert_eq!(bt.read_u32(INTSTAT).unwrap(), 0x0000_0000);
    bt.int_raw.set(0x0000_0811); // measured raw at three later halts
    assert_eq!(bt.read_u32(INTSTAT).unwrap(), 0x0000_0800);

    // W1C through INTACK, which itself reads back 0.
    bt.write_u32(INTACK, 0x0000_0800).unwrap();
    assert_eq!(bt.read_u32(INTRAWSTAT).unwrap(), 0x0000_0011);
    assert_eq!(bt.read_u32(INTSTAT).unwrap(), 0);
    assert_eq!(bt.read_u32(INTACK).unwrap(), 0, "INTACK reads 0 on silicon");
    // `+0x38C` is the mirror the ROM writes alongside INTACK.
    bt.int_raw.set(0x0000_0800);
    bt.write_u32(INTACK_FIFO, 0x0000_0800).unwrap();
    assert_eq!(bt.read_u32(INTRAWSTAT).unwrap(), 0);
    assert_eq!(bt.read_u32(INTACK_FIFO).unwrap(), 0);
}

/// The half-µs comparator is what drives advertising: `r_rwip_timer_hus_set`
/// writes the base-time target to `+0x0EC`, the fine target to `+0x0F0`,
/// acks bit 11 and ORs `0x800` into `INTCNTL`. On the live part `+0x0EC`
/// sat 119–150 CLKN ticks ahead of `+0x01C` every time it was sampled.
/// When the timebase gets there the model must raise `INTSTAT` bit 11 and
/// assert the RW-BLE matrix source — the whole point of this milestone.
#[test]
fn hus_comparator_raises_rwble_irq_at_its_target() {
    let clock = CycleClock::default();
    let mut bt = advertising_part(&clock);

    // Arm 130 CLKN ticks out, exactly as the ROM does.
    bt.write_u32(TIMER_HUS_TARGET, 130).unwrap();
    bt.write_u32(TIMER_HUS_FINE, 624).unwrap();
    bt.write_u32(INTACK, INT_TIMER_HUS).unwrap();

    // Not yet: one tick short, the line stays down.
    clock.publish(129 * CYCLES_PER_CLKN_TICK);
    assert!(bt.tick().explicit_irqs.is_none());
    assert_eq!(bt.read_u32(INTSTAT).unwrap(), 0);
    assert!(bt.matrix_irq_sources().is_empty());

    // At the target: raw latches, INTSTAT shows it, matrix source 8 up.
    clock.publish(130 * CYCLES_PER_CLKN_TICK);
    assert_eq!(
        bt.tick().explicit_irqs,
        Some(vec![RWBLE_IRQ_SOURCE]),
        "the hus comparator must raise the RWBLE matrix source"
    );
    assert_eq!(bt.read_u32(INTRAWSTAT).unwrap(), INT_TIMER_HUS);
    assert_eq!(bt.read_u32(INTSTAT).unwrap(), INT_TIMER_HUS);
    assert_eq!(bt.matrix_irq_sources(), vec![RWBLE_IRQ_SOURCE]);

    // Level-sensitive: it stays up until firmware W1C-acks, exactly like
    // the WiFi MAC's event level.
    clock.publish(131 * CYCLES_PER_CLKN_TICK);
    assert_eq!(bt.tick().explicit_irqs, Some(vec![RWBLE_IRQ_SOURCE]));
    bt.write_u32(INTACK, INT_TIMER_HUS).unwrap();
    assert!(bt.matrix_irq_sources().is_empty());
    assert!(bt.tick().explicit_irqs.is_none());

    // Re-arming pushes the deadline out again — the advertising cadence.
    bt.write_u32(TIMER_HUS_TARGET, 280).unwrap();
    clock.publish(279 * CYCLES_PER_CLKN_TICK);
    assert!(bt.tick().explicit_irqs.is_none());
    clock.publish(280 * CYCLES_PER_CLKN_TICK);
    assert_eq!(bt.tick().explicit_irqs, Some(vec![RWBLE_IRQ_SOURCE]));
}

/// A comparator runs only while its `INTCNTL` enable is set. Silicon
/// attests it: `+0x0E8` held a long-past `0x91` while CLKN was `0x462F`
/// with `INTCNTL` bit 10 clear, and `INTRAWSTAT` bit 10 read 0. Modelling
/// it the other way would raise a phantom interrupt out of a stale target.
#[test]
fn a_masked_comparator_does_not_latch() {
    let clock = CycleClock::default();
    let mut bt = advertising_part(&clock);
    assert_eq!(
        bt.read_u32(INTCNTL).unwrap() & INT_TIMER_HS,
        0,
        "the silicon enable word leaves the hs timer disarmed"
    );
    bt.write_u32(TIMER_HS_TARGET, 0x91).unwrap();
    clock.publish(0x462f * CYCLES_PER_CLKN_TICK);
    bt.tick();
    assert_eq!(
        bt.read_u32(INTRAWSTAT).unwrap() & INT_TIMER_HS,
        0,
        "a stale target behind a clear enable must not latch"
    );
    // Arm it (INTCNTL |= 0x400, as `r_rwip_timer_hs_set` does) and the same
    // stale target fires at once — a missed deadline, not a 23-hour wrap.
    bt.write_u32(INTCNTL, 0x0064_0b66 | INT_TIMER_HS).unwrap();
    assert_eq!(bt.tick().explicit_irqs, Some(vec![RWBLE_IRQ_SOURCE]));
    assert_eq!(
        bt.read_u32(INTRAWSTAT).unwrap() & INT_TIMER_HS,
        INT_TIMER_HS
    );
}

/// The 10 ms comparator counts in units of 32 CLKN ticks —
/// `r_rwip_timer_10ms_set` writes the target to `+0x0E4` and keeps
/// `rwip_env+8 = target << 5` (half-slots) alongside it.
#[test]
fn ten_ms_comparator_counts_in_32_clkn_units() {
    let clock = CycleClock::default();
    let mut bt = advertising_part(&clock);
    assert_ne!(bt.read_u32(INTCNTL).unwrap() & INT_TIMER_10MS, 0);
    bt.write_u32(TIMER_10MS_TARGET, 100).unwrap(); // 1 s = 3200 CLKN ticks
    clock.publish(3199 * CYCLES_PER_CLKN_TICK);
    assert!(bt.tick().explicit_irqs.is_none());
    clock.publish(3200 * CYCLES_PER_CLKN_TICK);
    assert_eq!(bt.tick().explicit_irqs, Some(vec![RWBLE_IRQ_SOURCE]));
    assert_eq!(
        bt.read_u32(INTSTAT).unwrap() & INT_TIMER_10MS,
        INT_TIMER_10MS
    );
}

/// The IRQ FIFO at `+0x2D8`. `sdk_cfg_priv_opts[69]` reads 1 on this
/// silicon, so `r_rwble_isr` dispatches from here rather than from a raw
/// `INTSTAT` read — and returns WITHOUT acking when `cnt == 0`, which
/// would turn a raised level into an interrupt storm. Silicon read
/// `+0x2D8 = 0x0020_003E` with `INTSTAT = 0x800`: cnt 1, rem 15,
/// bitmap `0x800`.
#[test]
fn irq_fifo_matches_the_silicon_word() {
    let clock = CycleClock::default();
    let mut bt = advertising_part(&clock);
    assert_eq!(
        bt.read_u32(IRQ_FIFO).unwrap(),
        0x0000_001E,
        "empty FIFO: the exact word silicon reads while idle (cnt 0, rem 15)"
    );

    bt.write_u32(TIMER_HUS_TARGET, 130).unwrap();
    bt.write_u32(TIMER_HUS_FINE, 624).unwrap();
    clock.publish(130 * CYCLES_PER_CLKN_TICK);
    bt.tick();
    assert_eq!(
        bt.read_u32(IRQ_FIFO).unwrap(),
        0x0020_003E,
        "one queued hus interrupt must read back the exact silicon word"
    );

    // `ori a5,a5,1; sw` pops the head.
    bt.write_u32(IRQ_FIFO, 0x0020_003F).unwrap();
    assert_eq!(bt.read_u32(IRQ_FIFO).unwrap() >> 5 & 31, 0, "cnt back to 0");
    assert_eq!(bt.read_u32(IRQ_FIFO).unwrap() >> 10, 0, "no head bitmap");
    // Popping the FIFO is NOT the ack: the raw latch is separate, and the
    // ISR clears it through `+0x018`.
    assert_eq!(bt.read_u32(INTRAWSTAT).unwrap(), INT_TIMER_HUS);
}

/// One entry per rising edge, capped at the 16-deep FIFO — never a
/// re-queue while the same bit is still latched, which would let one
/// unacked interrupt flood the queue.
#[test]
fn irq_fifo_queues_one_entry_per_rising_edge() {
    let clock = CycleClock::default();
    let mut bt = advertising_part(&clock);
    bt.write_u32(TIMER_HUS_TARGET, 10).unwrap();
    bt.write_u32(TIMER_HUS_FINE, 624).unwrap();
    for n in 10..40u64 {
        clock.publish(n * CYCLES_PER_CLKN_TICK);
        bt.tick();
    }
    assert_eq!(bt.irq_fifo.borrow().len(), 1, "still one unacked interrupt");
}

/// The walk must not run while there is nothing scheduled, and must run
/// the moment a comparator is armed or the line is up.
#[test]
fn walk_membership_follows_the_comparators() {
    let clock = CycleClock::default();
    let mut bt = Esp32c3Bt::new();
    bt.attach_cycle_clock(clock.clone());
    clock.publish(0);
    // Walk membership is only claimed in legacy mode; under the scheduler
    // the comparators ride events instead (and the C3 walk-pinner ledger
    // requires that).
    assert_eq!(bt.needs_legacy_walk(), !bt.uses_scheduler());
    assert!(bt.legacy_tick_dynamic());
    assert!(!bt.legacy_tick_active(), "gated block has nothing to tick");
    bt.write_u32(RWBLECNTL, 0x0010_070f).unwrap();
    assert!(!bt.legacy_tick_active(), "un-gated but nothing armed");
    bt.write_u32(INTCNTL, INT_TIMER_HUS).unwrap();
    assert!(
        !bt.legacy_tick_active(),
        "an enable over an unprogrammed target is not an armed comparator"
    );
    bt.write_u32(TIMER_HUS_TARGET, 130).unwrap();
    assert!(bt.legacy_tick_active(), "hus comparator armed");
}

/// Scheduler mode: the hus comparator must arrive as a scheduled event at
/// its exact cycle, with no per-cycle walk and no firmware poll — the same
/// contract `ledc` holds for `LSTIMERx_OVF`. This is what keeps the model
/// off the C3 walk-pinner ledger.
#[cfg(feature = "event-scheduler")]
#[test]
fn scheduled_event_delivers_the_comparator_without_a_walk() {
    use crate::sched::EventScheduler;

    let clock = CycleClock::default();
    let mut bt = advertising_part(&clock);
    assert!(bt.uses_scheduler(), "a clocked model is scheduler-driven");
    assert!(!bt.needs_legacy_walk(), "and must not pin the walk");

    // Arm 130 CLKN ticks out, exactly as `r_rwip_timer_hus_set` does.
    bt.write_u32(TIMER_HUS_TARGET, 130).unwrap();
    bt.write_u32(TIMER_HUS_FINE, 624).unwrap();
    bt.write_u32(INTACK, INT_TIMER_HUS).unwrap();
    let events = bt.take_scheduled_events();
    assert_eq!(events.len(), 1, "one in-flight comparator event");
    let (delay, token) = events[0];
    assert_eq!(
        delay,
        130 * CYCLES_PER_CLKN_TICK - 1,
        "the bus adds the +1 anchor offset back"
    );

    let mut sched = EventScheduler::new();
    let mut bus = crate::bus::SystemBus::new();
    sched.advance_to(130 * CYCLES_PER_CLKN_TICK);
    clock.publish(130 * CYCLES_PER_CLKN_TICK);
    let res = bt.on_event(token, &mut sched, &mut bus);
    assert_eq!(bt.read_u32(INTSTAT).unwrap(), INT_TIMER_HUS);
    assert_eq!(bt.matrix_irq_sources(), vec![RWBLE_IRQ_SOURCE]);
    assert!(
        res.reschedule_delay.is_none(),
        "nothing else armed, so the chain stops until firmware re-arms"
    );

    // A stale generation must not fire anything. The clock stays behind
    // the new deadline so the lazy read-path latch cannot mask the check.
    bt.write_u32(INTACK, INT_TIMER_HUS).unwrap();
    bt.write_u32(TIMER_HUS_TARGET, 300).unwrap();
    let fresh = bt.take_scheduled_events()[0].1;
    assert_ne!(token, fresh, "re-arming stamps a fresh generation");
    sched.advance_to(300 * CYCLES_PER_CLKN_TICK);
    bt.on_event(token, &mut sched, &mut bus);
    assert_eq!(bt.read_u32(INTSTAT).unwrap(), 0, "stale token is inert");
    bt.on_event(fresh, &mut sched, &mut bus);
    assert_eq!(bt.read_u32(INTSTAT).unwrap(), INT_TIMER_HUS);
}

// ── Programmed radio events ─────────────────────────────────────────────

/// A flat byte-addressed memory standing in for the C3's data RAM, so the
/// radio engine can be driven without a whole `Machine`.
#[derive(Default)]
// Fixture for the event-scheduler tests only; dead without the feature.
#[cfg(feature = "event-scheduler")]
struct RamBus {
    ram: std::collections::HashMap<u64, u8>,
    cfg: crate::SimulationConfig,
}

// Fixture for the event-scheduler tests only; dead without the feature.
#[cfg(feature = "event-scheduler")]
impl RamBus {
    fn put(&mut self, addr: u64, bytes: &[u8]) {
        for (i, b) in bytes.iter().enumerate() {
            self.ram.insert(addr + i as u64, *b);
        }
    }
    fn u16_at(&self, addr: u64) -> u16 {
        u16::from(*self.ram.get(&addr).unwrap_or(&0))
            | (u16::from(*self.ram.get(&(addr + 1)).unwrap_or(&0)) << 8)
    }
}

#[cfg(feature = "event-scheduler")]
impl crate::Bus for RamBus {
    fn read_u8(&self, addr: u64) -> SimResult<u8> {
        Ok(*self.ram.get(&addr).unwrap_or(&0))
    }
    fn write_u8(&mut self, addr: u64, value: u8) -> SimResult<()> {
        self.ram.insert(addr, value);
        Ok(())
    }
    fn tick_peripherals(&mut self) -> Vec<u32> {
        Vec::new()
    }
    fn execute_dma(&mut self, _requests: &[crate::DmaRequest]) -> SimResult<()> {
        Ok(())
    }
    fn config(&self) -> &crate::SimulationConfig {
        &self.cfg
    }
}

/// Base CPU address the fixture maps exchange memory at — the same
/// `0x3FC0_0000` data-RAM window `r_emi_get_mem_addr_by_offset` resolves
/// into. Offset chosen to match the live part's `0x3FCA_5C94`.
// Fixture for the event-scheduler tests only; dead without the feature.
#[cfg(feature = "event-scheduler")]
const FIXTURE_EM_BASE: u64 = 0x3FCA_5C94;

/// Program a base register that maps the 1 KiB exchange-memory bucket
/// starting at `em_off` to `cpu_addr`, in the exact encoding
/// `r_emi_get_mem_addr_by_offset` decodes:
/// bits[31:18] = `em_off >> 2`, bits[17:0] = `cpu_addr >> 2`.
// Only the event-scheduler tests stage a programmed event; without the
// feature this helper has no caller and clippy is right to say so.
#[cfg(feature = "event-scheduler")]
fn em_base_reg(em_off: u32, cpu_addr: u64) -> u32 {
    ((em_off >> 2) << 18) | (((cpu_addr as u32) & EM_RAM_ADDR_MASK) >> 2)
}

/// Stage exactly what the live board had staged: the exchange table at EM
/// `0x000`, the control structure at `0x400`, the TX descriptor at
/// `0x1400` and the advertising payload at `0x2400`, then push entry 0.
///
/// Every byte here is a value read off board `38:44:be:42:f5:58` on
/// 2026-08-02 (silicon capture), except the start time, which is set to
/// the caller's `start_clkn` so the test can drive the schedule.
// Only the event-scheduler tests stage a programmed event; without the
// feature this helper has no caller and clippy is right to say so.
#[cfg(feature = "event-scheduler")]
fn stage_advertising_event(bt: &mut Esp32c3Bt, bus: &mut RamBus, start_clkn: u32) {
    // Exchange-memory windows. Laid out non-contiguously on purpose: the
    // live part's allocator packs them (EM 0x400 lands only 0x158 bytes
    // after EM 0x000), so a model that assumed a flat map would break.
    let map = [
        (0x0000u32, FIXTURE_EM_BASE),
        (0x0400, FIXTURE_EM_BASE + 0x158),
        (0x1400, FIXTURE_EM_BASE + 0x800),
        (0x2400, FIXTURE_EM_BASE + 0xC00),
    ];
    for (i, (em_off, cpu)) in map.iter().enumerate() {
        bt.write_u32(
            EM_BASE_REG_BANK_A + (i as u64) * 4,
            em_base_reg(*em_off, *cpu),
        )
        .unwrap();
    }

    // ET entry 0: status 0, start at `start_clkn` with fine offset 624
    // (= hus 0), CS pointer 0x200 (EM 0x400), duration 0x0AF7.
    let et = FIXTURE_EM_BASE;
    bus.put(et, &0x2802u16.to_le_bytes()); // +0x0 control, status field 0
    bus.put(et + 2, &(start_clkn as u16).to_le_bytes());
    bus.put(
        et + 4,
        &(((start_clkn >> 16) & 0x0FFF) as u16).to_le_bytes(),
    );
    bus.put(et + 6, &624u16.to_le_bytes());
    bus.put(et + 8, &0x0200u16.to_le_bytes());
    bus.put(et + 10, &0x0AF7u16.to_le_bytes());
    bus.put(et + 12, &0x0C00u16.to_le_bytes());
    bus.put(et + 14, &0x0F00u16.to_le_bytes());

    // Control structure, verbatim from the live part.
    let cs = FIXTURE_EM_BASE + 0x158;
    bus.put(cs, &0x0404u16.to_le_bytes());
    bus.put(cs + 0x06, &[0x5a, 0xf5, 0x42, 0xbe, 0x44, 0x38]);
    bus.put(cs + 0x0C, &0x8E89_BED6u32.to_le_bytes());
    bus.put(cs + 0x10, &[0x55, 0x55, 0x55]);
    bus.put(cs + 0x16, &0x8027u16.to_le_bytes()); // channel 39
    bus.put(cs + 0x1C, &0x1400u16.to_le_bytes());

    // TX descriptor: ADV_IND (header 0x20), 15 bytes, payload at 0x2400.
    let txd = FIXTURE_EM_BASE + 0x800;
    bus.put(txd, &0x140Eu16.to_le_bytes());
    bus.put(txd + 2, &0x0F20u16.to_le_bytes());
    bus.put(txd + 4, &0x2400u16.to_le_bytes());

    // The advertising payload the live board had staged.
    bus.put(
        FIXTURE_EM_BASE + 0xC00,
        &[0x02, 0x01, 0x06, 0x05, 0x12, 0x20, 0x00, 0x40, 0x00],
    );
}

/// Stage a **scanning** activity the way the real firmware does: its own
/// exchange-table entry, its own control structure at `cs_idx` in the
/// measured scanning format ([`CS_FORMAT_SCAN`], `0x0208` — no TX
/// descriptor), plus the RX descriptor array and one receive buffer.
///
/// A separate activity from the advertising one on purpose: a node that
/// advertises AND scans has two, with different control-structure indices,
/// and getting the receive path to deliver into the right one is exactly
/// what these tests are about.
#[cfg(feature = "event-scheduler")]
fn stage_scan_event(
    bt: &mut Esp32c3Bt,
    bus: &mut RamBus,
    et_idx: u32,
    cs_idx: u32,
    start_clkn: u32,
) -> (u64, u64) {
    let et = FIXTURE_EM_BASE + u64::from(Esp32c3Bt::et_entry(et_idx));
    let cs_em = EM_CS_OFFSET + cs_idx * CS_STRIDE;
    bus.put(et, &0x2802u16.to_le_bytes());
    bus.put(et + 2, &(start_clkn as u16).to_le_bytes());
    bus.put(
        et + 4,
        &(((start_clkn >> 16) & 0x0FFF) as u16).to_le_bytes(),
    );
    bus.put(et + 6, &624u16.to_le_bytes());
    bus.put(et + 8, &((cs_em / 2) as u16).to_le_bytes());
    bus.put(et + 10, &0x0AF7u16.to_le_bytes());
    bus.put(et + 12, &0x0C00u16.to_le_bytes());
    bus.put(et + 14, &0x0F00u16.to_le_bytes());

    // EM 0x400..0x7FF is one 1 KiB bucket, so the advertising fixture's
    // base register already covers every control structure in it.
    let cs = FIXTURE_EM_BASE + 0x158 + u64::from(cs_em - 0x400);
    bus.put(cs, &0x0208u16.to_le_bytes()); // the measured scan format
    bus.put(cs + 0x06, &[0x5a, 0xf5, 0x42, 0xbe, 0x44, 0x38]);
    bus.put(cs + 0x0C, &0x8E89_BED6u32.to_le_bytes());
    bus.put(cs + 0x10, &[0x55, 0x55, 0x55]);
    bus.put(cs + 0x16, &0x8027u16.to_le_bytes()); // channel 39
    bus.put(cs + 0x1C, &0u16.to_le_bytes()); // listens only

    let rxd_cpu = FIXTURE_EM_BASE + 0x1000;
    let rxbuf_cpu = FIXTURE_EM_BASE + 0x1400;
    bt.write_u32(EM_BASE_REG_BANK_A + 16, em_base_reg(0x1000, rxd_cpu))
        .unwrap();
    bt.write_u32(EM_BASE_REG_BANK_A + 20, em_base_reg(0x1C00, rxbuf_cpu))
        .unwrap();
    // `next = 0x1014` with RXDONE CLEAR: the state firmware's own refill
    // leaves a descriptor in when it hands it to the core.
    bus.put(rxd_cpu, &0x1014u16.to_le_bytes());
    bus.put(rxd_cpu + 0x12, &0x1C00u16.to_le_bytes());
    bt.write_u32(RX_DESC_PTR, 0x0000_1000).unwrap();
    (rxd_cpu, rxbuf_cpu)
}

/// The base-register window is decoded exactly as
/// `r_emi_get_mem_addr_by_offset` does, including the packed non-contiguous
/// layout the live allocator produces. Values are the live registers.
#[test]
fn exchange_memory_offsets_resolve_through_the_base_registers() {
    let mut bt = Esp32c3Bt::new();
    // Silicon capture 2026-08-02, board `38:44:be:42:f5:58`.
    for (i, reg) in [
        0x0002_9725u32,
        0x0402_977B,
        0x0C02_988D,
        0x1002_992B,
        0x1402_9961,
    ]
    .into_iter()
    .enumerate()
    {
        bt.write_u32(EM_BASE_REG_BANK_A + (i as u64) * 4, reg)
            .unwrap();
    }
    assert_eq!(bt.em_cpu_addr(0x0000), Some(0x3FCA_5C94));
    assert_eq!(bt.em_cpu_addr(0x0400), Some(0x3FCA_5DEC));
    assert_eq!(bt.em_cpu_addr(0x1400), Some(0x3FCA_6584));
    // Bucket 2 (EM 0x800..0xBFF) is served by the register that covers
    // 0x400 — exactly what `em_base_reg_lut` encodes.
    assert_eq!(bt.em_cpu_addr(0x0800), Some(0x3FCA_5DEC + 0x400));
    // Offsets inside a region are byte-addressable.
    assert_eq!(bt.em_cpu_addr(0x0406), Some(0x3FCA_5DEC + 6));
    // Nothing mapped yet -> no address, rather than a fabricated one.
    assert_eq!(Esp32c3Bt::new().em_cpu_addr(0), None);
}

/// `+0x100` is a self-clearing command register: `r_sch_prog_ble_push`
/// writes `0x8000_0000 | idx` and the live window reads back 0.
#[test]
fn prog_push_is_a_command_register() {
    let mut bt = Esp32c3Bt::new();
    bt.write_u32(PROG_PUSH, 0x8000_000D).unwrap();
    assert_eq!(bt.read_u32(PROG_PUSH).unwrap(), 0, "reads 0 on silicon");
    assert_eq!(bt.prog_queue.front().copied(), Some(13));
    // Without the go bit nothing is queued.
    bt.prog_queue.clear();
    bt.write_u32(PROG_PUSH, 0x0000_0003).unwrap();
    assert!(bt.prog_queue.is_empty());
}

/// The whole milestone in one test: a pushed event runs at its programmed
/// instant, drives its exchange-table status through the ROM's own state
/// values, emits the PDU the controller staged, and raises `sch_prog_end`
/// through the IRQ FIFO after its programmed duration.
#[cfg(feature = "event-scheduler")]
#[test]
fn a_programmed_event_transmits_the_staged_pdu_and_ends() {
    use crate::sched::EventScheduler;

    let clock = CycleClock::default();
    let mut bt = advertising_part(&clock);
    let air = crate::peripherals::ble_air::BleAirBus::new();
    bt.air = air.clone();
    let mut bus = RamBus::default();
    let mut sched = EventScheduler::new();

    let start: u32 = 200;
    stage_advertising_event(&mut bt, &mut bus, start);
    bt.write_u32(PROG_PUSH, 0x8000_0000).unwrap();

    // The push schedules immediately: the entry cannot be decoded without
    // the bus, so the engine asks for a zero-delay event to do it.
    let events = bt.take_scheduled_events();
    let token = events[0].1;
    assert_eq!(events[0].0, 0, "decode the entry at once");

    // Before the programmed instant: decoded, but nothing has happened.
    let before = (start as u64 - 1) * CYCLES_PER_CLKN_TICK;
    sched.advance_to(before);
    clock.publish(before);
    let res = bt.on_event(token, &mut sched, &mut bus);
    assert!(air.trace_snapshot().is_empty(), "not transmitted early");
    assert_eq!(bt.read_u32(INTSTAT).unwrap() & INT_SCH_PROG_END, 0);
    assert_eq!(
        res.reschedule_delay,
        Some(CYCLES_PER_CLKN_TICK),
        "chained to the programmed instant"
    );

    // At the programmed instant: status 2 and the PDU is on the air.
    let at = start as u64 * CYCLES_PER_CLKN_TICK;
    sched.advance_to(at);
    clock.publish(at);
    let res = bt.on_event(token, &mut sched, &mut bus);
    assert_eq!(
        (bus.u16_at(FIXTURE_EM_BASE) & ET_STATUS_FIELD) >> ET_STATUS_SHIFT,
        ET_STATUS_ONGOING,
        "the core owns the status field while the event runs"
    );
    let frames = air.trace_snapshot();
    assert_eq!(frames.len(), 1, "exactly one frame per event");
    let f = &frames[0];
    assert_eq!(f.channel, 39, "the channel the hop word named");
    assert_eq!(f.access_address, 0x8E89_BED6);
    assert_eq!(f.crc_init, 0x0055_5555);
    assert_eq!(
        f.pdu,
        vec![
            0x20, 0x0f, // ADV_IND with ChSel, 15 bytes
            0x5a, 0xf5, 0x42, 0xbe, 0x44, 0x38, // AdvA from CS+0x06
            0x02, 0x01, 0x06, 0x05, 0x12, 0x20, 0x00, 0x40, 0x00, // AdvData
        ],
        "the real bytes the controller staged, not a synthesised packet"
    );
    // No `sch_prog_tx`: `r_lld_adv_frm_cbk` asserts on irq_type 3.
    assert_eq!(bt.read_u32(INTRAWSTAT).unwrap() & 0x2, 0);
    assert_eq!(
        bt.read_u32(INTSTAT).unwrap() & INT_SCH_PROG_END,
        0,
        "not over"
    );

    // The duration is the one the entry programmed: 0x0AF7 units of two
    // half-µs = 2807 µs.
    let duration = 0x0AF7 * 2 * CYCLES_PER_FINE_TICK;
    assert_eq!(res.reschedule_delay, Some(duration));

    // At the end: status 3 and `sch_prog_end`, queued in the FIFO exactly
    // as `r_rwble_isr` expects to find it.
    let end = at + duration;
    sched.advance_to(end);
    clock.publish(end);
    bt.on_event(token, &mut sched, &mut bus);
    assert_eq!(
        (bus.u16_at(FIXTURE_EM_BASE) & ET_STATUS_FIELD) >> ET_STATUS_SHIFT,
        ET_STATUS_END,
    );
    assert_eq!(
        bt.read_u32(INTSTAT).unwrap() & INT_SCH_PROG_END,
        INT_SCH_PROG_END
    );
    assert_eq!(bt.matrix_irq_sources(), vec![RWBLE_IRQ_SOURCE]);
    assert_eq!(
        bt.read_u32(IRQ_FIFO).unwrap(),
        0x0000_803E,
        "cnt 1, rem 15, bitmap 0x20 — the exact word silicon read mid-event"
    );
    assert_eq!(air.trace_snapshot().len(), 1, "one event, one frame");
}

/// The model must not invent an event it cannot read. With exchange memory
/// unmapped the push is dropped and nothing is completed — firmware stalls
/// visibly instead of being lied to.
#[cfg(feature = "event-scheduler")]
#[test]
fn an_undecodable_event_is_dropped_not_faked() {
    use crate::sched::EventScheduler;

    let clock = CycleClock::default();
    let mut bt = advertising_part(&clock);
    let mut bus = RamBus::default();
    let mut sched = EventScheduler::new();
    bt.write_u32(PROG_PUSH, 0x8000_0000).unwrap();
    let token = bt.take_scheduled_events()[0].1;
    clock.publish(CYCLES_PER_CLKN_TICK);
    sched.advance_to(CYCLES_PER_CLKN_TICK);
    bt.on_event(token, &mut sched, &mut bus);
    assert_eq!(
        bt.read_u32(INTRAWSTAT).unwrap() & INT_SCH_PROG_END,
        0,
        "no end interrupt out of an entry that was never read"
    );
    assert!(bt.radio.is_none());
}

/// A control structure whose format is not the measured legacy-advertising
/// one transmits nothing — but the event still completes, so an unmodelled
/// activity cannot wedge the controller.
#[cfg(feature = "event-scheduler")]
#[test]
fn an_unmeasured_cs_format_transmits_nothing_but_still_ends() {
    use crate::sched::EventScheduler;

    let clock = CycleClock::default();
    let mut bt = advertising_part(&clock);
    let air = crate::peripherals::ble_air::BleAirBus::new();
    bt.air = air.clone();
    let mut bus = RamBus::default();
    let mut sched = EventScheduler::new();
    stage_advertising_event(&mut bt, &mut bus, 0);
    // Anything but the measured 0x04.
    bus.put(FIXTURE_EM_BASE + 0x158, &0x0405u16.to_le_bytes());
    bt.write_u32(PROG_PUSH, 0x8000_0000).unwrap();
    let token = bt.take_scheduled_events()[0].1;
    let end = 0x0AF7 * 2 * CYCLES_PER_FINE_TICK;
    for at in [0, end] {
        sched.advance_to(at);
        clock.publish(at);
        bt.on_event(token, &mut sched, &mut bus);
    }
    assert!(air.trace_snapshot().is_empty(), "no invented frame");
    assert_eq!(
        bt.read_u32(INTRAWSTAT).unwrap() & INT_SCH_PROG_END,
        INT_SCH_PROG_END,
        "the event still ends"
    );
}

/// A listening event writes the air frame into the RX descriptor at
/// `+0x024` in the exact layout the ROM reads back, advances the pointer
/// along the ring, and raises `sch_prog_rx`.
///
/// The activity is a real SCANNING one (its own exchange-table entry and a
/// control structure in the measured `0x0208` format), programmed alongside
/// the advertising activity — which is what a node doing both actually
/// looks like, and what caught the misdelivery bug this fixture now guards.
#[cfg(feature = "event-scheduler")]
#[test]
fn a_listening_event_writes_the_frame_into_the_rx_descriptor() {
    use crate::peripherals::ble_air::{BleAirBus, BleAirFrame};
    use crate::sched::EventScheduler;

    let clock = CycleClock::default();
    let mut bt = advertising_part(&clock);
    let air = BleAirBus::new();
    bt.air = air.clone();
    let mut bus = RamBus::default();
    let mut sched = EventScheduler::new();
    stage_advertising_event(&mut bt, &mut bus, 0);
    let (rxd_cpu, rxbuf_cpu) = stage_scan_event(&mut bt, &mut bus, 1, 1, 0);

    // Somebody else advertises on the channel this control structure names.
    air.transmit(BleAirFrame {
        seq: 0,
        source: bt.node_id + 1,
        channel: 39,
        access_address: 0x8E89_BED6,
        crc_init: 0x0055_5555,
        pdu: vec![0x20, 0x07, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x42],
    });

    // Run the advertising event first (it transmits and, correctly, does
    // NOT consume the peer's frame), then the scan event.
    bt.write_u32(PROG_PUSH, 0x8000_0000).unwrap();
    bt.write_u32(PROG_PUSH, 0x8000_0001).unwrap();
    let token = bt.take_scheduled_events()[0].1;
    let adv_end = 0x0AF7 * 2 * CYCLES_PER_FINE_TICK;
    for at in [0, adv_end, adv_end] {
        sched.advance_to(at);
        clock.publish(at);
        bt.on_event(token, &mut sched, &mut bus);
    }

    assert_eq!(
        bus.u16_at(rxd_cpu + u64::from(RXD_HEADER)),
        (7 << 8) | 0x20,
        "(len << 8) | header, the mirror of the TX descriptor"
    );
    let status = bus.u16_at(rxd_cpu + u64::from(RXD_STATUS));
    assert_eq!(status, RXD_STATUS_GOOD);
    assert_eq!(
        status & RXD_STATUS_ERROR_MASK,
        0,
        "the ROM's own bad-packet mask must clear on it"
    );
    for (i, b) in [0xAAu8, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x42]
        .iter()
        .enumerate()
    {
        assert_eq!(
            bus.read_u8(rxbuf_cpu + i as u64).unwrap(),
            *b,
            "payload byte {i} at the offset +0x12 named"
        );
    }
    assert_eq!(
        bt.read_u32(RX_DESC_PTR).unwrap() & RX_DESC_PTR_MASK,
        0x1014,
        "the pointer walks the ring the firmware linked"
    );
    assert_eq!(
        bt.read_u32(INTSTAT).unwrap() & INT_SCH_PROG_RX,
        INT_SCH_PROG_RX
    );
    // And the same controller never decodes its own transmission: its
    // outgoing ADV_IND is on the very same channel and access address.
    assert_eq!(air.trace_snapshot().len(), 2, "one in, one out");
    assert_eq!(bt.rx_cursor, 1, "cursor past the peer's frame only");

    // ── The three bits `r_lld_rxdesc_check` gates the host report on ─────
    // Getting any one of them wrong is invisible at this level and fatal at
    // the application level: the controller receives and the host never
    // hears about it, which is exactly where this model sat before.
    let w0 = bus.u16_at(rxd_cpu + u64::from(RXD_NEXT));
    assert_eq!(
        w0 & RXD_DONE,
        RXD_DONE,
        "RXDONE must be SET — r_lld_rxdesc_check reports nothing without it"
    );
    assert_eq!(w0 & RXD_NEXT_PTR_MASK, 0x1014, "and the next pointer kept");
    assert_eq!(
        status & RXD_STATUS_RELEASED,
        0,
        "the software-owned released bit must be CLEAR — r_lld_rxdesc_check \
             returns `(status >> 15) ^ 1`, so a set bit means 'already consumed'"
    );
    assert_eq!(
        bus.u16_at(rxd_cpu + u64::from(RXD_LINK_LABEL)) >> RXD_LINK_LABEL_SHIFT,
        1,
        "link label = the control-structure index: this scan activity's CS \
             is at EM 0x45A = 1024 + 1*90, i.e. index 1, while the advertising \
             activity next to it is index 0"
    );
}

/// A receive buffer ABOVE 0x8000 is written where the ROM would read it —
/// the descriptor's payload pointer is a FULL 16-bit exchange-memory
/// offset, not a 15-bit one with a flag on top.
///
/// ## Derived from the ROM, not from this file
///
/// `r_ble_util_buf_rx_free` (`0x4000_315C`) range-checks the buffer it is
/// handed as `((buf - 0x7805) >> 10) & 0xFF <= 8`, so the RX pool is nine
/// 1 KiB buffers whose data pointers are `0x7805, 0x7C05, 0x8005, 0x8405,
/// 0x8805, 0x8C05, 0x9005, 0x9405, 0x9805`. **Five of the nine are at or
/// above 0x8000.** `r_lld_scan_process_pkt_rx_legacy_adv` (`0x4002_46EE`)
/// and `r_lld_scan_process_pkt_rx_adv_rep` (`0x4002_4878`) read `+0x12`
/// with `lhu` and a plain zero-extend — no mask anywhere.
///
/// So this test uses `0x8005`, one of the pool's real offsets, and asserts
/// the bytes land in the 1 KiB window mapped for EM `0x8000`. It also
/// asserts they do NOT land at `0x0005`, which is where the 0x7FFF mask
/// this model used to apply put them: inside the EXCHANGE TABLE. That
/// aliasing is what produced
/// `assert ble_util_buf.c 180, param 000000e2 00000205` ~198 M steps into
/// the two-node run — see [`RXD_DATA_PTR`].
#[cfg(feature = "event-scheduler")]
#[test]
fn a_receive_buffer_above_0x8000_is_not_aliased_into_the_exchange_table() {
    use crate::peripherals::ble_air::{BleAirBus, BleAirFrame};
    use crate::sched::EventScheduler;

    let clock = CycleClock::default();
    let mut bt = advertising_part(&clock);
    let air = BleAirBus::new();
    bt.air = air.clone();
    let mut bus = RamBus::default();
    let mut sched = EventScheduler::new();
    stage_advertising_event(&mut bt, &mut bus, 0);
    let (rxd_cpu, _) = stage_scan_event(&mut bt, &mut bus, 1, 1, 0);

    // Repoint the descriptor at a REAL pool buffer — the third of the nine,
    // the first one with bit15 set — and map the 1 KiB EM bucket it lives
    // in somewhere far away from every other window in the fixture.
    const POOL_BUF: u16 = 0x8005;
    let high_cpu = FIXTURE_EM_BASE + 0x4000;
    bt.write_u32(EM_BASE_REG_BANK_A + 24, em_base_reg(0x8000, high_cpu))
        .unwrap();
    bus.put(rxd_cpu + u64::from(RXD_DATA_PTR), &POOL_BUF.to_le_bytes());

    let et0_before: Vec<u8> = (5..9u64)
        .map(|i| bus.read_u8(FIXTURE_EM_BASE + i).unwrap())
        .collect();

    air.transmit(BleAirFrame {
        seq: 0,
        source: bt.node_id + 1,
        channel: 39,
        access_address: 0x8E89_BED6,
        crc_init: 0x0055_5555,
        pdu: vec![0x20, 0x04, 0xDE, 0xAD, 0xBE, 0xEF],
    });

    bt.write_u32(PROG_PUSH, 0x8000_0001).unwrap();
    let token = bt.take_scheduled_events()[0].1;
    let end = 0x0AF7 * 2 * CYCLES_PER_FINE_TICK;
    for at in [0, end] {
        sched.advance_to(at);
        clock.publish(at);
        bt.on_event(token, &mut sched, &mut bus);
    }

    let et0_after: Vec<u8> = (5..9u64)
        .map(|i| bus.read_u8(FIXTURE_EM_BASE + i).unwrap())
        .collect();

    // EM 0x8005 is 5 bytes into the bucket mapped at `high_cpu`.
    for (i, b) in [0xDEu8, 0xAD, 0xBE, 0xEF].iter().enumerate() {
        assert_eq!(
            bus.read_u8(high_cpu + 5 + i as u64).unwrap(),
            *b,
            "payload byte {i} must land at EM {POOL_BUF:#06x}, the offset \
                 the descriptor names and the ROM reads back"
        );
    }
    // And NOT at the 15-bit alias. EM 0x0005 is 5 bytes into the exchange
    // table, whose bucket the fixture maps at FIXTURE_EM_BASE; writing
    // there corrupts exchange-table entry 0 in place. Compared against what
    // the fixture staged rather than against zero, because "unchanged" is
    // the property and zero is not what is there.
    assert_eq!(
        &et0_after[..],
        &et0_before[..],
        "exchange-table entry 0 bytes 5..9 were overwritten by the received \
             payload — the buffer pointer is being masked with 0x7FFF, which \
             folds the top five RX pool buffers onto EM 0x0000..0x1FFF (and \
             0x9005 onto the descriptor ring itself)"
    );
    assert_eq!(
        bus.u16_at(rxd_cpu + u64::from(RXD_DATA_PTR)),
        POOL_BUF,
        "the buffer pointer is SOFTWARE-owned; the core must not touch it"
    );
}

/// A reception writes EVERY core-owned descriptor field, including the ones
/// this model has nothing to say about.
///
/// A hardware-owned field the model leaves alone is not "unmodelled", it is
/// whatever the SRAM behind exchange memory last held, and the link layer
/// reads it as hardware output either way. `+0xE` is the one that proved
/// it: `r_lld_scan_process_pkt_rx_adv_rep` (`0x4002_4978`) copies it into
/// the advertising report, and ESP-IDF's `lld_adv_rep_ind` handler
/// dereferences it as a resolving-list exchange-memory pointer whenever it
/// is non-zero.
///
/// The descriptor is POISONED first, because that is the only way this test
/// can fail: a zero-initialised fixture cannot tell "written 0" from "never
/// written".
#[cfg(feature = "event-scheduler")]
#[test]
fn a_reception_writes_every_core_owned_descriptor_field() {
    use crate::peripherals::ble_air::{BleAirBus, BleAirFrame};
    use crate::sched::EventScheduler;

    let clock = CycleClock::default();
    let mut bt = advertising_part(&clock);
    let air = BleAirBus::new();
    bt.air = air.clone();
    let mut bus = RamBus::default();
    let mut sched = EventScheduler::new();
    stage_advertising_event(&mut bt, &mut bus, 0);
    let (rxd_cpu, rxbuf_cpu) = stage_scan_event(&mut bt, &mut bus, 1, 1, 0);

    // Poison every core-owned field with a value that would be catastrophic
    // if it survived. 0xFF05 is the exact shape that killed the twin: the
    // handler dereferences `emi_get_mem_addr_by_offset(0xFF05 + 46)` and the
    // ROM asserts `emi.c 159` because `0xFF33 >> 10 = 63 > 50`.
    for off in [RXD_RSSI, RXD_RAL_PTR, RXD_UNKNOWN_10] {
        bus.put(rxd_cpu + u64::from(off), &0xFF05u16.to_le_bytes());
    }

    air.transmit(BleAirFrame {
        seq: 0,
        source: bt.node_id + 1,
        channel: 39,
        access_address: 0x8E89_BED6,
        crc_init: 0x0055_5555,
        pdu: vec![0x20, 0x02, 0x11, 0x22],
    });

    bt.write_u32(PROG_PUSH, 0x8000_0001).unwrap();
    let token = bt.take_scheduled_events()[0].1;
    let end = 0x0AF7 * 2 * CYCLES_PER_FINE_TICK;
    for at in [0, end] {
        sched.advance_to(at);
        clock.publish(at);
        bt.on_event(token, &mut sched, &mut bus);
    }

    assert_eq!(
        bus.u16_at(rxd_cpu + u64::from(RXD_RAL_PTR)),
        0,
        "+0xE is the resolving-list pointer ESP-IDF's lld_adv_rep_ind \
             handler dereferences when non-zero. Address resolution is not \
             modelled, so the core must write 0 — leaving the field alone hands \
             the link layer a stale exchange-memory pointer"
    );
    assert_eq!(
        bus.u16_at(rxd_cpu + u64::from(RXD_RSSI)),
        0,
        "+0x6 low byte is the raw RSSI r_lld_scan_process_pkt_rx_adv_rep \
             feeds to rf_api.rssi_convert; there is no PHY here, so 0"
    );
    assert_eq!(
        bus.u16_at(rxd_cpu + u64::from(RXD_UNKNOWN_10)),
        0,
        "+0x10 has no identified reader, but stale is not the same as \
             unmodelled"
    );
    // The reception itself still landed, so this is not passing because
    // nothing happened.
    assert_eq!(bus.read_u8(rxbuf_cpu).unwrap(), 0x11);
    assert_eq!(bus.read_u8(rxbuf_cpu + 1).unwrap(), 0x22);
    assert_eq!(
        bus.u16_at(rxd_cpu + u64::from(RXD_NEXT)) & RXD_DONE,
        RXD_DONE
    );
}

/// An ADVERTISING event does not swallow a frame the scanning activity is
/// waiting for. This is not tidiness: the core stamps the RUNNING
/// activity's link label into the descriptor, so a frame delivered to the
/// advertising activity is stamped with its label,
/// `r_lld_scan_process_pkt_rx` rejects it as somebody else's, never frees
/// it, and the descriptor stays `RXDONE` forever. One misdelivery and the
/// node is permanently deaf — which is exactly what a node advertising and
/// scanning at once did before this gate existed.
#[cfg(feature = "event-scheduler")]
#[test]
fn an_advertising_event_does_not_swallow_the_scanners_frame() {
    use crate::peripherals::ble_air::{BleAirBus, BleAirFrame};
    use crate::sched::EventScheduler;

    let clock = CycleClock::default();
    let mut bt = advertising_part(&clock);
    let air = BleAirBus::new();
    bt.air = air.clone();
    let mut bus = RamBus::default();
    let mut sched = EventScheduler::new();
    stage_advertising_event(&mut bt, &mut bus, 0);
    // The scan activity lives at a DIFFERENT control-structure index, as it
    // does in real firmware — so a misdelivery would be visible as a wrong
    // label even if the ring survived it.
    let (rxd_cpu, _) = stage_scan_event(&mut bt, &mut bus, 1, 2, 0);

    air.transmit(BleAirFrame {
        seq: 0,
        source: bt.node_id + 1,
        channel: 39,
        access_address: 0x8E89_BED6,
        crc_init: 0x0055_5555,
        pdu: vec![0x20, 0x06, 1, 2, 3, 4, 5, 6],
    });

    // ONLY the advertising event runs.
    bt.write_u32(PROG_PUSH, 0x8000_0000).unwrap();
    let token = bt.take_scheduled_events()[0].1;
    let adv_end = 0x0AF7 * 2 * CYCLES_PER_FINE_TICK;
    for at in [0, adv_end] {
        sched.advance_to(at);
        clock.publish(at);
        bt.on_event(token, &mut sched, &mut bus);
    }
    assert_eq!(air.trace_snapshot().len(), 2, "it transmitted");
    assert_eq!(
        bus.u16_at(rxd_cpu + u64::from(RXD_HEADER)),
        0,
        "and wrote NOTHING into the RX descriptor"
    );
    assert_eq!(bt.read_u32(INTRAWSTAT).unwrap() & INT_SCH_PROG_RX, 0);
    assert_eq!(bt.rx_cursor, 0, "the frame is still unread");

    // Now the scan event runs and picks it up, stamped with ITS index.
    bt.write_u32(PROG_PUSH, 0x8000_0001).unwrap();
    let token = bt.take_scheduled_events()[0].1;
    for at in [adv_end, adv_end] {
        sched.advance_to(at);
        clock.publish(at);
        bt.on_event(token, &mut sched, &mut bus);
    }
    assert_eq!(
        bus.u16_at(rxd_cpu + u64::from(RXD_HEADER)),
        (6 << 8) | 0x20,
        "the scanning activity received it"
    );
    assert_eq!(
        bus.u16_at(rxd_cpu + u64::from(RXD_LINK_LABEL)) >> RXD_LINK_LABEL_SHIFT,
        2,
        "stamped with the SCAN activity's control-structure index"
    );
}

/// The link label the core stamps into `+0x0C` is the *control-structure
/// index* of the activity that received, not a constant: a scan activity
/// whose control structure sits at CS index 2 gets label 2. Without this
/// `r_lld_rxdesc_check` would reject every packet a non-zero-index activity
/// received, and the host would see nothing while the trace showed a
/// perfectly healthy reception.
#[cfg(feature = "event-scheduler")]
#[test]
fn the_link_label_is_the_control_structure_index() {
    use crate::peripherals::ble_air::{BleAirBus, BleAirFrame};
    use crate::sched::EventScheduler;

    let clock = CycleClock::default();
    let mut bt = advertising_part(&clock);
    let air = BleAirBus::new();
    bt.air = air.clone();
    let mut bus = RamBus::default();
    let mut sched = EventScheduler::new();
    stage_advertising_event(&mut bt, &mut bus, 0);
    let (rxd_cpu, _) = stage_scan_event(&mut bt, &mut bus, 3, 2, 0);

    air.transmit(BleAirFrame {
        seq: 0,
        source: bt.node_id + 1,
        channel: 39,
        access_address: 0x8E89_BED6,
        crc_init: 0x0055_5555,
        pdu: vec![0x20, 0x06, 1, 2, 3, 4, 5, 6],
    });

    bt.write_u32(PROG_PUSH, 0x8000_0003).unwrap();
    let token = bt.take_scheduled_events()[0].1;
    sched.advance_to(0);
    clock.publish(0);
    bt.on_event(token, &mut sched, &mut bus);

    assert_eq!(
        bus.u16_at(rxd_cpu + u64::from(RXD_LINK_LABEL)) >> RXD_LINK_LABEL_SHIFT,
        2,
        "the label follows the control-structure index the ET named"
    );
}

/// The core does not overwrite a descriptor whose `RXDONE` firmware has not
/// cleared — that reception has not been consumed yet. Nothing is written,
/// no `sch_prog_rx` is raised, the pointer does not move, and the frame is
/// still on the air for the next event to pick up.
#[cfg(feature = "event-scheduler")]
#[test]
fn a_descriptor_the_link_layer_still_owns_is_not_overwritten() {
    use crate::peripherals::ble_air::{BleAirBus, BleAirFrame};
    use crate::sched::EventScheduler;

    let clock = CycleClock::default();
    let mut bt = advertising_part(&clock);
    let air = BleAirBus::new();
    bt.air = air.clone();
    let mut bus = RamBus::default();
    let mut sched = EventScheduler::new();
    stage_advertising_event(&mut bt, &mut bus, 0);
    let (rxd_cpu, rxbuf_cpu) = stage_scan_event(&mut bt, &mut bus, 1, 1, 0);
    // RXDONE still SET: the previous reception has not been released.
    bus.put(rxd_cpu, &(0x1014u16 | RXD_DONE).to_le_bytes());

    air.transmit(BleAirFrame {
        seq: 0,
        source: bt.node_id + 1,
        channel: 39,
        access_address: 0x8E89_BED6,
        crc_init: 0x0055_5555,
        pdu: vec![0x20, 0x02, 0x11, 0x22],
    });

    bt.write_u32(PROG_PUSH, 0x8000_0001).unwrap();
    let token = bt.take_scheduled_events()[0].1;
    sched.advance_to(0);
    clock.publish(0);
    bt.on_event(token, &mut sched, &mut bus);

    assert_eq!(
        bus.u16_at(rxd_cpu + u64::from(RXD_HEADER)),
        0,
        "not written"
    );
    assert_eq!(bus.read_u8(rxbuf_cpu).unwrap(), 0, "payload not written");
    assert_eq!(bt.read_u32(INTRAWSTAT).unwrap() & INT_SCH_PROG_RX, 0);
    assert_eq!(bt.read_u32(RX_DESC_PTR).unwrap() & RX_DESC_PTR_MASK, 0x1000);
    assert_eq!(bt.rx_cursor, 0, "the frame was not consumed");
    assert!(
        air.receive_from(39, 0x8E89_BED6, 0, bt.node_id).is_some(),
        "and it is still on the air for a later event"
    );
}

/// `+0x2D0` bit15 is the RX-buffer jump request, and its **rising edge** is
/// what raises `lld_update_rxbuf_isr` (bit 18) — the exact two-store
/// sequence `r_lld_update_rxbuf` ends with. The core adopts the requested
/// descriptor as its current one, and the ISR's clearing write must not
/// raise anything.
#[test]
fn rx_buf_jump_raises_bit_18_on_its_go_edge() {
    let clock = CycleClock::default();
    let mut bt = advertising_part(&clock);
    assert_ne!(
        bt.read_u32(INTCNTL).unwrap() & INT_LLD_UPDATE_RXBUF,
        0,
        "the silicon enable word arms bit 18"
    );
    bt.write_u32(RX_DESC_PTR, 0x0000_1000).unwrap();

    // Store 1: the target descriptor, no go bit. Nothing happens.
    bt.write_u32(RX_BUF_JUMP, 0x0000_1064).unwrap();
    assert_eq!(bt.read_u32(INTRAWSTAT).unwrap() & INT_LLD_UPDATE_RXBUF, 0);
    assert_eq!(bt.read_u32(RX_DESC_PTR).unwrap(), 0x0000_1000);

    // Store 2: set bit15. Bit 18 latches, the FIFO carries it, and the core
    // adopts the descriptor.
    bt.write_u32(RX_BUF_JUMP, 0x0000_1064 | RX_BUF_JUMP_GO)
        .unwrap();
    assert_eq!(
        bt.read_u32(INTSTAT).unwrap() & INT_LLD_UPDATE_RXBUF,
        INT_LLD_UPDATE_RXBUF
    );
    assert_eq!(bt.read_u32(IRQ_FIFO).unwrap() >> 10, INT_LLD_UPDATE_RXBUF);
    assert_eq!(
        bt.read_u32(RX_DESC_PTR).unwrap() & RX_DESC_PTR_MASK,
        0x1064,
        "the core jumps to the descriptor software handed it"
    );
    // The register reads back what was written: the ISR read-modify-writes
    // it to drop bit15.
    assert_eq!(
        bt.read_u32(RX_BUF_JUMP).unwrap(),
        0x0000_1064 | RX_BUF_JUMP_GO
    );

    // The ISR's clear is not a new request.
    bt.write_u32(IRQ_FIFO, 1).unwrap();
    bt.write_u32(INTACK, INT_LLD_UPDATE_RXBUF).unwrap();
    bt.write_u32(RX_BUF_JUMP, 0x0000_1064).unwrap();
    assert_eq!(
        bt.read_u32(INTRAWSTAT).unwrap() & INT_LLD_UPDATE_RXBUF,
        0,
        "clearing the go bit must not re-raise the interrupt"
    );
    // A fresh request does raise again.
    bt.write_u32(RX_BUF_JUMP, 0x0000_1078 | RX_BUF_JUMP_GO)
        .unwrap();
    assert_eq!(
        bt.read_u32(INTRAWSTAT).unwrap() & INT_LLD_UPDATE_RXBUF,
        INT_LLD_UPDATE_RXBUF
    );
    assert_eq!(bt.read_u32(RX_DESC_PTR).unwrap() & RX_DESC_PTR_MASK, 0x1078);
}

/// No frame on the air means no interrupt and no exchange-memory write —
/// a reception is never invented to keep a scanner busy.
#[cfg(feature = "event-scheduler")]
#[test]
fn a_silent_air_delivers_nothing() {
    use crate::sched::EventScheduler;

    let clock = CycleClock::default();
    let mut bt = advertising_part(&clock);
    bt.air = crate::peripherals::ble_air::BleAirBus::new();
    let mut bus = RamBus::default();
    let mut sched = EventScheduler::new();
    stage_advertising_event(&mut bt, &mut bus, 0);
    stage_scan_event(&mut bt, &mut bus, 1, 1, 0);
    bt.write_u32(PROG_PUSH, 0x8000_0001).unwrap();
    let token = bt.take_scheduled_events()[0].1;
    sched.advance_to(0);
    clock.publish(0);
    bt.on_event(token, &mut sched, &mut bus);
    assert_eq!(bt.read_u32(INTRAWSTAT).unwrap() & INT_SCH_PROG_RX, 0);
    assert_eq!(
        bt.read_u32(RX_DESC_PTR).unwrap(),
        0x0000_1000,
        "the descriptor pointer does not move without a reception"
    );
}

/// CLKN must be monotonic — the event scheduler re-reads it to decide
/// whether its deadline already slipped.
#[test]
fn clkn_is_monotonic() {
    let mut bt = Esp32c3Bt::new();
    let clock = CycleClock::default();
    bt.attach_cycle_clock(clock.clone());
    clock.publish(0);
    bt.write_u32(0x000, 1).unwrap();
    let mut last = 0;
    for n in 0..5_000u64 {
        clock.publish(n * 9_973);
        let now = bt.read_u32(CLKN).unwrap();
        assert!(now >= last, "CLKN went backwards: {last} -> {now}");
        last = now;
    }
    assert!(last > 0, "CLKN never advanced");
}
/// A simulation RESTART reuses the lab's `AirBus` and builds fresh
/// controllers. The previous run's frames are still on that air, and
/// `next_node_id()` gives the restarted controller a new identity — so the
/// old frames are not filtered out as its own. It must still not see them:
/// a radio hears what is transmitted while it is listening, not a backlog.
#[test]
fn a_controller_built_after_a_restart_skips_the_previous_runs_backlog() {
    use crate::peripherals::ble_air::{BleAirBus, BleAirFrame};
    let air = BleAirBus::new();

    // Run 1: two nodes trade advertising frames on the primary channels.
    for _ in 0..10 {
        for ch in [37u8, 38, 39] {
            air.transmit(BleAirFrame {
                seq: 0,
                source: 1,
                channel: ch,
                access_address: 0x8E89_BED6,
                crc_init: 0x0055_5555,
                pdu: vec![0x20, 0x02, 0xE5, 0x02],
            });
        }
    }
    let backlog = air.current_seq();
    assert_eq!(backlog, 30, "the previous run left frames on the air");

    // Restart: same air, brand-new controller.
    let restarted = Esp32c3Bt::with_air(air.clone());
    assert_eq!(
        restarted.rx_cursor, backlog,
        "a restarted controller joins the air where it is now, not at 0",
    );
    assert!(
        air.receive_from(37, 0x8E89_BED6, restarted.rx_cursor, restarted.node_id)
            .is_none(),
        "and therefore sees none of the previous run's traffic",
    );
}
