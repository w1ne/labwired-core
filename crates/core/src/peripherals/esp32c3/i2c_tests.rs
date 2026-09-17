use super::*;

#[test]
fn manifest_route_rejects_incomplete_or_invalid_c3_pads() {
    let incomplete = std::collections::BTreeMap::from([("sda".to_string(), "GPIO4".to_string())]);
    assert!(C3I2cPadRoute::from_manifest_route(&incomplete)
        .unwrap_err()
        .to_string()
        .contains("route.sda and route.scl"));

    let non_c3 = std::collections::BTreeMap::from([
        ("sda".to_string(), "PB7".to_string()),
        ("scl".to_string(), "PB6".to_string()),
    ]);
    assert!(C3I2cPadRoute::from_manifest_route(&non_c3)
        .unwrap_err()
        .to_string()
        .contains("route.sda"));

    let duplicate = std::collections::BTreeMap::from([
        ("sda".to_string(), "GPIO4".to_string()),
        ("scl".to_string(), "GPIO4".to_string()),
    ]);
    assert!(C3I2cPadRoute::from_manifest_route(&duplicate)
        .unwrap_err()
        .to_string()
        .contains("distinct pads"));

    let wrong_transport_signal = std::collections::BTreeMap::from([
        ("sda".to_string(), "GPIO4".to_string()),
        ("scl".to_string(), "GPIO5".to_string()),
        ("mosi".to_string(), "GPIO6".to_string()),
    ]);
    assert!(C3I2cPadRoute::from_manifest_route(&wrong_transport_signal)
        .unwrap_err()
        .to_string()
        .contains("route.mosi"));
}

const REG_CMD1_OFFSET: u64 = REG_CMD0 + 4;

// ── Wake-cadence safety: re-anchor while a wake is in flight ─────────────
//
// The scheduler-driven engine schedules its own successor wake. How FAR
// ahead it may schedule is bounded by one thing: a register write can
// re-anchor the engine underneath an in-flight wake, and the engine must
// not then be driven by a wake computed for the state it had BEFORE the
// write.
//
// `CTR.FSM_RST` is the case that reaches this. It parks the engine
// mid-transaction (state -> Idle, ticks_left/acc -> 0) while `scheduled`
// stays true and a wake stays queued (the scheduler has no cancel API by
// design). A following `CTR.TRANS_START` then finds `scheduled == true`
// and arms nothing, so the fresh transaction is driven by the STALE wake.
// `CTR.TRANS_START` alone cannot reach it — `start_transaction` early-returns
// while `engine_active()`.
//
// This is invisible at a one-module-tick cadence (the stale wake is <= 4
// cycles out, so it fires, clears `scheduled`, and TRANS_START re-arms
// cleanly). It becomes a real timing divergence the moment the cadence
// widens toward the next segment transition. Arduino's
// `i2c_ll_master_clr_bus()` writes FSM_RST, so this is a path real firmware
// takes — see `scl_reset_slave_enable_self_clears`.
//
// The reference is the LEGACY PER-CYCLE WALK, which has no wakes at all and
// therefore cannot be wrong about them.

/// Drive a scheduler-mode engine cycle-by-cycle through a real
/// `EventScheduler`, mirroring exactly what `SystemBus`/`Machine` do:
/// publish the clock, arm from `take_scheduled_events` at
/// `now + 1 + delay`, deliver due events, and honour `reschedule_delay`.
// Gated on `event-scheduler`: `uses_scheduler()` is
// `cfg!(feature = "event-scheduler") && clock.is_some()`, so with the
// feature off there is no scheduler path to compare the walk against and
// `SchedDriver::new` would trip its own assert. `core-integrity` runs
// these via `cargo test --release -p labwired-core --features
// event-scheduler --lib`.
#[cfg(feature = "event-scheduler")]
struct SchedDriver {
    dev: Esp32c3I2c,
    sched: crate::sched::EventScheduler,
    clock: CycleClock,
    bus: crate::bus::SystemBus,
    now: u64,
}

#[cfg(feature = "event-scheduler")]
impl SchedDriver {
    fn new() -> Self {
        let mut dev = Esp32c3I2c::new();
        let clock = CycleClock::default();
        clock.publish(0);
        dev.attach_cycle_clock(clock.clone());
        assert!(
            dev.uses_scheduler(),
            "driver must exercise the SCHEDULER path, not the walk"
        );
        Self {
            dev,
            sched: crate::sched::EventScheduler::new(),
            clock,
            bus: crate::bus::SystemBus::new(),
            now: 0,
        }
    }

    fn arm_pending(&mut self) {
        for (delay, token) in self.dev.take_scheduled_events() {
            self.sched.schedule(self.now + 1 + delay, 0, token);
        }
    }

    fn write(&mut self, offset: u64, value: u32) {
        self.dev.sync_to(self.now);
        self.dev.write_u32(offset, value).unwrap();
        self.arm_pending();
    }

    /// Advance one cycle and deliver anything due at the new cycle.
    fn step(&mut self) {
        self.now += 1;
        self.clock.publish(self.now);
        self.sched.advance_to(self.now);
        let mut due = Vec::new();
        self.sched.drain_due_into(&mut due);
        for ev in due {
            let res = self
                .dev
                .on_event(ev.event_token, &mut self.sched, &mut self.bus);
            if let Some(delay) = res.reschedule_delay {
                self.sched
                    .schedule(self.sched.now() + delay, 0, ev.event_token);
            }
        }
        self.arm_pending();
    }
}

/// EVERY attachable I²C device, scheduler path vs the per-cycle walk.
///
/// The wake cadence is a property of the CONTROLLER, but what rides on it is
/// the attached device: a slave decides ACK/NACK and drives read bytes onto
/// SDA within bit timing, so a mis-timed wake shows up as a different
/// waveform, a different RX FIFO, or a different interrupt status — and
/// which of those it shows up as depends on the device.
///
/// The two shipped OLED labs pin only SSD1306, and only its WRITE path.
/// This walks the whole `build_i2c_device` roster through a write-then-
/// repeated-START-read transaction, which is the shape every real sensor
/// driver uses and which the OLED never exercises. Reference is the LEGACY
/// PER-CYCLE WALK, which has no wakes and so cannot be wrong about them.
#[cfg(feature = "event-scheduler")]
#[test]
fn every_attached_i2c_device_matches_the_per_cycle_walk() {
    use crate::peripherals::components::i2c_factory::build_i2c_device;

    // The full `build_i2c_device` roster reachable from a manifest, with the
    // address each answers on. `shm_i2c` is excluded: it is backed by a
    // shared-memory file, not a modelled device.
    const DEVICES: &[(&str, u8)] = &[
        ("tmp102", 0x48),
        ("tmp117", 0x48),
        ("pca9685", 0x40),
        ("mpu6050", 0x68),
        ("bmi270", 0x68),
        ("fxos8700", 0x1E),
        ("bme280", 0x76),
        ("bmp280", 0x76),
        ("aht20", 0x38),
        ("ina219", 0x40),
        ("max30102", 0x57),
        ("cap1188", 0x29),
        ("drv2605", 0x5A),
        ("mlx90640", 0x33),
    ];

    // One SCL period at this timing is ~1600 CPU cycles, and the sequence is
    // ~30 bits INCLUDING a restart from scratch after the FSM_RST, so the
    // budget must cover roughly two full transactions. Sized empirically
    // until the RX FIFO actually fills — see the non-vacuity assert below.
    const PRE_RST: u64 = 1_500;
    const TOTAL: u64 = 400_000;

    let mut covered = 0usize;
    let mut devices_with_rx = 0usize;
    for (name, addr) in DEVICES {
        let cfg = std::collections::HashMap::from([(
            "i2c_address".to_string(),
            serde_yaml::Value::from(*addr as u64),
        )]);
        let Some(_probe) = build_i2c_device(name, &cfg) else {
            panic!(
                "build_i2c_device({name}) returned None — the roster in this \
                     test has drifted from the factory"
            );
        };
        covered += 1;

        // addr+W, register pointer, addr+R — the classic sensor read prologue.
        let tx_bytes: [u32; 3] = [u32::from(*addr) << 1, 0x00, (u32::from(*addr) << 1) | 1];
        // Write a register pointer, then repeated-START read 4 bytes back.
        let program = |write: &mut dyn FnMut(u64, u32)| {
            write(REG_SCL_LOW_PERIOD, 200);
            write(REG_SCL_HIGH_PERIOD, 200);
            write(REG_SDA_HOLD, 40);
            write(REG_SDA_SAMPLE, 40);
            write(REG_SCL_RSTART_SETUP, 200);
            write(REG_SCL_STOP_SETUP, 200);
            write(REG_SCL_STOP_HOLD, 200);
            write(REG_CMD0, cmd(CMD_RSTART, 0));
            write(REG_CMD1_OFFSET, cmd(CMD_WRITE, 2));
            write(REG_CMD1_OFFSET + 4, cmd(CMD_RSTART, 0));
            write(REG_CMD1_OFFSET + 8, cmd(CMD_WRITE, 1));
            write(REG_CMD1_OFFSET + 12, cmd(CMD_READ, 4));
            write(REG_CMD1_OFFSET + 16, cmd(CMD_STOP, 0));
            for b in tx_bytes {
                write(REG_DATA, b);
            }
        };

        // ── reference: per-cycle walk ──
        let mut walk = Esp32c3I2c::new();
        walk.push_slave(build_i2c_device(name, &cfg).unwrap());
        assert!(!walk.uses_scheduler());
        let walk_lines = walk.line_levels_arc();
        {
            let mut w = |o: u64, v: u32| {
                walk.write_u32(o, v).unwrap();
            };
            program(&mut w);
        }
        walk.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();
        let mut walk_wave = Vec::with_capacity(TOTAL as usize);
        for c in 1..=TOTAL {
            walk.tick_elapsed(1);
            if c == PRE_RST {
                // What a real driver does on a bus reset: clear the FSM,
                // re-prime the TX FIFO (FSM_RST drains it), restart.
                walk.write_u32(REG_CTR, CTR_FSM_RST).unwrap();
                walk.write_u32(REG_INT_CLR, u32::MAX).unwrap();
                for b in tx_bytes {
                    walk.write_u32(REG_DATA, b).unwrap();
                }
                walk.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();
            }
            walk_wave.push((walk_lines.scl(), walk_lines.sda()));
        }

        // ── under test: scheduler-driven ──
        let mut sd = SchedDriver::new();
        sd.dev.push_slave(build_i2c_device(name, &cfg).unwrap());
        let sched_lines = sd.dev.line_levels_arc();
        {
            let mut pending: Vec<(u64, u32)> = Vec::new();
            let mut w = |o: u64, v: u32| pending.push((o, v));
            program(&mut w);
            for (o, v) in pending {
                sd.write(o, v);
            }
        }
        sd.write(REG_CTR, CTR_TRANS_START_BIT);
        let mut sched_wave = Vec::with_capacity(TOTAL as usize);
        for c in 1..=TOTAL {
            sd.step();
            if c == PRE_RST {
                sd.write(REG_CTR, CTR_FSM_RST);
                sd.write(REG_INT_CLR, u32::MAX);
                for b in tx_bytes {
                    sd.write(REG_DATA, b);
                }
                sd.write(REG_CTR, CTR_TRANS_START_BIT);
            }
            sched_wave.push((sched_lines.scl(), sched_lines.sda()));
        }

        // A device that never got clocked proves nothing about wakes.
        let edges = walk_wave.windows(2).filter(|w| w[0] != w[1]).count();
        assert!(
            edges > 8,
            "{name}: reference waveform has only {edges} edges — the \
                 transaction never clocked, so this row is vacuous"
        );

        if let Some(c) = (0..TOTAL as usize).find(|&i| walk_wave[i] != sched_wave[i]) {
            panic!(
                "{name} @ {addr:#04x}: scheduler waveform diverges from the \
                     per-cycle walk at cycle {}: walk={:?} sched={:?}",
                c + 1,
                walk_wave[c],
                sched_wave[c]
            );
        }

        // The bytes the controller actually captured must match too — the
        // waveform is what the pads saw, the FIFO is what firmware reads.
        assert_eq!(
            walk.read_u32(REG_SR).unwrap(),
            sd.dev.read_u32(REG_SR).unwrap(),
            "{name}: SR diverges"
        );
        assert_eq!(walk.int_raw, sd.dev.int_raw, "{name}: INT_RAW diverges");
        let walk_rx: Vec<u8> = walk.core.rx_bytes();
        let sched_rx: Vec<u8> = sd.dev.core.rx_bytes();
        assert_eq!(walk_rx, sched_rx, "{name}: RX FIFO diverges");
        if !walk_rx.is_empty() {
            devices_with_rx += 1;
        }
    }

    assert_eq!(
        covered,
        DEVICES.len(),
        "every rostered device must have been exercised"
    );
    // NON-VACUITY. Comparing two empty RX FIFOs proves nothing about read
    // timing, and that is exactly what this test did on its first draft —
    // the budget was too small for the transaction to reach the READ at all,
    // so all 14 rows compared `[] == []` and passed. Most of these devices
    // answer a register read; require that the read path actually produced
    // bytes for a solid majority of the roster.
    assert!(
            devices_with_rx * 2 >= DEVICES.len(),
            "only {devices_with_rx}/{} devices returned any RX bytes — the READ              phase is not being reached, so the FIFO comparison is vacuous",
            DEVICES.len()
        );
}

/// Program the same short write transaction into either engine.
#[cfg(feature = "event-scheduler")]
fn program_write_txn(write: &mut dyn FnMut(u64, u32)) {
    // ~100 kHz-shaped timing: long SCL half-periods, so one wire segment is
    // ~200 module ticks (~800 CPU cycles at module clk = CPU/4). Default
    // reset timing gives ~9-tick segments, which is far too short for a
    // widened wake to be observably stale.
    write(REG_SCL_LOW_PERIOD, 200);
    write(REG_SCL_HIGH_PERIOD, 200);
    write(REG_SDA_HOLD, 40);
    write(REG_SDA_SAMPLE, 40);
    write(REG_SCL_RSTART_SETUP, 200);
    write(REG_SCL_STOP_SETUP, 200);
    write(REG_SCL_STOP_HOLD, 200);
    write(REG_CMD0, cmd(CMD_RSTART, 0));
    write(REG_CMD1_OFFSET, cmd(CMD_WRITE, 2));
    write(REG_CMD1_OFFSET + 4, cmd(CMD_STOP, 0));
    write(REG_DATA, 0x3C << 1);
    write(REG_DATA, 0xA5);
}

/// The SCL/SDA waveform, sampled every cycle, is the observable this gate
/// compares: it is what the GPIO matrix publishes to routed pads and what
/// the logic analyzer captures.
#[cfg(feature = "event-scheduler")]
#[test]
fn rearm_after_fsm_rst_matches_the_per_cycle_walk() {
    const PRE_RST: u64 = 1_500;
    const TOTAL: u64 = 60_000;

    // ── reference: legacy per-cycle walk (no scheduler, no wakes) ──
    let mut walk = Esp32c3I2c::new();
    assert!(
        !walk.uses_scheduler(),
        "reference must be the per-cycle walk"
    );
    let walk_lines = walk.line_levels_arc();
    {
        let mut w = |o: u64, v: u32| {
            walk.write_u32(o, v).unwrap();
        };
        program_write_txn(&mut w);
    }
    walk.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();
    let mut walk_wave = Vec::with_capacity(TOTAL as usize);
    for c in 1..=TOTAL {
        walk.tick_elapsed(1);
        if c == PRE_RST {
            // Abort mid-transaction, then immediately restart it.
            walk.write_u32(REG_CTR, CTR_FSM_RST).unwrap();
            walk.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();
        }
        walk_wave.push((walk_lines.scl(), walk_lines.sda()));
    }

    // ── under test: scheduler-driven ──
    let mut sd = SchedDriver::new();
    let sched_lines = sd.dev.line_levels_arc();
    {
        let mut pending: Vec<(u64, u32)> = Vec::new();
        let mut w = |o: u64, v: u32| pending.push((o, v));
        program_write_txn(&mut w);
        for (o, v) in pending {
            sd.write(o, v);
        }
    }
    sd.write(REG_CTR, CTR_TRANS_START_BIT);
    let mut sched_wave = Vec::with_capacity(TOTAL as usize);
    for c in 1..=TOTAL {
        sd.step();
        if c == PRE_RST {
            sd.write(REG_CTR, CTR_FSM_RST);
            sd.write(REG_CTR, CTR_TRANS_START_BIT);
        }
        sched_wave.push((sched_lines.scl(), sched_lines.sda()));
    }

    // The waveform must actually move, or this gate proves nothing.
    let edges = walk_wave.windows(2).filter(|w| w[0] != w[1]).count();
    assert!(
        edges > 8,
        "reference waveform has only {edges} edges — the transaction did not clock"
    );

    if let Some(c) = (0..TOTAL as usize).find(|&i| walk_wave[i] != sched_wave[i]) {
        panic!(
            "scheduler waveform diverges from the per-cycle walk at cycle {}: \
                 walk={:?} sched={:?}. A wake armed before the FSM_RST/TRANS_START \
                 re-anchor drove the engine after it — widen the wake cadence only \
                 with an arming token that kills the superseded chain.",
            c + 1,
            walk_wave[c],
            sched_wave[c]
        );
    }
}

/// ABSOLUTE golden for the pad-publication layer.
///
/// The neighbouring walk-vs-scheduler gate is DIFFERENTIAL: both paths
/// publish through the same cell, so a change to the publication layer that
/// shifted every level identically would pass it. This pins the actual
/// waveform — the exact per-cycle SCL/SDA the pads see for one fixed
/// transaction — so refactoring how levels reach the pad cannot silently
/// move an edge. Register offsets are spelled from the TRM rather than the
/// module's own constants, so the expectation is independent of them.
#[test]
fn published_scl_sda_waveform_is_byte_identical() {
    const REG_SCL_LOW: u64 = 0x00;
    const REG_CTR_: u64 = 0x04;
    const REG_DATA_: u64 = 0x1C;
    const REG_SDA_HOLD_: u64 = 0x30;
    const REG_SDA_SAMPLE_: u64 = 0x34;
    const REG_SCL_HIGH: u64 = 0x38;
    const REG_RSTART_SETUP: u64 = 0x44;
    const REG_STOP_HOLD: u64 = 0x48;
    const REG_STOP_SETUP: u64 = 0x4C;
    const REG_CMD0_: u64 = 0x58;
    const TRANS_START: u32 = 1 << 5;
    let enc = |opcode: u32, bytes: u32| (opcode << 11) | bytes;

    let mut i2c = Esp32c3I2c::new();
    let lines = i2c.line_levels_arc();
    assert!(
        lines.scl() && lines.sda(),
        "an idle open-drain bus rests high before anything drives it",
    );
    for (offset, value) in [
        (REG_SCL_LOW, 200),
        (REG_SCL_HIGH, 200),
        (REG_SDA_HOLD_, 40),
        (REG_SDA_SAMPLE_, 40),
        (REG_RSTART_SETUP, 200),
        (REG_STOP_SETUP, 200),
        (REG_STOP_HOLD, 200),
        (REG_CMD0_, enc(6, 0)),     // RSTART
        (REG_CMD0_ + 4, enc(1, 2)), // WRITE 2
        (REG_CMD0_ + 8, enc(2, 0)), // STOP
        (REG_DATA_, 0x3C << 1),
        (REG_DATA_, 0xA5),
    ] {
        i2c.write_u32(offset, value).unwrap();
    }
    i2c.write_u32(REG_CTR_, TRANS_START).unwrap();

    // Transitions only, stamped with the cycle they happened on: this is
    // exactly what the logic analyzer records off these pads.
    let mut edges: Vec<(u32, char, bool)> = Vec::new();
    // Sampled from wherever the START left the wire, not from idle: the
    // TRANS_START write itself drives the first edge.
    let (mut scl, mut sda) = (lines.scl(), lines.sda());
    for cycle in 1..=40_000u32 {
        i2c.tick_elapsed(1);
        let (next_scl, next_sda) = (lines.scl(), lines.sda());
        if next_scl != scl {
            edges.push((cycle, 'C', next_scl));
            scl = next_scl;
        }
        if next_sda != sda {
            edges.push((cycle, 'D', next_sda));
            sda = next_sda;
        }
    }

    // FNV-1a over the whole edge stream — one number that moves if any
    // edge moves, appears, or disappears.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for (cycle, line, level) in &edges {
        for byte in cycle
            .to_le_bytes()
            .iter()
            .chain([*line as u8, *level as u8].iter())
        {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    println!("GOLDEN edges={} hash={hash:#018x}", edges.len());
    assert!(
        edges.len() > 40,
        "only {} edges — the transaction never clocked, so this gate is \
             vacuous",
        edges.len()
    );
    assert_eq!(
        (edges.len(), hash),
        (EXPECTED_EDGES, EXPECTED_HASH),
        "the published SCL/SDA waveform changed"
    );
}
/// Captured from the pre-`PadLines` implementation and re-verified against
/// it after the port (see the module doc on `pad_lines`): 49 transitions,
/// FNV-1a over (cycle, line, level).
const EXPECTED_EDGES: usize = 49;
const EXPECTED_HASH: u64 = 0x5ac4_2643_150f_4e32;

/// Encode a 14-bit command word: opcode | byte_num.
fn cmd(opcode: u8, byte_num: u8) -> u32 {
    ((opcode as u32 & 0x7) << 11) | (byte_num as u32)
}

// ESP32-C3 TRM §16: 1=WRITE, 2=STOP, 3=READ, 4=END, 6=RSTART.
const CMD_WRITE: u8 = 1;
const CMD_STOP: u8 = 2;
const CMD_READ: u8 = 3;
const CMD_END: u8 = 4;
const CMD_RSTART: u8 = 6;

/// Clock the bit engine to completion (command lists execute over
/// simulated cycles now, not synchronously on the TRANS_START write).
fn run_engine(p: &mut Esp32c3I2c) {
    for _ in 0..1_000_000 {
        if !p.engine_active() {
            return;
        }
        p.tick_elapsed(64);
    }
    panic!("C3 I2C bit engine did not complete");
}

/// Kick TRANS_START and clock the engine until it parks (STOP complete,
/// END pause, or list termination).
fn start_and_run(p: &mut Esp32c3I2c) {
    p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();
    run_engine(p);
}

#[test]
fn i2c0_interrupt_source_is_29_not_42() {
    // C3-vs-S3 difference: the C3 routes I2C_EXT0 through interrupt-matrix
    // source 29 (I2C_EXT0_INTR_MAP at offset 116 = 4*29), NOT the S3's 42.
    assert_eq!(I2C0_INTR_SOURCE_ID, 29);
}

#[test]
fn ctr_round_trip() {
    let mut p = Esp32c3I2c::new();
    p.write_u32(REG_CTR, 0x0000_0010).unwrap(); // arbitrary, no TRANS_START
    assert_eq!(p.read_u32(REG_CTR).unwrap(), 0x0000_0010);
}

#[test]
fn slave_addr_round_trip() {
    let mut p = Esp32c3I2c::new();
    p.write_u32(REG_SLAVE_ADDR, 0x48).unwrap();
    assert_eq!(p.read_u32(REG_SLAVE_ADDR).unwrap(), 0x48);
}

#[test]
fn cmd_registers_round_trip() {
    let mut p = Esp32c3I2c::new();
    p.write_u32(REG_CMD0, 0x0000_0800).unwrap();
    p.write_u32(REG_CMD7, 0x0000_2000).unwrap();
    assert_eq!(p.read_u32(REG_CMD0).unwrap(), 0x0000_0800);
    assert_eq!(p.read_u32(REG_CMD7).unwrap(), 0x0000_2000);
}

#[test]
fn sr_txfifo_cnt_reflects_pushes() {
    let mut p = Esp32c3I2c::new();
    p.write_u32(REG_DATA, 0xAA).unwrap();
    p.write_u32(REG_DATA, 0xBB).unwrap();
    p.write_u32(REG_DATA, 0xCC).unwrap();
    let sr = p.read_u32(REG_SR).unwrap();
    assert_eq!(
        (sr >> 18) & 0x3F,
        3,
        "SR.txfifo_cnt should reflect 3 pushes"
    );
}

#[test]
fn fifo_reset_bits_clear_fifos() {
    let mut p = Esp32c3I2c::new();
    p.write_u32(REG_DATA, 0x11).unwrap();
    p.write_u32(REG_DATA, 0x22).unwrap();
    p.write_u32(REG_FIFO_CONF, 1 << 13).unwrap(); // TX_FIFO_RST
    let sr = p.read_u32(REG_SR).unwrap();
    assert_eq!((sr >> 18) & 0x3F, 0);
}

#[test]
fn int_clr_clears_specified_bits() {
    let mut p = Esp32c3I2c::new();
    p.int_raw = INT_TRANS_COMPLETE | INT_NACK;
    p.write_u32(REG_INT_CLR, INT_NACK).unwrap();
    assert_eq!(p.read_u32(REG_INT_RAW).unwrap(), INT_TRANS_COMPLETE);
}

#[test]
fn int_st_masks_with_int_ena() {
    let mut p = Esp32c3I2c::new();
    p.int_raw = INT_TRANS_COMPLETE | INT_NACK;
    assert!(
        !p.legacy_tick_active(),
        "disabled C3 I2C level IRQs must stay out of the legacy tick walk"
    );
    assert!(
        p.legacy_tick_dynamic(),
        "C3 I2C updates tick membership when INT_ST changes"
    );
    p.write_u32(REG_INT_ENA, INT_TRANS_COMPLETE).unwrap();
    assert_eq!(p.read_u32(REG_INT_ST).unwrap(), INT_TRANS_COMPLETE);
    assert!(
        p.legacy_tick_active(),
        "enabled C3 I2C level IRQ must re-enter the legacy tick walk"
    );
    p.write_u32(REG_INT_CLR, INT_TRANS_COMPLETE).unwrap();
    assert!(
        !p.legacy_tick_active(),
        "cleared C3 I2C level IRQ must leave the legacy tick walk"
    );
}

#[test]
fn end_opcode_raises_end_detect_not_trans_complete() {
    let mut p = Esp32c3I2c::new();
    p.write_u32(REG_CMD0, cmd(CMD_END, 0)).unwrap();
    start_and_run(&mut p);
    let int_raw = p.read_u32(REG_INT_RAW).unwrap();
    assert_eq!(
        int_raw & INT_END_DETECT,
        INT_END_DETECT,
        "END must raise END_DETECT"
    );
    assert_eq!(
        int_raw & INT_TRANS_COMPLETE,
        0,
        "END must NOT raise TRANS_COMPLETE"
    );
}

#[test]
fn rstart_then_stop_completes() {
    let mut p = Esp32c3I2c::new();
    p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
    p.write_u32(REG_CMD1_OFFSET, cmd(CMD_STOP, 0)).unwrap();
    start_and_run(&mut p);
    assert_eq!(
        p.read_u32(REG_INT_RAW).unwrap() & INT_TRANS_COMPLETE,
        INT_TRANS_COMPLETE
    );
}

#[test]
fn trans_start_auto_clears() {
    let mut p = Esp32c3I2c::new();
    p.write_u32(REG_CMD0, cmd(CMD_END, 0)).unwrap();
    start_and_run(&mut p);
    assert_eq!(p.read_u32(REG_CTR).unwrap() & CTR_TRANS_START_BIT, 0);
}

#[test]
fn one_shot_control_bits_auto_clear() {
    let mut p = Esp32c3I2c::new();
    p.write_u32(REG_CTR, CTR_FSM_RST | CTR_CONF_UPGATE).unwrap();
    assert_eq!(
        p.read_u32(REG_CTR).unwrap() & (CTR_FSM_RST | CTR_CONF_UPGATE),
        0
    );
}

#[test]
fn scl_reset_slave_enable_self_clears() {
    let mut p = Esp32c3I2c::new();
    // Exact value observed in Arduino's i2c_ll_master_clr_bus(): enable
    // plus 9 SCL pulses encoded in SCL_RST_SLV_NUM bits [5:1].
    p.write_u32(REG_SCL_SP_CONF, 0x13).unwrap();
    assert_eq!(
        p.read_u32(REG_SCL_SP_CONF).unwrap(),
        0x12,
        "SCL_RST_SLV_EN must self-clear while preserving pulse count"
    );
}

#[test]
fn txfifo_start_addr_window_peeks_tx_fifo_non_destructively() {
    let mut p = Esp32c3I2c::new();
    assert_eq!(
        p.read_u32(REG_TXFIFO_START).unwrap(),
        0,
        "empty TX FIFO reads 0"
    );
    p.write_u32(REG_DATA, 0xAA).unwrap();
    p.write_u32(REG_DATA, 0xBB).unwrap();
    assert_eq!(p.read_u32(REG_TXFIFO_START).unwrap(), 0xAA);
    assert_eq!(
        p.read_u32(REG_TXFIFO_START).unwrap(),
        0xAA,
        "peek is non-destructive"
    );
    let sr = p.read_u32(REG_SR).unwrap();
    assert_eq!((sr >> 18) & 0x3F, 2, "peek must not consume TX FIFO bytes");
}

#[test]
fn write_with_unmatched_address_sets_nack_int() {
    let mut p = Esp32c3I2c::new();
    // No slaves attached.
    p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
    p.write_u32(REG_CMD0 + 4, cmd(CMD_WRITE, 1)).unwrap();
    p.write_u32(REG_CMD0 + 8, cmd(CMD_STOP, 0)).unwrap();
    p.write_u32(REG_DATA, 0xA0).unwrap(); // some addr+W, no slave
    start_and_run(&mut p);
    assert_eq!(
        p.read_u32(REG_INT_RAW).unwrap() & INT_NACK,
        INT_NACK,
        "INT_NACK should fire when no slave matches"
    );
}

#[test]
fn config_registers_reset_values_match_c3_yaml() {
    let p = Esp32c3I2c::new();
    assert_eq!(
        p.read_u32(REG_CTR).unwrap(),
        0x0000_020B,
        "CTR reset (yaml 523)"
    );
    assert_eq!(
        p.read_u32(REG_FIFO_CONF).unwrap(),
        0x0000_408B,
        "FIFO_CONF (yaml 16523)"
    );
    assert_eq!(p.read_u32(REG_TO).unwrap(), 0x0000_0010, "TO (yaml 16)");
    assert_eq!(
        p.read_u32(REG_SCL_START_HOLD).unwrap(),
        0x0000_0008,
        "SCL_START_HOLD (yaml 8)"
    );
    assert_eq!(
        p.read_u32(REG_FILTER_CFG).unwrap(),
        0x0000_0300,
        "FILTER_CFG (yaml 768)"
    );
    assert_eq!(
        p.read_u32(REG_CLK_CONF).unwrap(),
        0x0020_0000,
        "CLK_CONF (yaml 2097152)"
    );
    assert_eq!(
        p.read_u32(REG_DATE).unwrap(),
        0x2007_0201,
        "DATE (yaml 537330177)"
    );
    let sr = p.read_u32(REG_SR).unwrap();
    assert_eq!(
        sr & 0x0000_C000,
        0x0000_C000,
        "SR STRETCH_CAUSE (yaml 49152)"
    );
    assert_eq!(
        p.read_u32(REG_INT_RAW).unwrap() & 0x2,
        0x2,
        "INT_RAW TXFIFO_WM (yaml 2)"
    );
}

// ── Headline test: an attached I2cDevice round-trips a write-then-read
//    transaction driven exactly as C3 firmware would. Uses the Bmp280
//    register-pointer device (an existing I2cDevice).

use crate::peripherals::components::Bmp280;

#[test]
fn write_read_drives_attached_bmp280() {
    let mut p = Esp32c3I2c::new();
    // Default address 0x76.
    p.push_slave(Box::new(Bmp280::new(0x76)));

    // Canonical register-pointer read: set pointer to 0xD0 (chip-id), then
    // repeated-start and read one byte. CHIP_ID for BMP280 is 0x58.
    //   RSTART; WRITE 2 (addr+W, pointer=0xD0); RSTART;
    //   WRITE 1 (addr+R); READ 1; STOP.
    p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
    p.write_u32(REG_CMD0 + 4, cmd(CMD_WRITE, 2)).unwrap();
    p.write_u32(REG_CMD0 + 8, cmd(CMD_RSTART, 0)).unwrap();
    p.write_u32(REG_CMD0 + 12, cmd(CMD_WRITE, 1)).unwrap();
    p.write_u32(REG_CMD0 + 16, cmd(CMD_READ, 1)).unwrap();
    p.write_u32(REG_CMD0 + 20, cmd(CMD_STOP, 0)).unwrap();

    // Push TX bytes: addr+W (0x76<<1=0xEC), pointer 0xD0, addr+R (0xED).
    p.write_u32(REG_DATA, 0xEC).unwrap();
    p.write_u32(REG_DATA, 0xD0).unwrap();
    p.write_u32(REG_DATA, 0xED).unwrap();

    start_and_run(&mut p);

    // Address must have matched (no NACK).
    assert_eq!(
        p.read_u32(REG_INT_RAW).unwrap() & INT_NACK,
        0,
        "BMP280 at 0x76 must ACK its address"
    );
    // Slave acked → RESP_REC set in SR.
    assert_eq!(
        p.read_u32(REG_SR).unwrap() & SR_RESP_REC,
        SR_RESP_REC,
        "SR.RESP_REC must be set after a successful transaction"
    );
    // The chip-id byte 0x58 should be in the RX FIFO.
    assert_eq!(
        p.read_u32(REG_DATA).unwrap(),
        0x58,
        "BMP280 CHIP_ID round-trip"
    );
    // STOP completed the transaction.
    assert_eq!(
        p.read_u32(REG_INT_RAW).unwrap() & INT_TRANS_COMPLETE,
        INT_TRANS_COMPLETE
    );
}

#[test]
fn inspect_ssd1306_framebuffer_reports_ink_metrics() {
    use crate::inspect::InspectOpts;
    use crate::peripherals::components::Ssd1306;

    let mut p = Esp32c3I2c::new();
    p.push_slave(Box::new(Ssd1306::new(0x3C)));

    // Same transaction shape as the C3 OLED firmware:
    // RSTART; WRITE 3 (addr+W, control=0x40, one framebuffer byte); STOP.
    p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
    p.write_u32(REG_CMD0 + 4, cmd(CMD_WRITE, 3)).unwrap();
    p.write_u32(REG_CMD0 + 8, cmd(CMD_STOP, 0)).unwrap();
    p.write_u32(REG_DATA, 0x78).unwrap(); // 0x3C << 1, write
    p.write_u32(REG_DATA, 0x40).unwrap(); // SSD1306 data stream
    p.write_u32(REG_DATA, 0xAA).unwrap(); // four lit pixels in byte 0
    start_and_run(&mut p);

    let pi = p.inspect(0x6001_3000, "i2c0", &InspectOpts::default());
    let fb = pi
        .artifacts
        .iter()
        .find(|a| a.kind == "framebuffer")
        .expect("framebuffer artifact present");
    assert_eq!(fb.meta["ink_bytes"], 1);
    assert_eq!(fb.meta["lit_pixels"], 4);
}

#[test]
fn register_addressed_write_delivers_payload_to_ssd1306() {
    use crate::inspect::InspectOpts;
    use crate::peripherals::components::Ssd1306;

    let mut p = Esp32c3I2c::new();
    p.push_slave(Box::new(Ssd1306::new(0x3C)));

    // Arduino-ESP32 / ESP-IDF may program SLAVE_ADDR with addr<<1 and
    // write only the SSD1306 payload bytes to TXFIFO: control byte 0x40,
    // then data 0xAA.
    p.write_u32(REG_SLAVE_ADDR, 0x3C << 1).unwrap();
    p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
    p.write_u32(REG_CMD0 + 4, cmd(CMD_WRITE, 2)).unwrap();
    p.write_u32(REG_CMD0 + 8, cmd(CMD_STOP, 0)).unwrap();
    p.write_u32(REG_DATA, 0x40).unwrap();
    p.write_u32(REG_DATA, 0xAA).unwrap();
    start_and_run(&mut p);

    assert_eq!(p.read_u32(REG_INT_RAW).unwrap() & INT_NACK, 0);
    let pi = p.inspect(0x6001_3000, "i2c0", &InspectOpts::default());
    let fb = pi
        .artifacts
        .iter()
        .find(|a| a.kind == "framebuffer")
        .expect("framebuffer artifact present");
    assert_eq!(fb.meta["ink_bytes"], 1);
    assert_eq!(fb.meta["lit_pixels"], 4);
}

#[test]
fn end_paused_address_phase_carries_active_slave() {
    use crate::inspect::InspectOpts;
    use crate::peripherals::components::Ssd1306;

    let mut p = Esp32c3I2c::new();
    p.push_slave(Box::new(Ssd1306::new(0x3C)));

    // Arduino-ESP32 splits a write: address phase ends with END_DETECT,
    // then payload bytes are sent by a second command-list run.
    p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
    p.write_u32(REG_CMD0 + 4, cmd(CMD_WRITE, 1)).unwrap();
    p.write_u32(REG_CMD0 + 8, cmd(CMD_END, 0)).unwrap();
    p.write_u32(REG_DATA, 0x78).unwrap();
    start_and_run(&mut p);
    assert_eq!(
        p.read_u32(REG_INT_RAW).unwrap() & INT_END_DETECT,
        INT_END_DETECT
    );
    p.write_u32(REG_INT_CLR, INT_END_DETECT).unwrap();

    p.write_u32(REG_CMD0, cmd(CMD_WRITE, 2)).unwrap();
    p.write_u32(REG_CMD0 + 4, cmd(CMD_STOP, 0)).unwrap();
    p.write_u32(REG_CMD0 + 8, 0).unwrap();
    p.write_u32(REG_DATA, 0x40).unwrap();
    p.write_u32(REG_DATA, 0xAA).unwrap();
    start_and_run(&mut p);

    assert_eq!(p.read_u32(REG_INT_RAW).unwrap() & INT_NACK, 0);
    let pi = p.inspect(0x6001_3000, "i2c0", &InspectOpts::default());
    let fb = pi
        .artifacts
        .iter()
        .find(|a| a.kind == "framebuffer")
        .expect("framebuffer artifact present");
    assert_eq!(fb.meta["ink_bytes"], 1);
    assert_eq!(fb.meta["lit_pixels"], 4);
}

#[test]
fn write_then_read_calibration_block_round_trip() {
    // Read the 24-byte calibration block starting at 0x88 — exercises a
    // multi-byte READ pulling sequential register-pointer data through the
    // RX FIFO.
    let mut p = Esp32c3I2c::new();
    p.push_slave(Box::new(Bmp280::new(0x76)));

    p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
    p.write_u32(REG_CMD0 + 4, cmd(CMD_WRITE, 2)).unwrap();
    p.write_u32(REG_CMD0 + 8, cmd(CMD_RSTART, 0)).unwrap();
    p.write_u32(REG_CMD0 + 12, cmd(CMD_WRITE, 1)).unwrap();
    p.write_u32(REG_CMD0 + 16, cmd(CMD_READ, 4)).unwrap();
    p.write_u32(REG_CMD0 + 20, cmd(CMD_STOP, 0)).unwrap();

    p.write_u32(REG_DATA, 0xEC).unwrap(); // addr+W
    p.write_u32(REG_DATA, 0x88).unwrap(); // pointer = calib start
    p.write_u32(REG_DATA, 0xED).unwrap(); // addr+R
    start_and_run(&mut p);

    // First four calibration bytes per the Bosch reference block.
    assert_eq!(p.read_u32(REG_DATA).unwrap(), 0x70);
    assert_eq!(p.read_u32(REG_DATA).unwrap(), 0x6B);
    assert_eq!(p.read_u32(REG_DATA).unwrap(), 0x43);
    assert_eq!(p.read_u32(REG_DATA).unwrap(), 0x67);
}

/// The headline fidelity contract: TRANS_COMPLETE does NOT assert on the
/// TRANS_START write. The transaction clocks over simulated cycles at the
/// rate the (reset-default) clock registers dictate, SR.BUS_BUSY reads 1
/// on the wire, and completion lands at the exact analytically-derived
/// cycle.
#[test]
fn trans_complete_asserts_at_derived_wire_time_not_instantly() {
    let mut p = Esp32c3I2c::new();
    p.push_slave(Box::new(Bmp280::new(0x76)));
    p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
    p.write_u32(REG_CMD0 + 4, cmd(CMD_WRITE, 2)).unwrap();
    p.write_u32(REG_CMD0 + 8, cmd(CMD_STOP, 0)).unwrap();
    p.write_u32(REG_DATA, 0xEC).unwrap(); // addr+W
    p.write_u32(REG_DATA, 0xD0).unwrap(); // pointer
    p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();

    assert_eq!(
        p.read_u32(REG_INT_RAW).unwrap() & INT_TRANS_COMPLETE,
        0,
        "TRANS_COMPLETE must not assert instantly on TRANS_START"
    );
    assert!(p.engine_active(), "engine must be clocking the wire");
    assert_eq!(
        p.read_u32(REG_SR).unwrap() & SR_BUS_BUSY,
        SR_BUS_BUSY,
        "SR.BUS_BUSY must read 1 while the transaction is on the wire"
    );

    let mut cycles = 0u64;
    while p.engine_active() {
        p.tick_elapsed(1);
        cycles += 1;
        assert!(cycles < 10_000_000, "engine never completed");
    }
    // Reset-default timing (datasheet reset values, firmware programmed
    // nothing): module tick = 4 engine cycles (XTAL 40 MHz, divider 1, on
    // the 160 MHz cycle base). Wire time in module ticks:
    //   START:  SCL_START_HOLD 8+1                       =  9
    //   bits:   2 bytes x 9 bits x (low 0+1 + high 0+0+1) = 36
    //   STOP:   low 1 + SCL_STOP_SETUP 8+1 + SCL_STOP_HOLD 8+1 = 19
    // total = 64 module ticks = 256 engine cycles.
    assert_eq!(
        cycles, 256,
        "completion time must derive from the registers"
    );
    assert_eq!(
        p.read_u32(REG_INT_RAW).unwrap() & INT_TRANS_COMPLETE,
        INT_TRANS_COMPLETE
    );
    assert_eq!(p.read_u32(REG_SR).unwrap() & SR_BUS_BUSY, 0);
}

/// Timing derivation follows the PROGRAMMED registers: a 100 kHz-style
/// configuration (as esp-hal would write) stretches the same transaction
/// accordingly. SCL period = (low + high) module ticks; all counters use
/// the TRM's `reg + 1` semantics.
#[test]
fn scl_timing_follows_programmed_registers() {
    let mut p = Esp32c3I2c::new();
    // 400-tick SCL period at 40 MHz module clock = 100 kHz.
    p.write_u32(REG_SCL_LOW_PERIOD, 199).unwrap(); // low = 200 ticks
    p.write_u32(REG_SCL_HIGH_PERIOD, 180 | (19 << 9)).unwrap(); // high = 200
    p.write_u32(REG_SDA_HOLD, 29).unwrap(); // 30 ticks
    p.write_u32(REG_SCL_START_HOLD, 199).unwrap();
    p.write_u32(REG_SCL_STOP_SETUP, 199).unwrap();
    p.write_u32(REG_SCL_STOP_HOLD, 199).unwrap();

    // One-byte write to an absent slave (NACK still clocks all 9 bits).
    p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
    p.write_u32(REG_CMD0 + 4, cmd(CMD_WRITE, 1)).unwrap();
    p.write_u32(REG_CMD0 + 8, cmd(CMD_STOP, 0)).unwrap();
    p.write_u32(REG_DATA, 0xA0).unwrap();
    p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();

    let mut cycles = 0u64;
    while p.engine_active() {
        p.tick_elapsed(1);
        cycles += 1;
        assert!(cycles < 10_000_000, "engine never completed");
    }
    // START 200 + 9 bits x 400 + STOP (200 low + 200 setup + 200 hold)
    // = 4400 module ticks x 4 engine cycles = 17600 cycles.
    assert_eq!(cycles, 17_600);
    assert_eq!(
        p.read_u32(REG_INT_RAW).unwrap() & INT_NACK,
        INT_NACK,
        "absent slave must NACK"
    );
}

#[test]
fn set_bus_trace_records_transactions_for_attached_slaves() {
    use crate::peripherals::components::Bmp280;

    let log = crate::bus::bus_trace::new_log();
    let mut p = Esp32c3I2c::new();
    // The bus choke point wraps before push; emulate it here.
    p.push_slave(crate::bus::bus_trace::wrap_i2c(
        "i2c0",
        &log,
        Box::new(Bmp280::new(0x76)),
    ));

    // Same canonical pointer-write transaction as
    // write_read_drives_attached_bmp280: RSTART; WRITE 2; STOP.
    p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
    p.write_u32(REG_CMD0 + 4, cmd(CMD_WRITE, 2)).unwrap();
    p.write_u32(REG_CMD0 + 8, cmd(CMD_STOP, 0)).unwrap();
    p.write_u32(REG_DATA, 0xEC).unwrap(); // addr+W
    p.write_u32(REG_DATA, 0xD0).unwrap(); // pointer
    start_and_run(&mut p);

    let events = log.snapshot();
    assert!(
        !events.is_empty(),
        "tracing wrapper must record I2C traffic on the C3 controller"
    );
    assert!(events.iter().all(|e| e.bus == "i2c0"));
    // The controller must signal START at address match so the trace
    // carries a decodable address frame, not just raw data bytes.
    assert!(
        events.iter().any(|e| matches!(
            &e.payload,
            crate::bus::bus_trace::BusPayload::I2c {
                kind: crate::bus::bus_trace::I2cSym::AddrWrite,
                ..
            }
        )),
        "trace must contain an address frame for transaction decode"
    );
}

/// A realistic SSD1306 pixel-data burst: four full GDDRAM pages
/// (128×4 = 512 data bytes) streamed the way a display driver does — each
/// transfer is far larger than the 32-byte TX FIFO, so the FIFO underruns
/// and must be refilled mid-WRITE (the watermark / OP_END refill the IDF and
/// Arduino I²C drivers rely on).
///
/// The real ESP32-C3 controller holds SCL low (clock-stretch) on a TX-FIFO
/// underrun and resumes when firmware refills; it NEVER invents a 0x00. A
/// model that pops a spurious 0x00 on underrun (`pop_front().unwrap_or(0)`)
/// clocks bogus bytes into the panel — the extra pixels land in GDDRAM as
/// zeros (and shift every real byte that follows), so the OLED reads back an
/// all-but-blank framebuffer even though the CPU/serial/LED are healthy.
///
/// Every existing OLED test only ever sends a 2–3 byte prologue that fits in
/// one FIFO load, so this multi-chunk burst is the first coverage of the
/// underrun-refill path.
#[test]
fn multi_chunk_pixel_burst_delivers_every_byte_to_ssd1306() {
    use crate::peripherals::components::Ssd1306;

    const ADDR7: u8 = 0x3C;
    const ADDR_W: u32 = (ADDR7 as u32) << 1; // 0x78, R/W = write

    let mut p = Esp32c3I2c::new();
    p.push_slave(Box::new(Ssd1306::new(ADDR7)));

    // ── Init: a short command transaction that fits in ONE FIFO load (the
    //    prologue that already works in the field). Horizontal addressing,
    //    full 128×64 window, display on. Control byte 0x00 = command stream.
    let init = [0x20u8, 0x00, 0x21, 0x00, 0x7F, 0x22, 0x00, 0x07, 0xAF];
    p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
    p.write_u32(REG_CMD0 + 4, cmd(CMD_WRITE, (2 + init.len()) as u8))
        .unwrap();
    p.write_u32(REG_CMD0 + 8, cmd(CMD_STOP, 0)).unwrap();
    p.write_u32(REG_DATA, ADDR_W).unwrap();
    p.write_u32(REG_DATA, 0x00).unwrap(); // command-stream control byte
    for b in init {
        p.write_u32(REG_DATA, b as u32).unwrap();
    }
    start_and_run(&mut p);
    assert_eq!(
        p.read_u32(REG_INT_RAW).unwrap() & INT_NACK,
        0,
        "init prologue must ACK"
    );

    // ── Pixel data: four full pages. Distinct nonzero pattern so a dropped
    //    byte (read back as 0x00) or a shifted byte is caught at its exact
    //    GDDRAM position.
    const N_PAGES: usize = 4;
    const DATA_LEN: usize = 128 * N_PAGES; // 512 bytes → N = 4
    let pattern: Vec<u8> = (0..DATA_LEN).map(|i| ((i % 251) + 1) as u8).collect();

    // Stream one page (128 bytes) per transaction, exactly how
    // Adafruit_SSD1306 pushes the framebuffer with the 0x40 data control
    // byte. Each WRITE command is addr(1) + control(1) + 128 data = 130
    // bytes — over 4× the 32-byte TX FIFO — so it underruns and is refilled
    // mid-command.
    for page in 0..N_PAGES {
        let page_data = &pattern[page * 128..(page + 1) * 128];
        let mut payload = Vec::with_capacity(2 + 128);
        payload.push(ADDR_W as u8);
        payload.push(0x40); // SSD1306 data-stream control byte
        payload.extend_from_slice(page_data);

        p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
        p.write_u32(REG_CMD0 + 4, cmd(CMD_WRITE, payload.len() as u8))
            .unwrap();
        p.write_u32(REG_CMD0 + 8, cmd(CMD_STOP, 0)).unwrap();

        // Preload the TX FIFO to capacity, then kick the transaction.
        let mut next = 0usize;
        while next < payload.len() && p.tx_fifo.len() < FIFO_CAPACITY {
            p.write_u32(REG_DATA, payload[next] as u32).unwrap();
            next += 1;
        }
        p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();

        // Clock the engine, refilling the TX FIFO only once it has actually
        // drained — modelling an ISR that services the watermark / empty
        // interrupt with real latency. A faithful controller holds SCL low
        // until the refill lands; a controller that pops 0x00 on underrun
        // has already clocked bogus bytes into the panel by then.
        let mut guard = 0u64;
        while p.engine_active() {
            if p.tx_fifo.is_empty() && next < payload.len() {
                while next < payload.len() && p.tx_fifo.len() < FIFO_CAPACITY {
                    p.write_u32(REG_DATA, payload[next] as u32).unwrap();
                    next += 1;
                }
            }
            p.tick_elapsed(512);
            guard += 1;
            assert!(guard < 1_000_000, "engine never completed page {page}");
        }
        assert_eq!(
            next,
            payload.len(),
            "every byte of page {page} must have been pulled from the FIFO, \
                 not fabricated as 0x00 on underrun"
        );
        assert_eq!(
            p.read_u32(REG_INT_RAW).unwrap() & INT_NACK,
            0,
            "page {page} data burst must ACK"
        );
    }

    // ── Read back GDDRAM: every pixel byte must equal what was written, with
    //    no spurious 0x00 from a FIFO underrun and no positional shift.
    let oled = p
        .attached_slaves()
        .iter()
        .find_map(|d| d.as_any().and_then(|a| a.downcast_ref::<Ssd1306>()))
        .expect("SSD1306 attached");
    let fb = oled.framebuffer();
    assert_eq!(
        &fb[..DATA_LEN],
        &pattern[..],
        "multi-chunk pixel burst must land byte-exact in GDDRAM (a 0x00 or a \
             shift here is the black-OLED underrun bug)"
    );
    assert_eq!(
        oled.ink_bytes(),
        DATA_LEN,
        "all {DATA_LEN} written pixel bytes are nonzero and must be lit"
    );
}
// ── TCA9548A driven through the ESP32-C3 bit-level engine ───────────────
//
// The C3 does not execute its command list synchronously: the transaction
// is clocked out bit by bit over simulated cycles, and the address frame is
// resolved at the ACK bit (`ack_bit_level`). That is a third, independent
// resolution site — plus the `SLAVE_ADDR` fallback beside it — and neither
// had ever been driven with a bus switch attached.
mod mux {
    use super::*;
    use crate::peripherals::components::mux_fixture::{
        bytes_written_to, mux_with_tags, tag_for, MUX_ADDR, SENSOR_ADDR,
    };
    use crate::peripherals::components::tca9548a::Tca9548a;

    fn controller() -> Esp32c3I2c {
        let mut p = Esp32c3I2c::new();
        p.push_slave(Box::new(mux_with_tags(4)));
        p
    }

    fn with_mux<R>(p: &Esp32c3I2c, f: impl FnOnce(&Tca9548a) -> R) -> R {
        let mux = p.attached_slaves()[0]
            .as_any()
            .and_then(|a| a.downcast_ref::<Tca9548a>())
            .expect("slave 0 is the switch");
        f(mux)
    }

    /// Program a command list + TX FIFO and clock the bit engine to a park.
    fn program(p: &mut Esp32c3I2c, list: &[(u8, u8)], tx: &[u8]) {
        p.write_u32(REG_INT_CLR, 0xFFFF_FFFF).unwrap();
        // Flush both FIFOs without disturbing the watermark fields.
        let conf = p.read_u32(REG_FIFO_CONF).unwrap();
        p.write_u32(REG_FIFO_CONF, conf | (1 << 12) | (1 << 13))
            .unwrap();
        for (i, (op, n)) in list.iter().enumerate() {
            p.write_u32(REG_CMD0 + 4 * i as u64, cmd(*op, *n)).unwrap();
        }
        for b in tx {
            p.write_u32(REG_DATA, *b as u32).unwrap();
        }
        start_and_run(p);
    }

    fn write_bytes(p: &mut Esp32c3I2c, addr: u8, payload: &[u8]) {
        let mut tx = vec![addr << 1];
        tx.extend_from_slice(payload);
        p.write_u32(REG_SLAVE_ADDR, addr as u32).unwrap();
        program(
            p,
            &[(CMD_RSTART, 0), (CMD_WRITE, tx.len() as u8), (CMD_STOP, 0)],
            &tx,
        );
    }

    fn read_byte(p: &mut Esp32c3I2c, addr: u8) -> u8 {
        p.write_u32(REG_SLAVE_ADDR, addr as u32).unwrap();
        program(
            p,
            &[
                (CMD_RSTART, 0),
                (CMD_WRITE, 1),
                (CMD_READ, 1),
                (CMD_STOP, 0),
            ],
            &[(addr << 1) | 1],
        );
        p.read_u32(REG_DATA).unwrap() as u8
    }

    fn nacked(p: &Esp32c3I2c) -> bool {
        p.read_u32(REG_INT_RAW).unwrap() & INT_NACK != 0
    }

    fn probe_acked(p: &mut Esp32c3I2c, addr: u8) -> bool {
        p.write_u32(REG_SLAVE_ADDR, addr as u32).unwrap();
        program(
            p,
            &[(CMD_RSTART, 0), (CMD_WRITE, 1), (CMD_STOP, 0)],
            &[addr << 1],
        );
        !nacked(p)
    }

    #[test]
    fn four_sensors_at_one_address_answer_independently() {
        let mut p = controller();
        for ch in 0..4u8 {
            write_bytes(&mut p, MUX_ADDR, &[1 << ch]);
            assert_eq!(
                read_byte(&mut p, SENSOR_ADDR),
                tag_for(ch),
                "channel {ch} must be answered by the sensor wired to it"
            );
        }
    }

    #[test]
    fn switching_channels_changes_which_sensor_answers() {
        let mut p = controller();
        for ch in [2u8, 0, 3, 1, 3, 0] {
            write_bytes(&mut p, MUX_ADDR, &[1 << ch]);
            assert_eq!(read_byte(&mut p, SENSOR_ADDR), tag_for(ch), "channel {ch}");
        }
    }

    #[test]
    fn control_register_reads_back_over_the_bus() {
        let mut p = controller();
        write_bytes(&mut p, MUX_ADDR, &[0b0000_1010]);
        assert!(
            probe_acked(&mut p, MUX_ADDR),
            "the switch ACKs its own address"
        );
        assert_eq!(read_byte(&mut p, MUX_ADDR), 0b0000_1010);
    }

    #[test]
    fn a_sensor_on_a_disabled_channel_does_not_answer() {
        let mut p = controller();
        assert!(
            !probe_acked(&mut p, SENSOR_ADDR),
            "with all channels disabled the sensor address must raise INT_NACK, \
                 exactly as an unpopulated bus does"
        );

        write_bytes(&mut p, MUX_ADDR, &[1 << 1]);
        assert!(probe_acked(&mut p, SENSOR_ADDR));
        assert_eq!(read_byte(&mut p, SENSOR_ADDR), tag_for(1));

        write_bytes(&mut p, MUX_ADDR, &[0x00]);
        assert!(
            !probe_acked(&mut p, SENSOR_ADDR),
            "re-isolating the switch takes the sensor off the bus again"
        );
    }

    #[test]
    fn the_slave_addr_register_path_also_routes_through_the_switch() {
        let mut p = controller();

        // A zero-payload WRITE has no address byte to clock, so the C3
        // engine skips it entirely; the SLAVE_ADDR fallback on this
        // controller is reached from the address frame itself, when the
        // wire address matches nothing. Park a DIFFERENT wire address and
        // let SLAVE_ADDR carry the real target.
        write_bytes(&mut p, MUX_ADDR, &[1 << 3]);
        p.write_u32(REG_SLAVE_ADDR, SENSOR_ADDR as u32).unwrap();
        program(
            &mut p,
            &[
                (CMD_RSTART, 0),
                (CMD_WRITE, 1),
                (CMD_READ, 1),
                (CMD_STOP, 0),
            ],
            &[0x00],
        );
        assert!(
            !nacked(&p),
            "SLAVE_ADDR holds 0x13 and channel 3 is enabled — the fallback \
                 must resolve through the switch"
        );
        assert_eq!(
            p.read_u32(REG_DATA).unwrap() as u8,
            tag_for(3),
            "the SLAVE_ADDR fallback must reach the sensor on the SELECTED channel"
        );

        // Isolate every channel: the same fallback must now find nothing.
        write_bytes(&mut p, MUX_ADDR, &[0x00]);
        p.write_u32(REG_SLAVE_ADDR, SENSOR_ADDR as u32).unwrap();
        program(
            &mut p,
            &[(CMD_RSTART, 0), (CMD_WRITE, 1), (CMD_STOP, 0)],
            &[0x00],
        );
        assert!(
            nacked(&p),
            "with every channel isolated the SLAVE_ADDR fallback must NACK"
        );
    }

    #[test]
    fn a_write_reaches_only_the_selected_channel() {
        let mut p = controller();
        write_bytes(&mut p, MUX_ADDR, &[1 << 2]);
        write_bytes(&mut p, SENSOR_ADDR, &[0x5A]);

        with_mux(&p, |mux| {
            assert_eq!(bytes_written_to(mux, 2), vec![0x5A]);
            for ch in [0u8, 1, 3] {
                assert!(
                    bytes_written_to(mux, ch).is_empty(),
                    "channel {ch} is isolated and must receive nothing"
                );
            }
        });
    }
}
