// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Does `analogWrite` reach a PIN on BRD2709A?
//!
//! For the whole of this board's onboarding the answer was no, and the reason
//! was never the timer: the duty went into real `CC_OC` registers, correctly,
//! and on Series 2 an output reaches a pad only through `GPIO_TIMERROUTE`,
//! which nothing modelled. Every write-up of that gap said "unmodelled".
//!
//! ⚠️ This file exists because the unit tests were not enough. `Efr32s2Timer`
//! publishes its CC levels, `Efr32s2TimerRoute` claims pads, the GPIO port
//! reads claims — all three had passing unit tests while the wiring pass that
//! joins them silently did nothing, because the route model implemented
//! `as_any` where the pass downcasts through `as_any_mut` (which defaults to
//! `None`). Everything compiled. Nothing was routed. Only an end-to-end read of
//! the PAD can tell the difference.
//!
//! So this drives the SHIPPED `from_config` bus through MMIO, the way firmware
//! does, and reads the pad through `read_gpio_pad`.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::CortexM;
use labwired_core::logic_capture::LogicSource;
use labwired_core::{Bus, Machine};
use std::path::PathBuf;

/// TIMER1 — the instance `analogWrite` uses (`wiring.cpp`'s `pwm_start`).
const TIMER1: u64 = 0x4004_C000;
const TIMER_CFG: u64 = 0x04;
const TIMER_CMD: u64 = 0x0C;
const TIMER_TOP: u64 = 0x1C;
const TIMER_EN: u64 = 0x30;
/// `CC[3]` at +0x60, stride 0x20: CFG at +0x00, OC at +0x08.
const TIMER_CC0_CFG: u64 = 0x60;
const TIMER_CC0_OC: u64 = 0x68;

const CMU_CLKEN0: u64 = 0x4000_8064;
const CLKEN0_GPIO: u32 = 1 << 26;
const CLKEN0_TIMER1: u32 = 1 << 5;

/// `GPIO_TIMERROUTE[1]` — block +0x6E0, stride 0x20.
const TIMERROUTE1: u64 = 0x4003_C6E0 + 0x20;
const ROUTEEN: u64 = 0x00;
const CC0ROUTE: u64 = 0x04;

/// Port C, pin 8 — LED0 on BRD2709A (`variants/xg26explorerkit/pins_arduino.h`).
const PORT_C: u32 = 2;
const PIN_8: u32 = 8;
/// GPIO port C's own window: block +0x30 + 2 * 0x30.
const GPIOC: u64 = 0x4003_C030 + 0x60;
const PORT_MODEH: u64 = 0x0C;

const CC_MODE_PWM: u32 = 0x3;
const PWM_TOP: u32 = 255;

fn root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// The SHIPPED bus — `from_config` on the committed yaml, with no wiring call
/// of this file's own. A gate that wired its own pads would prove the mechanism
/// works while the shipped chip stayed dark.
fn machine() -> Machine<CortexM> {
    let abs = root("configs/chips/efr32mg26.yaml");
    let chip = ChipDescriptor::from_file(&abs).expect("load the efr32mg26 descriptor");
    let manifest = SystemManifest {
        parts: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "efr32mg26-pwm-pad".to_string(),
        chip: abs.to_string_lossy().to_string(),
        cpu_hz: None,
        external_devices: vec![],
        cosim_models: Vec::new(),
        motor_models: Vec::new(),
        board_io: vec![],
        debug_uart: None,
        wifi_ap: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
    };
    let mut bus = SystemBus::from_config(&chip, &manifest)
        .expect("build the bus from the committed chip config");
    let (cpu, _nvic) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    // `b .` (0xE7FE): a Thumb branch to itself, so the CPU retires instructions
    // and time advances. Every register below is driven through MMIO, exactly
    // as a driver would; no firmware is involved.
    machine.bus.write_u8(0x2000_1000, 0xFE).unwrap();
    machine.bus.write_u8(0x2000_1001, 0xE7).unwrap();
    machine.cpu.pc = 0x2000_1000;
    machine
}

/// The pad level, read the way the analyzer reads it — through the routing
/// seam, not by peeking at a register.
fn pad(machine: &mut Machine<CortexM>, pin: u8) -> Option<bool> {
    let idx = machine
        .bus
        .find_peripheral_index_by_name("gpioc")
        .expect("the chip declares a gpioc port");
    machine.logic_watch(&[Some(LogicSource::pad(idx, pin))])[0]
}

fn run(machine: &mut Machine<CortexM>, steps: u64) {
    for _ in 0..steps {
        machine.step().expect("step");
    }
}

/// Exactly what `pwm_start()` + `analogWrite()` do, in that order.
fn program_pwm(bus: &mut dyn Bus, duty: u32) {
    bus.write_u32(CMU_CLKEN0, CLKEN0_GPIO | CLKEN0_TIMER1)
        .unwrap();
    // ⚠️ CC_CFG before EN: it is a disabled-only register on this silicon.
    bus.write_u32(TIMER1 + TIMER_CC0_CFG, CC_MODE_PWM).unwrap();
    bus.write_u32(TIMER1 + TIMER_CFG, 0).unwrap();
    bus.write_u32(TIMER1 + TIMER_EN, 1).unwrap();
    bus.write_u32(TIMER1 + TIMER_TOP, PWM_TOP).unwrap();
    bus.write_u32(TIMER1 + TIMER_CMD, 1).unwrap(); // START
    bus.write_u32(TIMER1 + TIMER_CC0_OC, duty).unwrap();
}

/// Route TIMER1 CC0 to PC08, the way a driver programming the pin-mux would.
fn route_cc0_to_pc08(bus: &mut dyn Bus) {
    bus.write_u32(TIMERROUTE1 + CC0ROUTE, PORT_C | (PIN_8 << 16))
        .unwrap();
    bus.write_u32(TIMERROUTE1 + ROUTEEN, 1).unwrap();
}

#[test]
fn a_routed_pwm_channel_drives_its_pad() {
    let mut m = machine();
    program_pwm(&mut m.bus, 200);
    route_cc0_to_pc08(&mut m.bus);

    // CNT is 0 and OC is 200, so the output is HIGH — `CNT < OC`.
    assert_eq!(
        pad(&mut m, 8),
        Some(true),
        "PC08 must follow TIMER1 CC0 once GPIO_TIMERROUTE points it there",
    );
}

/// The whole point, in one assertion: the pad FOLLOWS the waveform. A pad stuck
/// at one level would pass the test above and still be wrong.
#[test]
fn the_pad_follows_the_duty_across_a_whole_period() {
    let mut m = machine();
    program_pwm(&mut m.bus, 64); // 25% of TOP=255
    route_cc0_to_pc08(&mut m.bus);

    let mut high = 0usize;
    let mut seen_low = false;
    let mut seen_high = false;
    for _ in 0..=PWM_TOP {
        match pad(&mut m, 8) {
            Some(true) => {
                high += 1;
                seen_high = true;
            }
            Some(false) => seen_low = true,
            None => panic!("PC08 stopped answering mid-period"),
        }
        // One timer clock: the descriptor declares cpu_hz 78 MHz against a
        // 19 MHz peripheral band, so four core cycles.
        run(&mut m, 4);
    }
    assert!(seen_high && seen_low, "a 25% duty must show BOTH levels");
    // 64 of 256 counts high. The sampling is coarse (one read per timer clock,
    // and the tick ratio is integer), so this asserts the SHAPE — a quarter,
    // not a half and not a sliver — rather than an exact count.
    assert!(
        (40..=90).contains(&high),
        "a 25% duty should read high for roughly a quarter of the period; got {high}/256",
    );
}

/// ⚠️ The negative control, and the one that would have caught the silent
/// wiring miss: WITHOUT the route, the pad is an ordinary GPIO and must NOT
/// carry the waveform. A model that routed everything unconditionally would
/// pass every test above.
#[test]
fn an_unrouted_pad_does_not_carry_the_waveform() {
    let mut m = machine();
    program_pwm(&mut m.bus, 200);
    // No TIMERROUTE write at all. PC08 is an input in DISABLED mode.
    assert_eq!(
        pad(&mut m, 8),
        Some(false),
        "an unrouted pad reads its own port state, not the timer's output",
    );
}

/// And clearing `ROUTEEN` hands the pad back — firmware turning a PWM channel
/// off expects an ordinary pin again, not a frozen waveform.
#[test]
fn clearing_the_route_hands_the_pad_back_to_the_port() {
    let mut m = machine();
    program_pwm(&mut m.bus, 200);
    route_cc0_to_pc08(&mut m.bus);
    assert_eq!(pad(&mut m, 8), Some(true));

    m.bus.write_u32(TIMERROUTE1 + ROUTEEN, 0).unwrap();
    // Drive the pin low as a plain push-pull output and read it back: if the
    // route were still live the timer would win.
    m.bus.write_u32(GPIOC + PORT_MODEH, 0x4).unwrap(); // pin 8 PUSHPULL
    assert_eq!(
        pad(&mut m, 8),
        Some(false),
        "with ROUTEEN clear the port owns the pad again",
    );
}
