use super::*;
use crate::Peripheral;

/// A slave that returns an incrementing byte pattern on each read (so a
/// master-receive advances observably) and records writes.
struct RampDevice {
    address: u8,
    next: std::cell::Cell<u8>,
}
impl I2cDevice for RampDevice {
    fn address(&self) -> u8 {
        self.address
    }
    fn read(&mut self) -> u8 {
        let v = self.next.get();
        self.next.set(v.wrapping_add(1));
        v
    }
    fn write(&mut self, _data: u8) {}
}

fn ramp_slave() -> Box<dyn I2cDevice> {
    Box::new(RampDevice {
        address: 0x1E,
        next: std::cell::Cell::new(0x40),
    })
}

fn kinetis(scheduler: bool) -> I2c {
    let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Kinetis);
    i2c.push_slave(ramp_slave());
    if scheduler {
        i2c.attach_cycle_clock(CycleClock::default());
    }
    i2c
}

fn f1(scheduler: bool) -> I2c {
    let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Stm32F1);
    i2c.push_slave(ramp_slave());
    if scheduler {
        i2c.attach_cycle_clock(CycleClock::default());
    }
    i2c
}

fn l4(scheduler: bool) -> I2c {
    let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Stm32L4);
    i2c.push_slave(ramp_slave());
    if scheduler {
        i2c.attach_cycle_clock(CycleClock::default());
    }
    i2c
}

/// Clone the bus clock a scheduler-mode instance latched (any variant).
fn clock_of(i2c: &I2c) -> CycleClock {
    match i2c {
        I2c::Stm32F1(i) => i.clock.clone(),
        I2c::Stm32L4(i) => i.clock.clone(),
        I2c::Kinetis(i) => i.clock.clone(),
        I2c::Efr32s2(i) => i.clock.clone(),
    }
    .expect("scheduler-mode instance has a clock")
}

#[derive(Clone, Copy, Debug)]
enum Op {
    Write(u64, u8),
    Read(u64),
}

/// Drive a scheduler-mode Kinetis I2C exactly the way `Machine` +
/// `SystemBus` do at tick interval 1: publish the clock each cycle, arm
/// write-harvested events at `cycle + 1 + delay`, and drain due events
/// through `on_event` (rescheduling at `now + delay`), recording the cycles
/// the level chain pends the own-IRQ.
struct SchedHarness {
    i2c: I2c,
    clock: CycleClock,
    bus: crate::bus::SystemBus,
    events: Vec<(u64, u32)>,
    now: u64,
    pends: Vec<u64>,
}

impl SchedHarness {
    fn new(build: &dyn Fn(bool) -> I2c) -> Self {
        let i2c = build(true);
        let clock = clock_of(&i2c);
        Self {
            i2c,
            clock,
            bus: crate::bus::SystemBus::new(),
            events: Vec::new(),
            now: 0,
            pends: Vec::new(),
        }
    }

    fn write(&mut self, off: u64, val: u8) {
        self.i2c.sync_to(self.now);
        self.i2c.write(off, val).unwrap();
        for (delay, token) in self.i2c.take_scheduled_events() {
            self.events.push((self.now + 1 + delay, token));
        }
    }

    /// A `&self` register read — never arms an event (mirrors the bus read
    /// path); a `D` read that latches IICIF is caught by the already-live
    /// perpetual chain.
    fn read(&mut self, off: u64) -> u8 {
        self.i2c.read(off).unwrap()
    }

    fn step(&mut self) {
        self.now += 1;
        self.clock.publish(self.now);
        let due: Vec<(u64, u32)> = self
            .events
            .iter()
            .copied()
            .filter(|(d, _)| *d <= self.now)
            .collect();
        self.events.retain(|(d, _)| *d > self.now);
        let mut sched = crate::sched::EventScheduler::new();
        sched.advance_to(self.now);
        for (_, token) in due {
            let res = self.i2c.on_event(token, &mut sched, &mut self.bus);
            if res.raise_own_irq {
                self.pends.push(self.now);
            }
            if let Some(delay) = res.reschedule_delay {
                self.events.push((self.now + delay, token));
            }
        }
    }
}

/// Legacy per-tick oracle.
fn walk_tick(i2c: &mut I2c) -> bool {
    i2c.tick().irq
}

/// The heart of the gate: replay the SAME op script against (a) the legacy
/// per-tick walk and (b) the event path, comparing the full register
/// snapshot AND every returned read byte at every cycle, plus the exact set
/// of NVIC-pend cycles. An `Op` scheduled at cycle `c` is applied before
/// that cycle's tick.
fn assert_walk_identical_with(
    build: &dyn Fn(bool) -> I2c,
    script: &[(u64, Op)],
    cycles: u64,
    what: &str,
) {
    let mut walk = build(false);
    let mut sched = SchedHarness::new(build);
    let mut walk_pends: Vec<u64> = Vec::new();

    for c in 1..=cycles {
        for (sc, op) in script {
            if *sc == c {
                match *op {
                    Op::Write(off, val) => {
                        walk.write(off, val).unwrap();
                        sched.now = c - 1;
                        sched.write(off, val);
                    }
                    Op::Read(off) => {
                        let w = walk.read(off).unwrap();
                        sched.now = c - 1;
                        let s = sched.read(off);
                        assert_eq!(w, s, "{what}: read(0x{off:02x}) diverged at cycle {c}");
                    }
                }
            }
        }
        if walk_tick(&mut walk) {
            walk_pends.push(c);
        }
        sched.now = c - 1;
        sched.step();
        assert_eq!(
            walk.snapshot(),
            sched.i2c.snapshot(),
            "{what}: register state diverged at cycle {c}"
        );
    }
    assert_eq!(walk_pends, sched.pends, "{what}: NVIC pend cycles diverged");
}

/// Kinetis-variant convenience wrapper.
fn assert_walk_identical(script: &[(u64, Op)], cycles: u64, what: &str) {
    assert_walk_identical_with(&kinetis, script, cycles, what);
}

#[test]
fn clock_attach_flips_to_scheduler_and_walk_tick_is_inert() {
    let mut i2c = kinetis(true);
    assert!(i2c.uses_scheduler());
    assert!(!i2c.needs_legacy_walk());
    // Latch a level (address byte with IICIE) then confirm tick() is inert.
    i2c.write(0x02, KI_C1_MST | KI_C1_TX | KI_C1_IICIE).unwrap();
    i2c.write(0x04, 0x3C).unwrap(); // address → byte_complete sets IICIF
    assert!(!i2c.tick().irq, "tick must be inert in scheduler mode");
}

#[test]
fn all_three_variants_flip_to_scheduler_and_walk_tick_is_inert() {
    // With a clock attached (event-scheduler builds) every I2C variant now
    // migrates: the F1/L4 transaction engine is self-paced by the same
    // held-level event chain the Kinetis variant uses, so the per-cycle walk
    // is no longer needed. The walk-guarded `tick()` is inert in that mode.
    for build in [&f1 as &dyn Fn(bool) -> I2c, &l4, &kinetis] {
        let mut i2c = build(true);
        assert!(i2c.uses_scheduler());
        assert!(!i2c.needs_legacy_walk());
        assert!(
            !i2c.tick().irq && i2c.tick().cycles == 0,
            "walk tick must be inert in scheduler mode"
        );
        // Clock detached (differential reference / featureless): back to walk.
        i2c.force_legacy_walk();
        assert!(!i2c.uses_scheduler());
        assert!(i2c.needs_legacy_walk());
    }
}

// ── STM32 F1 transaction-engine walk-vs-scheduler byte identity ───────────

/// Master WRITE: START → address(W) → data byte → STOP, with ITEVTEN|ITBUFEN
/// enabled so the completion IRQs are pend-compared. Every register snapshot,
/// read byte and NVIC-pend cycle must be byte-identical between the per-cycle
/// walk and the event-scheduled engine.
#[test]
fn f1_master_write_walk_identity() {
    let addr_w = 0x1E << 1; // 0x3C
    let script = [
        (1u64, Op::Write(0x05, 0x06)), // CR2 = ITEVTEN|ITBUFEN (bits 9,10)
        (1, Op::Write(0x01, 0x01)),    // CR1.START (bit 8)
        (4, Op::Read(0x14)),           // poll SR1 (SB)
        (5, Op::Write(0x10, addr_w)),  // DR = address(W) → AddressPending
        (28, Op::Read(0x14)),          // poll SR1 (ADDR/TXE)
        (28, Op::Read(0x18)),          // poll SR2 (MSL/BUSY)
        (30, Op::Write(0x10, 0xAF)),   // DR = data byte → DataPending
        (54, Op::Read(0x14)),          // poll SR1 (TXE/BTF)
        (56, Op::Write(0x01, 0x02)),   // CR1.STOP (bit 9)
    ];
    assert_walk_identical_with(&f1, &script, 64, "f1 master write");
}

/// Master READ: START → address(R) → multi-byte receive (the `&self` DR-read
/// path that the prior model claimed could not be event-scheduled) → STOP.
/// The receive bytes come straight from the device in `read()`; the engine
/// only paces the START/ADDR/first-byte countdowns. The already-live chain
/// keeps the register state identical across the read-gated stream.
#[test]
fn f1_master_read_multibyte_walk_identity() {
    let addr_r = (0x1E << 1) | 1; // 0x3D
    let script = [
        (1u64, Op::Write(0x05, 0x06)), // CR2 = ITEVTEN|ITBUFEN
        (1, Op::Write(0x01, 0x01)),    // START
        (5, Op::Write(0x10, addr_r)),  // DR = address(R) → AddressPending(read)
        (30, Op::Read(0x14)),          // poll SR1 (ADDR)
        (54, Op::Read(0x14)),          // poll SR1 (RXNE after first byte)
        (54, Op::Read(0x10)),          // read byte 0 (buffered dr)
        (55, Op::Read(0x10)),          // read byte 1 (device pull)
        (56, Op::Read(0x10)),          // read byte 2 (device pull)
        (57, Op::Read(0x18)),          // SR2 still BUSY
        (58, Op::Write(0x01, 0x02)),   // STOP
    ];
    assert_walk_identical_with(&f1, &script, 66, "f1 master read multibyte");
}

/// Address NACK (no slave at the addressed target) — the AF/MSL/BUSY latch
/// and the ITERREN-gated error IRQ must match. Uses a mismatched address so
/// `current_target` is `None`.
#[test]
fn f1_address_nack_walk_identity() {
    let script = [
        (1u64, Op::Write(0x05, 0x01)), // CR2 ITERREN (bit 8) → byte at offset 0x05
        (1, Op::Write(0x01, 0x01)),    // START
        (5, Op::Write(0x10, 0x40)),    // DR = address 0x20<<1 (no device) → NACK
        (30, Op::Read(0x14)),          // poll SR1 (AF)
        (30, Op::Read(0x18)),          // poll SR2 (MSL/BUSY held)
        (32, Op::Write(0x01, 0x02)),   // STOP releases the bus
    ];
    assert_walk_identical_with(&f1, &script, 40, "f1 address NACK");
}

// ── STM32 L4 transaction-engine walk-vs-scheduler byte identity ───────────

/// L4 master WRITE via CR2 START/AUTOEND + TXDR, with TCIE|NACKIE enabled.
#[test]
fn l4_master_write_walk_identity() {
    // CR1.PE (bit0) | TCIE (bit6) | NACKIE (bit4) = 0x51.
    // CR2 = SADD(0x1E<<1) | NBYTES=1<<16 | AUTOEND<<25 | START<<13.
    let cr2: u32 = ((0x1E << 1) as u32) | (1 << 16) | (1 << 25) | (1 << 13);
    let script = [
        (1u64, Op::Write(0x00, 0x51)), // CR1 = PE|TCIE|NACKIE
        (2, Op::Write(0x04, (cr2 & 0xFF) as u8)),
        (2, Op::Write(0x05, ((cr2 >> 8) & 0xFF) as u8)),
        (2, Op::Write(0x06, ((cr2 >> 16) & 0xFF) as u8)),
        (2, Op::Write(0x07, ((cr2 >> 24) & 0xFF) as u8)), // START latches BUSY
        (3, Op::Read(0x19)),                              // ISR byte3 (BUSY bit15)
        (4, Op::Write(0x28, 0xAF)),                       // TXDR → AddressPending
        (28, Op::Read(0x18)),                             // ISR byte0 (TXE/TC)
        (28, Op::Read(0x19)),                             // ISR byte3 (BUSY cleared by AUTOEND)
    ];
    assert_walk_identical_with(&l4, &script, 36, "l4 master write");
}

/// L4 address NACK (no device) — NACKF + AUTOEND STOPF, NACKIE IRQ.
#[test]
fn l4_address_nack_walk_identity() {
    let cr2: u32 = ((0x20 << 1) as u32) | (1 << 16) | (1 << 25) | (1 << 13);
    let script = [
        (1u64, Op::Write(0x00, 0x51)), // CR1 = PE|TCIE|NACKIE
        (2, Op::Write(0x04, (cr2 & 0xFF) as u8)),
        (2, Op::Write(0x05, ((cr2 >> 8) & 0xFF) as u8)),
        (2, Op::Write(0x06, ((cr2 >> 16) & 0xFF) as u8)),
        (2, Op::Write(0x07, ((cr2 >> 24) & 0xFF) as u8)),
        (4, Op::Write(0x28, 0xAF)), // TXDR → AddressPending → NACK
        (28, Op::Read(0x18)),       // ISR (NACKF/STOPF)
        (28, Op::Read(0x19)),       // ISR byte3 (BUSY)
    ];
    assert_walk_identical_with(&l4, &script, 36, "l4 address NACK");
}

#[test]
fn master_write_level_irq_walk_identity() {
    // START, address (byte_complete latches IICIF), enable IICIE (level
    // high), let it pend for a few cycles (ISR latency), clear IICIF + send
    // a data byte (re-latch), clear again, then STOP.
    let addr_w = 0x1E << 1; // write
    let script = [
        (1u64, Op::Write(0x02, KI_C1_MST | KI_C1_TX)), // START
        (1, Op::Write(0x04, addr_w)),                  // address → IICIF
        (2, Op::Write(0x02, KI_C1_MST | KI_C1_TX | KI_C1_IICIE)), // enable IICIE
        (6, Op::Write(0x03, KI_S_IICIF)),              // ISR clears IICIF
        (6, Op::Write(0x04, 0xAA)),                    // next byte → IICIF
        (11, Op::Write(0x03, KI_S_IICIF)),             // clear
        (11, Op::Write(0x04, 0xBB)),                   // byte → IICIF
        (16, Op::Write(0x03, KI_S_IICIF)),             // clear
        (17, Op::Write(0x02, 0)),                      // STOP (MST 1→0)
    ];
    assert_walk_identical(&script, 24, "kinetis master write level IRQ");
}

#[test]
fn master_read_dread_latches_irq_walk_identity() {
    // The crux: a master-receive `D` read latches IICIF via a `&self` read
    // (which cannot arm an event) — the already-live perpetual level chain
    // must pend on the SAME cycle as the walk.
    let addr_r = (0x1E << 1) | 1; // read
    let script = [
        (1u64, Op::Write(0x02, KI_C1_MST | KI_C1_TX | KI_C1_IICIE)), // START + IICIE
        (1, Op::Write(0x04, addr_r)), // address(R) → IICIF, is_reading
        (5, Op::Write(0x03, KI_S_IICIF)), // ISR clears IICIF
        (5, Op::Write(0x02, KI_C1_MST | KI_C1_IICIE)), // TX=0 → enter RX (rx_dummy_pending)
        (6, Op::Read(0x04)),          // dummy read → IICIF (bus release)
        (10, Op::Write(0x03, KI_S_IICIF)), // clear
        (11, Op::Read(0x04)),         // data read → device byte + IICIF
        (15, Op::Write(0x03, KI_S_IICIF)), // clear
        (16, Op::Read(0x04)),         // data read → IICIF
        (20, Op::Write(0x03, KI_S_IICIF)), // clear
        (21, Op::Write(0x02, 0)),     // STOP
    ];
    assert_walk_identical(&script, 28, "kinetis master read D-latch level IRQ");
}

#[test]
fn iicie_disabled_never_pends_walk_identity() {
    // IICIF latched but IICIE never set: the level is low, no pend in either
    // mode, and the chain must not even arm.
    let script = [
        (1u64, Op::Write(0x02, KI_C1_MST | KI_C1_TX)), // START, no IICIE
        (1, Op::Write(0x04, 0x1E << 1)),               // address → IICIF (but IICIE off)
        (5, Op::Write(0x04, 0x55)),                    // byte → IICIF
    ];
    assert_walk_identical(&script, 12, "kinetis IICIE-off no pend");
}
