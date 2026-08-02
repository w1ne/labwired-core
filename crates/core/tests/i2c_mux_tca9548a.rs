// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! TCA9548A I²C bus switch, driven through a REAL controller.
//!
//! The unit tests next to the model (`peripherals/components/tca9548a.rs`)
//! prove the switch's own protocol. This file proves the thing that actually
//! blocked the use case: that a controller's address resolution reaches the
//! devices BEHIND the switch. Every controller resolved slaves with a flat
//! `position(|d| d.address() == addr)` — first match wins — so four sensors at
//! the same fixed address collapsed onto one no matter how they were wired.
//!
//! The scenario is the real one: 4 × VCNL4010-class proximity sensors, all at
//! the unchangeable address 0x13, one per switch channel.

use labwired_core::peripherals::components::tca9548a::Tca9548a;
use labwired_core::peripherals::i2c::{I2c, I2cDevice, I2cRegisterLayout};
use labwired_core::sim_input::{InputChannel, SimInput, SimInputError};
use labwired_core::Peripheral;

const MUX_ADDR: u8 = 0x70;
/// VCNL4010's fixed address. The part has no strap pin — this is exactly why a
/// switch is the only way to run four of them.
const SENSOR_ADDR: u8 = 0x13;

// ── A stand-in for the sensor ───────────────────────────────────────────────
//
// Deliberately minimal: one register file, a free-running counter driven by
// `advance_time_us`, and a drivable `proximity` channel. This file tests the
// SWITCH and the controllers' resolution, not a part model — using a real
// sensor here would test the sensor.

struct FakeProximity {
    address: u8,
    /// Identifies which physical sensor answered.
    tag: u8,
    pointer: Option<u8>,
    proximity: u16,
    /// Microseconds this device has been told have elapsed.
    elapsed_us: u64,
    component_id: Option<String>,
}

const CH_PROXIMITY: &[InputChannel] = &[InputChannel {
    key: "proximity",
    label: "Proximity",
    unit: "count",
    min: 0.0,
    max: 65535.0,
}];

impl FakeProximity {
    fn new(tag: u8) -> Self {
        Self {
            address: SENSOR_ADDR,
            tag,
            pointer: None,
            proximity: 0,
            elapsed_us: 0,
            component_id: None,
        }
    }
}

impl SimInput for FakeProximity {
    fn input_channels(&self) -> &'static [InputChannel] {
        CH_PROXIMITY
    }
    fn set_input(&mut self, channel: &str, value: f64) -> Result<(), SimInputError> {
        if channel != "proximity" {
            return Err(SimInputError::UnknownChannel(channel.to_string()));
        }
        self.proximity = value as u16;
        Ok(())
    }
    fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }
    fn set_component_id(&mut self, id: String) {
        self.component_id = Some(id);
    }
}

impl I2cDevice for FakeProximity {
    fn address(&self) -> u8 {
        self.address
    }
    /// The register pointer survives START — including the repeated START of a
    /// write-pointer-then-read transaction, which is how every I²C register
    /// device is read. Clearing it here would break the read sequence.
    fn start(&mut self) {}
    fn stop(&mut self) {}
    fn write(&mut self, data: u8) {
        self.pointer = Some(data);
    }
    fn read(&mut self) -> u8 {
        match self.pointer {
            // 0x81 = "who answered" — the per-sensor tag.
            Some(0x81) => self.tag,
            // 0x87/0x88 = proximity result, high/low byte.
            Some(0x87) => (self.proximity >> 8) as u8,
            Some(0x88) => (self.proximity & 0xFF) as u8,
            // 0x8A = elapsed milliseconds, saturating — proves the
            // free-running clock reached this device.
            Some(0x8A) => (self.elapsed_us / 1000).min(255) as u8,
            _ => 0x00,
        }
    }
    fn advance_time_us(&mut self, us: u64) {
        self.elapsed_us += us;
    }
    fn as_sim_input_mut(&mut self) -> Option<&mut dyn SimInput> {
        Some(self)
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

/// A switch with one sensor per channel 0..=3, all at 0x13, tags 0xA0..0xA3.
fn mux_with_four_sensors() -> Tca9548a {
    let mut mux = Tca9548a::new(MUX_ADDR);
    for ch in 0..4u8 {
        let mut dev = FakeProximity::new(0xA0 + ch);
        dev.set_component_id(format!("prox{ch}"));
        mux.attach(ch, Box::new(dev)).unwrap();
    }
    mux
}

// ── STM32F1 legacy I²C driver helpers ───────────────────────────────────────
//
// Bare-register sequences, matching what `tests/i2c_component.rs` does for the
// MPU6050: this exercises the controller's real transaction state machine, not
// a shortcut into the device.

fn f1_start(i2c: &mut I2c) {
    // Clear any latched error (SR1 bits 8..15 are rc_w0) so a NACK from an
    // earlier transaction does not leak into this one's verdict.
    i2c.write(0x15, 0x00).unwrap();
    i2c.write(0x00, 0x01).unwrap(); // CR1.PE
    i2c.write(0x01, 0x01).unwrap(); // CR1.START
    for _ in 0..10 {
        i2c.tick();
    }
}

fn f1_stop(i2c: &mut I2c) {
    i2c.write(0x01, 0x02).unwrap(); // CR1.STOP
    for _ in 0..10 {
        i2c.tick();
    }
}

fn f1_addr(i2c: &mut I2c, addr: u8, reading: bool) {
    i2c.write(0x10, (addr << 1) | u8::from(reading)).unwrap();
    for _ in 0..40 {
        i2c.tick();
    }
}

fn f1_byte(i2c: &mut I2c, byte: u8) {
    i2c.write(0x10, byte).unwrap();
    for _ in 0..20 {
        i2c.tick();
    }
}

/// Write one byte to `addr` (no register pointer) — the TCA9548A control write.
fn f1_write_byte(i2c: &mut I2c, addr: u8, byte: u8) {
    f1_start(i2c);
    f1_addr(i2c, addr, false);
    f1_byte(i2c, byte);
    f1_stop(i2c);
}

/// Write a register pointer to `addr`, repeated-START, read one byte back.
fn f1_read_reg(i2c: &mut I2c, addr: u8, reg: u8) -> u8 {
    f1_start(i2c);
    f1_addr(i2c, addr, false);
    f1_byte(i2c, reg);
    // Repeated START, then read.
    i2c.write(0x01, 0x01).unwrap();
    for _ in 0..10 {
        i2c.tick();
    }
    f1_addr(i2c, addr, true);
    let b = i2c.read(0x10).unwrap();
    f1_stop(i2c);
    b
}

/// Did the last address phase get an ACK? SR1.AF (bit 10) is the NACK flag;
/// `peek` is byte-wide, so bit 10 is bit 2 of the byte at 0x15.
fn f1_acked(i2c: &I2c) -> bool {
    i2c.peek(0x15).unwrap() & (1 << 2) == 0
}

fn f1_bus_with_mux() -> I2c {
    let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Stm32F1);
    let trace = labwired_core::bus::bus_trace::new_log();
    i2c.attach_traced("i2c1", &trace, Box::new(mux_with_four_sensors()));
    i2c
}

// ── (a) four identical addresses, each independently reachable ──────────────

/// THE regression this work exists for. Before `claims_address`, the
/// controller's flat first-match resolution meant only the first 0x13 on the
/// bus was ever reachable — and with the switch in the way, none of them was.
#[test]
fn four_sensors_at_one_address_answer_independently_through_a_controller() {
    let mut i2c = f1_bus_with_mux();

    for ch in 0..4u8 {
        f1_write_byte(&mut i2c, MUX_ADDR, 1 << ch);
        let tag = f1_read_reg(&mut i2c, SENSOR_ADDR, 0x81);
        assert_eq!(
            tag,
            0xA0 + ch,
            "channel {ch} must be answered by the sensor wired to channel {ch}"
        );
    }
}

// ── (b) channel switching actually changes who answers ──────────────────────

#[test]
fn switching_channels_changes_which_sensor_answers() {
    let mut i2c = f1_bus_with_mux();

    // Drive each sensor to a distinct value through the stimulus API, then
    // prove the value read back tracks the selected channel.
    for ch in 0..4u8 {
        f1_write_byte(&mut i2c, MUX_ADDR, 1 << ch);
        let expect = 0x1100u16 + ch as u16;
        set_proximity_on_channel(&mut i2c, ch, expect);
    }

    // Out of order on purpose: a stale selection would show up as the previous
    // channel's value.
    for ch in [2u8, 0, 3, 1, 3, 0] {
        f1_write_byte(&mut i2c, MUX_ADDR, 1 << ch);
        let hi = f1_read_reg(&mut i2c, SENSOR_ADDR, 0x87);
        let lo = f1_read_reg(&mut i2c, SENSOR_ADDR, 0x88);
        assert_eq!(
            u16::from_be_bytes([hi, lo]),
            0x1100 + ch as u16,
            "reading after selecting channel {ch}"
        );
    }
}

/// Reach into the model to set a sensor's proximity. Uses the switch's public
/// channel accessor rather than any bus transaction, so the assertion above is
/// about the BUS path only.
fn set_proximity_on_channel(i2c: &mut I2c, channel: u8, value: u16) {
    let cell = &i2c.attached_devices()[0];
    let mut traced = cell.borrow_mut();
    let mux = traced
        .as_any_mut()
        .and_then(|a| a.downcast_mut::<Tca9548a>())
        .expect("slave 0 is the switch");
    // `channel_devices` is read-only by design (the wiring is not editable at
    // runtime); drive through the stimulus seam instead, which is the API an
    // agent would use.
    let mut done = false;
    mux.for_each_sim_input(&mut |si| {
        if si.component_id() == Some(&format!("prox{channel}")) {
            si.set_input("proximity", value as f64).unwrap();
            done = true;
            return true;
        }
        false
    });
    assert!(done, "no sensor stamped 'prox{channel}' behind the switch");
}

// ── (c) the switch's own control register ───────────────────────────────────

#[test]
fn control_register_reads_back_over_the_bus() {
    let mut i2c = f1_bus_with_mux();

    f1_write_byte(&mut i2c, MUX_ADDR, 0b0000_1010);

    // The TCA9548A has no register pointer: a plain read returns the control
    // register, so address it directly for a read.
    f1_start(&mut i2c);
    f1_addr(&mut i2c, MUX_ADDR, true);
    assert!(f1_acked(&i2c), "the switch must ACK its own address");
    let readback = i2c.read(0x10).unwrap();
    f1_stop(&mut i2c);

    assert_eq!(readback, 0b0000_1010);
}

#[test]
fn an_unselected_sensor_address_is_nacked_like_an_empty_bus() {
    let mut i2c = f1_bus_with_mux();

    // Reset state: every channel isolated, so 0x13 is on no reachable segment.
    f1_start(&mut i2c);
    f1_addr(&mut i2c, SENSOR_ADDR, false);
    assert!(
        !f1_acked(&i2c),
        "with all channels disabled the sensor address must NACK, exactly as an \
         empty bus does — a switch that ACKs everything hides the missing select"
    );
    f1_stop(&mut i2c);

    // Enable channel 1 and the very same address now ACKs.
    f1_write_byte(&mut i2c, MUX_ADDR, 1 << 1);
    f1_start(&mut i2c);
    f1_addr(&mut i2c, SENSOR_ADDR, false);
    assert!(f1_acked(&i2c), "channel 1 is enabled; 0x13 must ACK");
    f1_stop(&mut i2c);
}

// ── (d) simultaneous channels: a real bus collision ─────────────────────────

/// The control register is a BITMASK, so firmware can legally enable two
/// channels at once. Two sensors then drive SDA together. I²C is open-drain, so
/// the wire carries the AND of what they drive and the master reads garbage.
/// Reproducing that is the point — silently returning the first channel's byte
/// is precisely the failure mode this work removes.
#[test]
fn two_enabled_channels_collide_instead_of_silently_picking_one() {
    let mut mux = Tca9548a::new(MUX_ADDR);
    mux.attach(0, Box::new(FakeProximity::new(0b1111_0000)))
        .unwrap();
    mux.attach(1, Box::new(FakeProximity::new(0b1100_1100)))
        .unwrap();

    let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Stm32F1);
    let trace = labwired_core::bus::bus_trace::new_log();
    i2c.attach_traced("i2c1", &trace, Box::new(mux));

    // One channel at a time: clean, unambiguous reads.
    f1_write_byte(&mut i2c, MUX_ADDR, 0b0000_0001);
    assert_eq!(f1_read_reg(&mut i2c, SENSOR_ADDR, 0x81), 0b1111_0000);
    f1_write_byte(&mut i2c, MUX_ADDR, 0b0000_0010);
    assert_eq!(f1_read_reg(&mut i2c, SENSOR_ADDR, 0x81), 0b1100_1100);

    // Both at once: the wired-AND of the two.
    f1_write_byte(&mut i2c, MUX_ADDR, 0b0000_0011);
    let collided = f1_read_reg(&mut i2c, SENSOR_ADDR, 0x81);
    assert_eq!(
        collided, 0b1100_0000,
        "an open-drain bus with two talkers carries the AND of what they drive"
    );
    assert_ne!(collided, 0b1111_0000, "must not silently pick channel 0");
    assert_ne!(collided, 0b1100_1100, "must not silently pick channel 1");
}

/// The other half of a multi-channel select, and the reason drivers do it: a
/// write is broadcast to every enabled channel.
#[test]
fn a_write_with_several_channels_enabled_reaches_all_of_them() {
    let mut i2c = f1_bus_with_mux();

    f1_write_byte(&mut i2c, MUX_ADDR, 0b0000_1111);
    // One transaction sets the register pointer on all four sensors at once.
    f1_start(&mut i2c);
    f1_addr(&mut i2c, SENSOR_ADDR, false);
    f1_byte(&mut i2c, 0x81);
    f1_stop(&mut i2c);

    // Now read each one back individually: all four must have latched 0x81.
    for ch in 0..4u8 {
        f1_write_byte(&mut i2c, MUX_ADDR, 1 << ch);
        f1_start(&mut i2c);
        f1_addr(&mut i2c, SENSOR_ADDR, true);
        let tag = i2c.read(0x10).unwrap();
        f1_stop(&mut i2c);
        assert_eq!(
            tag,
            0xA0 + ch,
            "channel {ch} must have received the broadcast pointer write"
        );
    }
}

// ── (e) free-running clocks reach downstream devices ────────────────────────

/// A sensor behind an OPEN pass-gate is still powered and still sampling. If
/// the switch only advanced the enabled channels, a FIFO would stop filling the
/// moment firmware looked away — which is exactly the CPU-starvation class the
/// `advance_time_us` hook exists to expose.
#[test]
fn advance_time_reaches_every_sensor_behind_the_switch() {
    let mut i2c = f1_bus_with_mux();

    // Only channel 0 is connected to the master for the whole advance.
    f1_write_byte(&mut i2c, MUX_ADDR, 0b0000_0001);
    i2c.advance_attached_i2c_us(50_000);

    for ch in 0..4u8 {
        f1_write_byte(&mut i2c, MUX_ADDR, 1 << ch);
        let ms = f1_read_reg(&mut i2c, SENSOR_ADDR, 0x8A);
        assert_eq!(
            ms, 50,
            "sensor on channel {ch} must have advanced 50 ms even while isolated"
        );
    }
}

// ── stimulus reachability ───────────────────────────────────────────────────

/// A switch must not SUBTRACT reachability: putting a sensor behind one has to
/// leave it drivable through the same stimulus walk it had on a bare bus.
#[test]
fn sensors_behind_the_switch_stay_reachable_from_the_stimulus_walk() {
    let mut mux = Tca9548a::new(MUX_ADDR);
    for ch in 0..4u8 {
        let mut dev = FakeProximity::new(0xA0 + ch);
        dev.set_component_id(format!("prox{ch}"));
        mux.attach(ch, Box::new(dev)).unwrap();
    }
    let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Stm32F1);
    let trace = labwired_core::bus::bus_trace::new_log();
    i2c.attach_traced("i2c1", &trace, Box::new(mux));

    let mut seen: Vec<String> = Vec::new();
    i2c.for_each_attached_sim_input(&mut |si| {
        seen.push(si.component_id().unwrap_or("?").to_string());
        false
    });
    seen.sort();
    assert_eq!(
        seen,
        vec!["prox0", "prox1", "prox2", "prox3"],
        "every sensor behind the switch must appear in the ONE stimulus walk"
    );
}

// ── no-mux behaviour is unchanged ───────────────────────────────────────────

/// The trait defaults must leave a plain bus byte-identical. A device that
/// never heard of `claims_address` / `select_address` resolves exactly as
/// before, including the first-match tie-break for a genuine address clash.
#[test]
fn a_bus_without_a_switch_resolves_exactly_as_before() {
    let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Stm32F1);
    let trace = labwired_core::bus::bus_trace::new_log();
    i2c.attach_traced("i2c1", &trace, Box::new(FakeProximity::new(0x11)));
    i2c.attach_traced("i2c1", &trace, Box::new(FakeProximity::new(0x22)));

    // Two devices at the same address on a bare bus: first attached wins, which
    // is what the flat resolution always did.
    assert_eq!(f1_read_reg(&mut i2c, SENSOR_ADDR, 0x81), 0x11);

    // An unpopulated address still NACKs.
    f1_start(&mut i2c);
    f1_addr(&mut i2c, 0x55, false);
    assert!(!f1_acked(&i2c));
    f1_stop(&mut i2c);
}

// ── manifest wiring ─────────────────────────────────────────────────────────

fn ext(
    id: &str,
    ty: &str,
    connection: &str,
    channel: Option<u8>,
    config: &[(&str, serde_yaml::Value)],
) -> labwired_config::ExternalDevice {
    labwired_config::ExternalDevice {
        id: id.to_string(),
        r#type: ty.to_string(),
        connection: connection.to_string(),
        channel,
        route: Default::default(),
        config: config
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
    }
}

/// `channel:` is optional, so every manifest written before this change must
/// still deserialize — and must still mean "not behind a switch".
#[test]
fn an_external_device_without_channel_still_deserializes() {
    let yaml = r#"
id: tmp
type: tmp102
connection: i2c1
config:
  i2c_address: 0x48
"#;
    let dev: labwired_config::ExternalDevice = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(dev.channel, None);
    assert_eq!(dev.connection, "i2c1");

    // And it round-trips without growing a `channel:` key.
    let out = serde_yaml::to_string(&dev).unwrap();
    assert!(!out.contains("channel"), "got:\n{out}");
}

#[test]
fn channel_deserializes_when_present() {
    let yaml = r#"
id: prox0
type: tmp102
connection: mux
channel: 3
"#;
    let dev: labwired_config::ExternalDevice = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(dev.channel, Some(3));
    assert_eq!(dev.connection, "mux");
}

fn manifest_with(devices: Vec<labwired_config::ExternalDevice>) -> labwired_config::SystemManifest {
    labwired_config::SystemManifest {
        external_devices: devices,
        ..manifest_skeleton()
    }
}

fn manifest_skeleton() -> labwired_config::SystemManifest {
    serde_yaml::from_str(
        r#"
name: i2c-mux-test
chip: stm32f103
"#,
    )
    .expect("skeleton manifest")
}

/// The grouping pass: a manifest that hangs sensors off the switch's id builds
/// ONE bus slave (the switch) carrying all four.
#[test]
fn manifest_groups_devices_onto_switch_channels() {
    let mut devices = vec![ext("mux", "tca9548a", "i2c1", None, &[])];
    for ch in 0..4u8 {
        devices.push(ext(
            &format!("prox{ch}"),
            "tmp102",
            "mux",
            Some(ch),
            &[("i2c_address", serde_yaml::Value::from(0x48))],
        ));
    }
    let manifest = manifest_with(devices);

    let children = labwired_core::peripherals::components::i2c_mux_child_ids(&manifest);
    assert_eq!(children, vec!["prox0", "prox1", "prox2", "prox3"]);

    let device = labwired_core::peripherals::components::build_i2c_tree(
        &manifest,
        &manifest.external_devices[0],
    )
    .unwrap()
    .expect("tca9548a must build");

    let mux = device
        .as_any()
        .and_then(|a| a.downcast_ref::<Tca9548a>())
        .expect("the assembled unit is the switch itself");
    assert_eq!(mux.address(), MUX_ADDR);
    for ch in 0..4u8 {
        assert_eq!(
            mux.channel_devices(ch).len(),
            1,
            "one sensor bucketed onto channel {ch}"
        );
        assert_eq!(mux.channel_devices(ch)[0].address(), 0x48);
    }
    assert!(mux.channel_devices(4).is_empty());
}

#[test]
fn strap_pins_choose_the_switch_address() {
    let manifest = manifest_with(vec![ext(
        "mux",
        "tca9548a",
        "i2c1",
        None,
        &[
            ("a0", serde_yaml::Value::from(true)),
            ("a2", serde_yaml::Value::from(true)),
        ],
    )]);
    let device = labwired_core::peripherals::components::build_i2c_tree(
        &manifest,
        &manifest.external_devices[0],
    )
    .unwrap()
    .unwrap();
    assert_eq!(device.address(), 0x75, "0x70 | A0 | (A2 << 2)");
}

#[test]
fn topology_validation_rejects_a_wiring_mistake() {
    // Hanging a device off a device that is not a switch.
    let m = manifest_with(vec![
        ext("tmp", "tmp102", "i2c1", None, &[]),
        ext("other", "tmp102", "tmp", Some(0), &[]),
    ]);
    let err = labwired_core::peripherals::components::validate_i2c_mux_topology(&m).unwrap_err();
    assert!(
        err.to_string().contains("only an I²C bus switch"),
        "got: {err}"
    );

    // Channel out of range.
    let m = manifest_with(vec![
        ext("mux", "tca9548a", "i2c1", None, &[]),
        ext("tmp", "tmp102", "mux", Some(9), &[]),
    ]);
    let err = labwired_core::peripherals::components::validate_i2c_mux_topology(&m).unwrap_err();
    assert!(err.to_string().contains("channel 9"), "got: {err}");

    // A type with no I²C model behind a switch has no fallback path, so it must
    // fail loudly rather than silently attach straight to the controller.
    let m = manifest_with(vec![
        ext("mux", "tca9548a", "i2c1", None, &[]),
        ext("panel", "oled-ssd1306", "mux", Some(0), &[]),
    ]);
    let err = labwired_core::peripherals::components::validate_i2c_mux_topology(&m).unwrap_err();
    assert!(err.to_string().contains("no I²C model"), "got: {err}");

    // A `connection` cycle must not recurse forever.
    let m = manifest_with(vec![
        ext("a", "tca9548a", "b", Some(0), &[]),
        ext("b", "tca9548a", "a", Some(0), &[]),
    ]);
    let err = labwired_core::peripherals::components::validate_i2c_mux_topology(&m).unwrap_err();
    assert!(err.to_string().contains("cycle"), "got: {err}");
}

/// A manifest with no switch at all must validate clean and report no children
/// — the no-change guarantee for every existing system.yaml.
#[test]
fn a_manifest_without_a_switch_is_untouched() {
    let m = manifest_with(vec![
        ext("tmp", "tmp102", "i2c1", None, &[]),
        ext("imu", "mpu6050", "i2c1", None, &[]),
    ]);
    assert!(
        labwired_core::peripherals::components::validate_i2c_mux_topology(&m)
            .unwrap()
            .is_empty()
    );
    assert!(labwired_core::peripherals::components::i2c_mux_child_ids(&m).is_empty());
}
