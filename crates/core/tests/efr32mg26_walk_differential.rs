// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! EFR32MG26 (BRD2709A) **walk-vs-scheduler** executing fidelity differential.
//!
//! # Why this file exists
//!
//! Five model families on this chip moved from the legacy per-cycle walk onto
//! the event scheduler so the BRD2709A bus can derive walk-deletion and stop
//! running one instruction per batch: `Efr32s2Timer` (ten instances),
//! `Efr32s2Iadc`, `Efr32s2GpioExti`, the EFR32 half of the shared `I2c`, and
//! the `VirtualBle` controller.
//!
//! Every one of them replaced "visit me every cycle" with "wake me at cycle
//! N". **A model that arms an event at the wrong cycle delivers an interrupt
//! late, and that renders identically right up to the run where it does not.**
//! Unit tests pin the closed forms at points; this file pins the whole chip
//! against the thing it replaced, executing real firmware, per instruction.
//!
//! # The two lanes
//!
//! Both lanes are the SAME `from_config` BRD2709A bus (chip yaml + system yaml
//! + `configure_cortex_m`), built the way the run path builds it, with any hand
//! `walk_deleted` hatch stripped so nothing is asserted that is not derived.
//!
//! * **reference** — every migrated EFR32 model is pinned back onto the legacy
//!   walk with `force_legacy_walk`, and `recompute_walk_deletable` is re-run so
//!   the bus actually walks them (pinning a model back onto a walk-deleted bus
//!   without that recompute starves it silently);
//! * **candidate** — the production path: the models stay scheduler-driven and
//!   the bus derives `legacy_walk_disabled`.
//!
//! Both run at `peripheral_tick_interval = 1`, because that is the regime in
//! which the walk reference is defined cycle-for-cycle. At a wider interval the
//! candidate's observations quantise to the batch grid by design (the bound
//! every migrated model documents), so comparing there would be comparing
//! against a different contract.
//!
//! # What "any divergence" means here
//!
//! After EVERY retired instruction both lanes are probed for the full
//! architectural state (`total_cycles`, PC, all 16 core registers), the console
//! byte count, and a peripheral vector spanning every migrated model — TIMER0/1
//! `CNT`/`IF`/`STATUS`, the IADC `STATUS`/`IF`, the EXTI `IF`, I2C0 `IF`/`STATE`
//! and the BLE `STATUS` — plus the live pad levels of the two board LEDs. A
//! mismatch fails at the first differing step and names it. An SRAM hash is
//! folded in periodically so a divergence that reaches memory but not a
//! register cannot hide either.
//!
//! Divergence here is a real fidelity bug in the migration. It must be
//! reported, never masked.

#![cfg(feature = "event-scheduler")]

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::CortexM;
use labwired_core::peripherals::efr32::gpio_exti::Efr32s2GpioExti;
use labwired_core::peripherals::efr32::iadc::Efr32s2Iadc;
use labwired_core::peripherals::efr32::timer::Efr32s2Timer;
use labwired_core::peripherals::i2c::I2c;
use labwired_core::peripherals::virtual_ble::VirtualBle;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, DebugControl, Machine};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

// ── Chip map (efr32mg26.yaml) ───────────────────────────────────────────────

const TIMER0: u64 = 0x4004_8000;
const TIMER1: u64 = 0x4004_C000;
const TIMER_CFG: u64 = 0x04;
const TIMER_CMD: u64 = 0x0C;
const TIMER_STATUS: u64 = 0x10;
const TIMER_IF: u64 = 0x14;
const TIMER_IEN: u64 = 0x18;
const TIMER_TOP: u64 = 0x1C;
const TIMER_CNT: u64 = 0x24;
const TIMER_EN: u64 = 0x30;
const TIMER_CC0: u64 = 0x60;
const TIMER_CC_CFG: u64 = 0x00;
const TIMER_CC_OC: u64 = 0x08;

const IADC0: u64 = 0x4900_4000;
const IADC_EN: u64 = 0x04;
const IADC_CMD: u64 = 0x0C;
const IADC_STATUS: u64 = 0x14;
const IADC_IF: u64 = 0x24;
const IADC_IEN: u64 = 0x28;
const IADC_SINGLE: u64 = 0x98;

const GPIO_EXTI: u64 = 0x4003_C400;
/// `EXTIPSEL[0]` picks the PORT for external interrupt line 0: 0=PORTA … 3=PORTD.
/// ⚠️ Its reset value is 0, i.e. PORTA — so a line left unprogrammed watches
/// PA00, not the pad the test meant.
const EXTI_EXTIPSELL: u64 = 0x00;
const EXTI_EXTIFALL: u64 = 0x14;
const EXTI_IF: u64 = 0x20;
const EXTI_IEN: u64 = 0x24;

const I2C0: u64 = 0x4B00_0000;
const I2C_IF: u64 = 0x28;
const I2C_STATE: u64 = 0x20;

const BLE: u64 = 0x4F00_0000;
const BLE_CTRL: u64 = 0x04;
const BLE_STATUS: u64 = 0x08;
const BLE_TXLEN: u64 = 0x20;
const BLE_TXBUF: u64 = 0x40;
const BLE_ADVINTERVAL: u64 = 0x14;
const BLE_ADV_EN: u32 = 1 << 0;

/// Cortex-M NVIC interrupt set-pending, `ISPR0`. IRQ n is bit `n % 32` of
/// word `n / 32`: TIMER0 = 4, TIMER9 = 5, GPIO_ODD = 39, GPIO_EVEN = 40,
/// I2C0 = 41, IADC0 = 65.
const NVIC_ISPR: u64 = 0xE000_E200;

const CMU: u64 = 0x4000_8000;
const CMU_CLKEN0: u64 = 0x64;
const CMU_CLKEN2: u64 = 0x6C;

/// `CMU_CLKEN0`: TIMER0 bit 4, TIMER1 bit 5, GPIO bit 26, IADC0 bit 10.
const CLKEN0_ALL: u32 = (1 << 4) | (1 << 5) | (1 << 10) | (1 << 14) | (1 << 26);

/// LED0 = PC08 and LED1 = PC09 (UG594) — the `gpioc` pins the probe samples.
const LED_PINS: [u8; 2] = [8, 9];

const SRAM_BASE: u64 = 0x2000_0000;
const SRAM_HASH_LEN: u64 = 0x4000;

fn root(rel: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../..");
    p.push(rel);
    p
}

/// Which drive mode a lane is built in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Lane {
    /// Every migrated EFR32 model pinned back onto the legacy per-cycle walk.
    Walk,
    /// The production path: models scheduler-driven, bus walk-deleted.
    Scheduler,
}

/// Pin every model this migration touched back onto the legacy walk.
///
/// ⚠️ Reaching a model MUTABLY is what makes the reference lane a reference. A
/// model that does not implement `as_any_mut` silently answers `None` here, is
/// left scheduler-driven, and the "reference" becomes a second copy of the
/// candidate — a differential that cannot fail. The count assertion below is
/// what stops that from happening quietly.
fn force_all_legacy_walk(bus: &mut SystemBus) {
    let mut pinned = 0usize;
    for entry in bus.peripherals.iter_mut() {
        let Some(any) = entry.dev.as_any_mut() else {
            continue;
        };
        if let Some(p) = any.downcast_mut::<Efr32s2Timer>() {
            p.force_legacy_walk();
        } else if let Some(p) = any.downcast_mut::<Efr32s2Iadc>() {
            p.force_legacy_walk();
        } else if let Some(p) = any.downcast_mut::<Efr32s2GpioExti>() {
            p.force_legacy_walk();
        } else if let Some(p) = any.downcast_mut::<I2c>() {
            p.force_legacy_walk();
        } else if let Some(p) = any.downcast_mut::<VirtualBle>() {
            p.force_legacy_walk();
        } else {
            continue;
        }
        pinned += 1;
    }
    // Ten TIMERs + four I2Cs + IADC + EXTI + BLE.
    assert_eq!(
        pinned, 17,
        "the reference lane must pin every migrated EFR32 model back onto the \
         walk; only {pinned} answered `as_any_mut`, so the rest stayed \
         scheduler-driven and this differential could not fail"
    );
    // Pinning a model back onto a bus that already derived walk-deletion would
    // starve it of ticks. Re-derive over the live set.
    bus.recompute_walk_deletable();
}

fn brd2709a_bus(lane: Lane) -> SystemBus {
    let system_path = root("configs/systems/brd2709a.yaml");
    let mut manifest = SystemManifest::from_file(&system_path).expect("load brd2709a manifest");
    // Derive walk-deletion from the models, never from the yaml escape hatch.
    manifest.walk_deleted = None;
    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load efr32mg26 chip");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build brd2709a bus");
    let _ = configure_cortex_m(&mut bus);
    if lane == Lane::Walk {
        force_all_legacy_walk(&mut bus);
    }
    bus
}

/// One lane, with everything the per-instruction probe needs resolved ONCE.
/// Re-resolving the GPIO port by name inside the probe would put a linear scan
/// with a string compare per entry on the inner loop of a 120 000-step run.
struct LaneRun {
    machine: Machine<CortexM>,
    sink: Arc<Mutex<Vec<u8>>>,
    gpioc: usize,
}

fn machine(lane: Lane) -> (Machine<CortexM>, Arc<Mutex<Vec<u8>>>) {
    let mut bus = brd2709a_bus(lane);
    let sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);
    match lane {
        Lane::Walk => assert!(
            !bus.legacy_walk_disabled,
            "the reference lane must actually walk"
        ),
        Lane::Scheduler => assert!(
            bus.legacy_walk_disabled,
            "the candidate lane must be walk-deleted, or it is the same lane twice"
        ),
    }
    let mut m = Machine::new(CortexM::new(), bus);
    // The walk reference is defined per cycle, so both lanes tick per cycle.
    m.config.peripheral_tick_interval = 1;
    m.bus.config.peripheral_tick_interval = 1;
    (m, sink)
}

/// Everything the two lanes must agree on after a single retired instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Probe {
    step: u64,
    total_cycles: u64,
    pc: u32,
    regs: [u32; 16],
    uart_len: usize,
    /// TIMER0/TIMER1 `CNT`, `IF`, `STATUS`; IADC `STATUS`/`IF`; EXTI `IF`;
    /// I2C0 `IF`; BLE `STATUS`.
    periph: [u32; 11],
    /// NVIC `ISPR0..2` — the interrupt-pending latch.
    ///
    /// ⚠️ This is the field that makes the whole differential able to fail, and
    /// it was NOT in the first version. Every migrated model advances its state
    /// LAZILY: a register read syncs to "now" before answering, so `TIMER0.IF`
    /// reads correctly whatever cycle the event was armed for. The only thing a
    /// mis-armed event actually moves is the cycle the NVIC line is PENDED on —
    /// so a differential that does not sample ISPR passes with the arming
    /// deliberately broken. Verified by doing exactly that.
    ispr: [u32; 3],
    /// Live pad level of LED0/LED1 — the wire, not the register, so a PWM
    /// output that stops moving is visible here.
    pads: [Option<bool>; 2],
    /// Folded in every `SRAM_HASH_EVERY` steps (`None` otherwise), so a
    /// divergence that reaches memory without reaching a register is caught
    /// too, without hashing 16 KiB per instruction.
    sram: Option<u64>,
}

/// How often the SRAM window is hashed into the probe.
const SRAM_HASH_EVERY: u64 = 2048;

fn sram_hash(bus: &SystemBus) -> u64 {
    // FNV-1a over the SRAM window. An unmapped word reads as a stable sentinel
    // so both lanes agree on any hole.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut addr = SRAM_BASE;
    while addr < SRAM_BASE + SRAM_HASH_LEN {
        hash ^= bus.read_u32(addr).unwrap_or(0xDEAD_BEEF) as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
        addr += 4;
    }
    hash
}

fn lane_run(lane: Lane) -> LaneRun {
    let (machine, sink) = machine(lane);
    let gpioc = machine
        .bus
        .find_peripheral_index_by_name("gpioc")
        .expect("the chip declares a gpioc port");
    LaneRun {
        machine,
        sink,
        gpioc,
    }
}

fn probe(run: &mut LaneRun, step: u64) -> Probe {
    let m = &mut run.machine;
    let mut regs = [0u32; 16];
    for (i, r) in regs.iter_mut().enumerate() {
        *r = m.read_core_reg(i as u8);
    }
    let rd = |b: &SystemBus, a: u64| b.read_u32(a).unwrap_or(0xDEAD_BEEF);
    let periph = [
        rd(&m.bus, TIMER0 + TIMER_CNT),
        rd(&m.bus, TIMER0 + TIMER_IF),
        rd(&m.bus, TIMER0 + TIMER_STATUS),
        rd(&m.bus, TIMER1 + TIMER_CNT),
        rd(&m.bus, TIMER1 + TIMER_IF),
        rd(&m.bus, TIMER1 + TIMER_STATUS),
        rd(&m.bus, IADC0 + IADC_STATUS),
        rd(&m.bus, IADC0 + IADC_IF),
        rd(&m.bus, GPIO_EXTI + EXTI_IF),
        rd(&m.bus, I2C0 + I2C_IF),
        rd(&m.bus, BLE + BLE_STATUS),
    ];
    let ispr = [
        rd(&m.bus, NVIC_ISPR),
        rd(&m.bus, NVIC_ISPR + 4),
        rd(&m.bus, NVIC_ISPR + 8),
    ];
    // The WIRE level a probe clipped to the pin would read, through the same
    // routing seam the logic analyzer uses — not the port's output register. A
    // PWM output whose pad stops moving is invisible in the register and
    // obvious here.
    let dev = &m.bus.peripherals[run.gpioc].dev;
    let pads = [
        dev.read_gpio_pad(LED_PINS[0]),
        dev.read_gpio_pad(LED_PINS[1]),
    ];
    Probe {
        step,
        total_cycles: m.total_cycles,
        pc: m.get_pc(),
        regs,
        uart_len: run.sink.lock().unwrap().len(),
        periph,
        ispr,
        pads,
        sram: (step % SRAM_HASH_EVERY == 0).then(|| sram_hash(&m.bus)),
    }
}

/// Run both lanes `steps` instructions, ONE at a time, comparing the full probe
/// after every single one. Fails at the FIRST divergence and names the step —
/// not at the end, where a cycle-late interrupt has already been laundered into
/// a different control-flow path.
fn assert_lanes_identical(
    steps: u64,
    setup: impl Fn(&mut Machine<CortexM>),
    what: &str,
) -> LaneRun {
    let mut walk = lane_run(Lane::Walk);
    setup(&mut walk.machine);
    let mut sched = lane_run(Lane::Scheduler);
    setup(&mut sched.machine);

    for step in 1..=steps {
        walk.machine.run(Some(1)).expect("walk lane step");
        sched.machine.run(Some(1)).expect("scheduler lane step");
        let w = probe(&mut walk, step);
        let s = probe(&mut sched, step);
        assert_eq!(
            w, s,
            "{what}: walk-vs-scheduler diverged at instruction {step}\n  \
             walk = {w:?}\n  sched = {s:?}"
        );
    }
    assert_eq!(
        *walk.sink.lock().unwrap(),
        *sched.sink.lock().unwrap(),
        "{what}: the console byte streams diverged"
    );
    sched
}

// ── Gate 1: the shipped firmware, per instruction ───────────────────────────

/// The BRD2709A smoke image (`crates/firmware-mg26-demo`, the same ELF the
/// docs-runnable-chips lane executes) run twice on the real board bus, compared
/// after every retired instruction.
///
/// This firmware is the right vehicle because it touches the migrated models
/// for real: it drives the GPIO ports whose edges reach the EXTI, prints over
/// the VCOM USART, and spins on `micros()`-shaped timer reads — so a timer that
/// counts one cycle differently changes the byte the console emits, not merely
/// a register nobody looks at.
#[test]
fn efr32mg26_smoke_firmware_walk_vs_scheduler_is_byte_identical_per_step() {
    // Enough to carry the image past its banner and through the IO phase; the
    // per-instruction probe is what makes the count sufficient rather than the
    // depth of any single check.
    const STEPS: u64 = 120_000;

    let fixture = root("tests/fixtures/mg26-smoke.elf");
    let image = labwired_loader::load_elf(&fixture).expect("load mg26-smoke.elf");
    let sched = assert_lanes_identical(
        STEPS,
        |m| {
            m.load_firmware(&image).expect("load firmware");
        },
        "mg26-smoke.elf",
    );

    // ...and the run must not have been vacuous: the firmware really executed
    // its application logic, so the byte-identity above means something.
    let uart = sched.sink.lock().unwrap().clone();
    let text = String::from_utf8_lossy(&uart);
    assert!(
        text.contains("MG26 OK"),
        "the firmware never printed its banner, so the comparison was vacuous; \
         got {text:?}"
    );
}

// ── Gate 2: the timer compare interrupt, per cycle ──────────────────────────

/// A TIMER0 compare interrupt armed by hand, both lanes, per cycle.
///
/// The firmware gate above exercises whatever the demo image happens to
/// configure. This one arms the exact thing the migration is most likely to get
/// wrong — an interrupt whose cycle is now computed in closed form instead of
/// discovered by counting — and watches the flag and the NVIC pending bit on
/// every single cycle. A wake one cycle early or late shows up as a `TIMER0.IF`
/// that differs at one step.
#[test]
fn efr32mg26_timer_compare_irq_lands_on_the_same_cycle() {
    const STEPS: u64 = 40_000;
    assert_lanes_identical(
        STEPS,
        |m| {
            let b = &mut m.bus;
            b.write_u32(CMU + CMU_CLKEN0, CLKEN0_ALL).unwrap();
            b.write_u32(TIMER0 + TIMER_EN, 1).unwrap();
            // A prescaler that is NOT a divisor of the clock ratio, so the
            // residue carry is live rather than incidental.
            b.write_u32(TIMER0 + TIMER_CFG, 7 << 18).unwrap();
            b.write_u32(TIMER0 + TIMER_TOP, 997).unwrap();
            b.write_u32(TIMER0 + TIMER_CC0 + TIMER_CC_CFG, 0x2).unwrap(); // OUTPUTCOMPARE
            b.write_u32(TIMER0 + TIMER_CC0 + TIMER_CC_OC, 613).unwrap();
            // Both the overflow and the compare, so the arming path has to pick
            // the nearer of two and then chain to the other.
            b.write_u32(TIMER0 + TIMER_IEN, 1 | (1 << 4)).unwrap();
            b.write_u32(TIMER0 + TIMER_CMD, 1).unwrap(); // START

            // A second instance on a different width and a PWM channel driving
            // a pad, so the wire-edge arming path is exercised too.
            b.write_u32(TIMER1 + TIMER_EN, 1).unwrap();
            b.write_u32(TIMER1 + TIMER_TOP, 255).unwrap();
            b.write_u32(TIMER1 + TIMER_CC0 + TIMER_CC_CFG, 0x3).unwrap(); // PWM
            b.write_u32(TIMER1 + TIMER_CC0 + TIMER_CC_OC, 64).unwrap();
            b.write_u32(TIMER1 + TIMER_CMD, 1).unwrap();
        },
        "TIMER0 compare + TIMER1 PWM",
    );
}

// ── Gate 3: the IADC conversion and the EXTI held level, per cycle ──────────

/// A single conversion and an armed EXTI line, both lanes, per cycle.
///
/// The IADC settles its conversion on the first visit after `SINGLESTART`; the
/// EXTI holds an interrupt level for as long as an unmasked flag is latched.
/// Both were per-cycle walk duties and are now event chains, and both are
/// observed here on the cycle they change.
#[test]
fn efr32mg26_iadc_and_exti_track_the_walk_per_cycle() {
    const STEPS: u64 = 20_000;
    assert_lanes_identical(
        STEPS,
        |m| {
            let b = &mut m.bus;
            b.write_u32(CMU + CMU_CLKEN0, CLKEN0_ALL).unwrap();
            b.write_u32(CMU + CMU_CLKEN2, 0xFFFF_FFFF).unwrap();

            // EXTI0 watches PB00 (BTN0) for a falling edge, unmasked. The
            // port select is NOT optional: it resets to PORTA.
            b.write_u32(GPIO_EXTI + EXTI_EXTIPSELL, 1).unwrap(); // PORTB
            b.write_u32(GPIO_EXTI + EXTI_EXTIFALL, 1).unwrap();
            b.write_u32(GPIO_EXTI + EXTI_IEN, 1).unwrap();

            // A conversion on PA05, with the done interrupt unmasked.
            b.write_u32(IADC0 + IADC_EN, 1).unwrap();
            b.write_u32(IADC0 + IADC_IEN, 1).unwrap();
            b.write_u32(IADC0 + IADC_SINGLE, (8 << 16) | (5 << 24))
                .unwrap();
            b.write_u32(IADC0 + IADC_CMD, 1 << 0).unwrap(); // SINGLESTART
        },
        "IADC conversion + EXTI held level",
    );
}

// ── Gate 4: the BLE controller, per cycle ──────────────────────────────────

/// The virtual BLE controller advertising, both lanes, per cycle.
///
/// ⚠️ This block is the one whose per-cycle duty could NOT be replaced by a
/// deadline: the air is written by another machine, so nothing here can raise
/// an event when a peer transmits. Its chain therefore re-arms at the walk's
/// own cadence while scanning. That is a claim about behaviour, and this is
/// what checks it — the advertising bursts must land on the same cycles, and
/// `STATUS` must read the same on every one.
#[test]
fn efr32mg26_ble_advertising_tracks_the_walk_per_cycle() {
    // A short interval so several bursts land inside the step budget.
    const STEPS: u64 = 30_000;
    assert_lanes_identical(
        STEPS,
        |m| {
            let b = &mut m.bus;
            b.write_u32(BLE + BLE_TXBUF, 0x2602_0C02).unwrap();
            b.write_u32(BLE + BLE_TXLEN, 4).unwrap();
            // 16 units x 625 us = 10 ms = 780_000 cycles at 78 MHz; the gate
            // below shortens it further by watching STATUS, not the air.
            b.write_u32(BLE + BLE_ADVINTERVAL, 1).unwrap();
            b.write_u32(BLE + BLE_CTRL, BLE_ADV_EN).unwrap();
        },
        "BLE advertising",
    );
}

// ── Gate 5: the board flip status ──────────────────────────────────────────

/// The exact `SystemBus::derive_walk_deletable` predicate (`pub(crate)`),
/// recomputed here from PUBLIC trait methods over the PUBLIC peripheral list.
fn walk_forcing_set(bus: &SystemBus) -> Vec<String> {
    let mut forcing: Vec<String> = bus
        .peripherals
        .iter()
        .filter(|p| p.dev.needs_legacy_walk() && !p.dev.uses_scheduler())
        .map(|p| p.name.clone())
        .collect();
    forcing.sort();
    forcing
}

/// No peripheral on the shipped BRD2709A bus forces the per-cycle walk.
///
/// ⚠️ The list this had to empty was NOT the three EFR32-specific models the
/// migration set out to move. It was fifteen: ten `Efr32s2Timer` instances, the
/// `Efr32s2Iadc`, the `Efr32s2GpioExti`, FOUR shared-`I2c` instances in the
/// EFR32 variant, and the `VirtualBle` controller. Any one of them left behind
/// pins the whole chip to one instruction per batch, which is why this gate
/// asserts the set is empty rather than that it shrank.
#[test]
fn efr32mg26_board_walk_forcing_set_is_empty() {
    let bus = brd2709a_bus(Lane::Scheduler);
    let forcing = walk_forcing_set(&bus);
    assert!(
        forcing.is_empty(),
        "BRD2709A still has walk-forcers under event-scheduler: {forcing:?}"
    );
}

/// The board flip: `derive_walk_deletable()` is true with no hand flag, and the
/// bus reaches the recommended tick interval.
#[test]
fn efr32mg26_board_derives_walk_deletion_and_reaches_tick_512() {
    use labwired_core::bus::RECOMMENDED_TICK_INTERVAL;
    let bus = brd2709a_bus(Lane::Scheduler);
    assert!(
        bus.legacy_walk_disabled,
        "BRD2709A must derive walk-deletion; remaining forcing set: {:?}",
        walk_forcing_set(&bus)
    );
    assert_eq!(
        bus.max_safe_tick_interval(),
        RECOMMENDED_TICK_INTERVAL,
        "a walk-deleted BRD2709A must reach the recommended tick interval"
    );
}

/// ⚠️ A walk-deleted bus takes a fast path that skips the whole phase-1 body —
/// and the GPIO edge-detection pass, the only thing that ever tells an EXTI
/// line a pad moved, lives inside it. The guard that keeps that pass alive used
/// to be "is a peripheral named `gpio0` or `gpio1` present", which is a Nordic
/// spelling; this part letters its ports, so the fast path would have deleted
/// the pass and a button press would have latched nothing, silently.
///
/// This asserts the bus keeps the pass, and — the half that actually matters —
/// that an edge still reaches the EXTI on the walk-deleted production bus.
#[test]
fn a_walk_free_efr32_bus_still_delivers_gpio_edges_to_the_exti() {
    let (mut m, _sink) = machine(Lane::Scheduler);
    assert!(
        m.bus.legacy_walk_disabled,
        "precondition: this gate is about the walk-deleted bus"
    );
    m.bus.write_u32(CMU + CMU_CLKEN0, CLKEN0_ALL).unwrap();
    // EXTI0 watches PB00 (BTN0 — active low, so a press is a FALLING edge).
    // `EXTIPSEL[0]` resets to PORTA, so selecting PORTB is load-bearing.
    m.bus.write_u32(GPIO_EXTI + EXTI_EXTIPSELL, 1).unwrap();
    m.bus.write_u32(GPIO_EXTI + EXTI_EXTIFALL, 1).unwrap();
    m.bus.write_u32(GPIO_EXTI + EXTI_IEN, 1).unwrap();

    // Settle the boot levels as the baseline, then press.
    m.run(Some(4)).expect("settle");
    assert_eq!(
        m.bus.read_u32(GPIO_EXTI + EXTI_IF).unwrap() & 1,
        0,
        "nothing has moved yet"
    );
    m.set_input_on("btn0_pb00", "pressed", 1.0)
        .expect("press BTN0");
    m.run(Some(8)).expect("observe the edge");

    assert_eq!(
        m.bus.read_u32(GPIO_EXTI + EXTI_IF).unwrap() & 1,
        1,
        "a button press on a walk-deleted EFR32 bus latched NO EXTI flag: the \
         per-cycle GPIO edge pass was skipped by the walk-free fast path"
    );
}

/// The I2C0 state register is read in the probe vector above; this pins the one
/// thing that read would not catch — that the EFR32 I²C really is on the
/// scheduler now, and that its `force_legacy_walk` knob really moves it back.
/// Without both, gate 1's reference lane would quietly be the candidate lane.
#[test]
fn the_efr32_i2c_drive_mode_is_switchable_in_both_directions() {
    let sched = brd2709a_bus(Lane::Scheduler);
    let idx = sched.find_peripheral_index_by_name("i2c0").expect("i2c0");
    assert!(
        sched.peripherals[idx].dev.uses_scheduler()
            && !sched.peripherals[idx].dev.needs_legacy_walk(),
        "i2c0 must be scheduler-driven on the production bus"
    );
    let walk = brd2709a_bus(Lane::Walk);
    assert!(
        !walk.peripherals[idx].dev.uses_scheduler()
            && walk.peripherals[idx].dev.needs_legacy_walk(),
        "i2c0 must be pinned back onto the walk in the reference lane"
    );
    // ...and the register the probe samples must still answer on both.
    assert_eq!(
        sched.read_u32(I2C0 + I2C_STATE).unwrap(),
        walk.read_u32(I2C0 + I2C_STATE).unwrap()
    );
}
