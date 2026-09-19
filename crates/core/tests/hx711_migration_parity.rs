// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! HX711: the declarative descriptor against the hand-written model it replaces,
//! **on a real bus**, clocked the way firmware clocks it.
//!
//! `components/hx711.rs` is DELETED — and so are the two pieces of engine it
//! needed: `SystemBus::hx711`, its private `Vec`, and `maybe_clock_hx711`, its
//! private write hook. That hook is now
//! [`BusResidentDevice::edge_service_addrs`], which any descriptor can use.
//!
//! ## Why this test drives MMIO and not the device
//!
//! The claim under test is not "the state machine shifts bits" — a unit test
//! over the rules would show that, and would pass with the part completely
//! unwired. The claim is that a `gpio_device` sees EVERY EDGE of a bit-bang
//! loop. A `digitalWrite(SCK, HIGH); digitalWrite(SCK, LOW)` pair is two stores
//! inside one peripheral tick; a service pass that only ran on the tick would
//! sample the pad after both and see no change at all, so a 24-bit frame would
//! never advance past its first bit.
//!
//! So every clock pulse below is a store to the GPIO output register and every
//! DOUT read is a load from the GPIO input register, with **no `advance`
//! between them**. [`the_frame_completes_with_no_tick_at_all`] is the negative
//! control: it counts the cycles the machine ran while the frame was clocked
//! and asserts the tick pass could not have delivered the edges.
//!
//! The chip is the NUCLEO-L476RG, the same hard case `tier2_device_pins.rs`
//! uses: its device clock is derived from `cpu_hz`.

use labwired_config::{ChipDescriptor, ExternalDevice, SystemManifest};
use labwired_core::snapshot::{ArmCpuSnapshot, CpuSnapshot};
use labwired_core::{
    AdvanceRequest, Bus, Cpu, Machine, SimResult, SimulationConfig, SimulationObserver,
};
use std::path::PathBuf;
use std::sync::Arc;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// A CPU that executes nothing: the point here is what MMIO stores do, not what
/// any instruction stream does.
#[derive(Debug, Default)]
struct IdleCpu {
    pc: u32,
    steps: u32,
}

impl Cpu for IdleCpu {
    fn reset(&mut self, _bus: &mut dyn Bus) -> SimResult<()> {
        self.pc = 0;
        self.steps = 0;
        Ok(())
    }
    fn step(
        &mut self,
        _bus: &mut dyn Bus,
        _observers: &[Arc<dyn SimulationObserver>],
        _config: &SimulationConfig,
    ) -> SimResult<()> {
        self.steps = self.steps.wrapping_add(1);
        self.pc = self.pc.wrapping_add(2);
        Ok(())
    }
    fn set_pc(&mut self, val: u32) {
        self.pc = val;
    }
    fn get_pc(&self) -> u32 {
        self.pc
    }
    fn set_sp(&mut self, _val: u32) {}
    fn set_exception_pending(&mut self, _exception_num: u32) {}
    fn get_register(&self, id: u8) -> u32 {
        match id {
            0 => self.steps,
            15 => self.pc,
            _ => 0,
        }
    }
    fn set_register(&mut self, id: u8, val: u32) {
        match id {
            0 => self.steps = val,
            15 => self.pc = val,
            _ => {}
        }
    }
    fn snapshot(&self) -> CpuSnapshot {
        let mut registers = vec![0; 16];
        registers[0] = self.steps;
        registers[15] = self.pc;
        CpuSnapshot::Arm(ArmCpuSnapshot {
            registers,
            pc: self.pc,
            xpsr: 0,
            primask: false,
            pending_exceptions: 0,
            pending_exceptions_hi: Vec::new(),
            vtor: 0,
        })
    }
    fn apply_snapshot(&mut self, snapshot: &CpuSnapshot) {
        if let CpuSnapshot::Arm(s) = snapshot {
            self.steps = s.registers.first().copied().unwrap_or(0);
            self.pc = s.pc;
        }
    }
    fn get_register_names(&self) -> Vec<String> {
        (0..=12)
            .map(|id| format!("R{id}"))
            .chain(["SP", "LR", "PC"].into_iter().map(String::from))
            .collect()
    }
    fn index_of_register(&self, name: &str) -> Option<u8> {
        if name.eq_ignore_ascii_case("PC") {
            return Some(15);
        }
        let id = name
            .strip_prefix('R')
            .or_else(|| name.strip_prefix('r'))?
            .parse::<u8>()
            .ok()?;
        (id <= 12).then_some(id)
    }
}

/// RCC AHB2ENR on the STM32L4: bit *n* ungates GPIO port *n* (A…H). Without
/// this the port's whole register file is dead and the test would measure the
/// clock gate rather than the part.
const RCC_AHB2ENR: u64 = 0x4002_104C;
const SCK_PIN: &str = "PA8";
const DT_PIN: &str = "PA9";

fn machine() -> Machine<IdleCpu> {
    let system_path = repo_root().join("configs/systems/nucleo-l476rg.yaml");
    let mut manifest = SystemManifest::from_file(&system_path).expect("load nucleo-l476rg.yaml");
    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    manifest.external_devices.push(ExternalDevice {
        id: "scale".to_string(),
        r#type: "hx711".to_string(),
        connection: "gpio".to_string(),
        channel: None,
        route: Default::default(),
        config: [
            ("sck_pin".to_string(), serde_yaml::Value::from(SCK_PIN)),
            ("dt_pin".to_string(), serde_yaml::Value::from(DT_PIN)),
        ]
        .into_iter()
        .collect(),
    });
    let chip = ChipDescriptor::from_file(&chip_path).expect("load stm32l476");
    let mut bus = labwired_core::bus::SystemBus::from_config(&chip, &manifest).expect("build bus");
    bus.write_u32(RCC_AHB2ENR, 0xFF).expect("RCC AHB2ENR");
    Machine::new(IdleCpu::default(), bus)
}

/// Drive SCK through the GPIO OUTPUT register, exactly as `digitalWrite` does.
fn sck(m: &mut Machine<IdleCpu>, high: bool) {
    let (addr, bit) = labwired_core::bus::SystemBus::resolve_pin_odr_pub(&m.bus, SCK_PIN)
        .unwrap_or_else(|| panic!("{SCK_PIN} is not a GPIO output on this chip"));
    let word = m.bus.read_u32(addr).expect("output register reads back");
    let next = if high {
        word | (1 << bit)
    } else {
        word & !(1 << bit)
    };
    m.bus.write_u32(addr, next).expect("drive SCK");
}

/// Read DOUT through the GPIO INPUT register — the same load `digitalRead`
/// performs. Never asks the device what it thinks it drove.
fn dout(m: &mut Machine<IdleCpu>) -> bool {
    let (addr, bit) = labwired_core::bus::SystemBus::resolve_pin_idr_pub(&m.bus, DT_PIN)
        .unwrap_or_else(|| panic!("{DT_PIN} is not a GPIO input on this chip"));
    let word = m.bus.read_u32(addr).expect("input register reads back");
    (word >> bit) & 1 != 0
}

fn run(m: &mut Machine<IdleCpu>, cycles: u64) {
    m.advance(AdvanceRequest::run(Some(cycles)))
        .expect("advance");
}

fn set_input(m: &mut Machine<IdleCpu>, key: &str, value: f64) {
    m.bus
        .set_input(Some("scale"), key, value)
        .unwrap_or_else(|e| panic!("set_input({key}, {value}): {e}"));
}

/// One clock pulse and the bit it shifts out: SCK rises, DOUT is sampled while
/// the clock is high, SCK falls. That is the datasheet's order and the order
/// every HX711 library uses.
fn pulse(m: &mut Machine<IdleCpu>) -> bool {
    sck(m, true);
    let bit = dout(m);
    sck(m, false);
    bit
}

/// Clock a whole 24-bit frame plus its terminating pulse, MSB first.
fn read_frame(m: &mut Machine<IdleCpu>) -> u32 {
    let mut word = 0u32;
    for _ in 0..24 {
        word = (word << 1) | u32::from(pulse(m));
    }
    pulse(m); // the 25th pulse ends the frame
    word
}

// ─── the protocol ──────────────────────────────────────────────────────────

/// The behaviour the deleted model's own unit test asserted, reproduced here
/// over a real bus: 0xABCDEF shifts out MSB first over 24 rising edges, and one
/// more pulse returns the part to ready.
#[test]
fn the_frame_is_twenty_four_bits_msb_first() {
    let mut m = machine();
    run(&mut m, 200_000); // let the derived clock reach the power-on timer
    assert!(!dout(&mut m), "DOUT low means a sample is ready");

    set_input(&mut m, "raw", f64::from(0x0055_AA55));
    assert!(!dout(&mut m), "a fresh sample is ready");

    assert_eq!(read_frame(&mut m), 0x0055_AA55);
    assert!(!dout(&mut m), "the 25th pulse returns the part to ready");
}

/// ⚠️ THE NEGATIVE CONTROL for the whole change. The frame is clocked with the
/// machine ADVANCED NOT AT ALL between the pulses, so nothing the peripheral
/// tick does can be what delivered the edges. Remove
/// `maybe_service_edge_driven_gpio_devices` from the write path and this reads
/// back zero.
#[test]
fn the_frame_completes_with_no_tick_at_all() {
    let mut m = machine();
    run(&mut m, 200_000);
    set_input(&mut m, "raw", 0x0055_AA55 as f64);

    let before = m.bus.current_cycle;
    let word = read_frame(&mut m);
    let after = m.bus.current_cycle;

    assert!(
        after.saturating_sub(before) < 24,
        "the machine advanced {} cycles while the frame was clocked; the tick \
         pass could then have delivered the 24 edges and this test would not be \
         measuring the write hook",
        after - before
    );
    assert_eq!(
        word, 0x0055_AA55,
        "fifty stores inside one tick interval delivered the whole frame, which \
         is only possible through the MMIO write hook"
    );
}

/// A negative sample is 24-bit two's complement on the wire. A model that
/// shifted an unsigned value would put zeros where the sign extension goes.
#[test]
fn a_negative_sample_shifts_out_as_twos_complement() {
    let mut m = machine();
    run(&mut m, 200_000);
    set_input(&mut m, "raw", -1000.0);
    assert_eq!(read_frame(&mut m), (-1000i32 as u32) & 0x00FF_FFFF);
}

/// The `weight` channel is grams at 100 counts per gram — the demo scale the
/// deleted model used — and `expr_scale` is what keeps the sub-gram digits. A
/// rule reading a raw `input(weight)` would truncate to whole grams, so 10.0 g
/// and 10.5 g would shift out the SAME word.
#[test]
fn the_weight_channel_keeps_its_sub_gram_digits() {
    for (grams, counts) in [(10.0, 1000u32), (10.5, 1050), (0.01, 1), (-2.5, 250)] {
        let mut m = machine();
        run(&mut m, 200_000);
        set_input(&mut m, "weight", grams);
        let expected = if grams < 0.0 {
            (-(counts as i32) as u32) & 0x00FF_FFFF
        } else {
            counts
        };
        assert_eq!(
            read_frame(&mut m),
            expected,
            "{grams} g must shift out {expected:#08X}"
        );
    }
}

/// Clocking while no sample is ready must not start a frame. In this model the
/// part is ready from power-on, so the case is reached mid-frame: the bits are
/// consumed in order and a partial frame is not restarted by an extra clock.
#[test]
fn a_frame_in_progress_is_not_restarted_by_a_clock() {
    let mut m = machine();
    run(&mut m, 200_000);
    set_input(&mut m, "raw", f64::from(0x007F_0000));

    // Eight pulses: the top byte, all ones.
    let mut top = 0u32;
    for _ in 0..8 {
        top = (top << 1) | u32::from(pulse(&mut m));
    }
    assert_eq!(top, 0x7F, "the first eight bits are the high byte");

    // The next eight continue the SAME word rather than restarting it.
    let mut next = 0u32;
    for _ in 0..8 {
        next = (next << 1) | u32::from(pulse(&mut m));
    }
    assert_eq!(next, 0x00, "the middle byte of 0x7F0000 is zero");
}

/// A stimulus that arrives MID-FRAME must not corrupt the word being clocked
/// out. The datasheet's frame is atomic; a model that swapped the sample under
/// the master would hand a driver half of one reading and half of another.
#[test]
fn a_stimulus_mid_frame_does_not_corrupt_the_word() {
    let mut m = machine();
    run(&mut m, 200_000);
    set_input(&mut m, "raw", f64::from(0x007F_0000));

    let mut word = 0u32;
    for _ in 0..8 {
        word = (word << 1) | u32::from(pulse(&mut m));
    }
    // Change the sample half way through — the remaining bits still come from
    // the word the frame opened with.
    set_input(&mut m, "raw", 0.0);
    for _ in 0..16 {
        word = (word << 1) | u32::from(pulse(&mut m));
    }
    assert_eq!(word, 0x007F_0000, "the frame carried one sample end to end");

    // …and the NEXT frame carries the new one.
    pulse(&mut m);
    assert_eq!(read_frame(&mut m), 0x0000_0000);
}

// ─── the attach path ───────────────────────────────────────────────────────

/// The descriptor is embedded (so wasm builds carry it) and it keeps its entry
/// in the peripheral MANIFEST. Without `DeclarativeGpioKit` a ported gpio part
/// still attaches — the universal resolver's declarative step finds it — but
/// vanishes from the library, which is how `keypad`, `dht22` and
/// `rotary_encoder` came to be missing from that manifest.
#[test]
fn the_part_is_embedded_and_still_in_the_kit_registry() {
    assert!(
        labwired_config::embedded_device_yaml("hx711").is_some(),
        "hx711.yaml is not embedded — it would be missing in the browser only"
    );
    let kit = labwired_core::peripherals::kit::registry::lookup("hx711")
        .expect("hx711 must still be a registered kit, or it leaves the manifest");
    let meta = kit.metadata();
    assert_eq!(meta.device_type, "hx711");
    assert_eq!(meta.label, "HX711 Load Cell");
    let keys: Vec<&str> = meta.config_keys.iter().map(|k| k.name.as_ref()).collect();
    assert_eq!(keys, vec!["sck_pin", "dt_pin"]);
    let channels: Vec<&str> = meta.inputs.iter().map(|c| c.key.as_ref()).collect();
    assert_eq!(channels, vec!["weight", "raw"]);
}
