// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **Phase C's gate**: a declarative I²C part drives a real GPIO pad on a real
//! machine.
//!
//! Why this file and not a unit test
//! =================================
//! `pcf8574_migration_parity.rs` proves the DEVICE queues the right
//! `(role, level)` pairs. That is half the claim and the easy half. The other
//! half is four things that only exist on a bus:
//!
//!   1. the `outputs:` role resolved to a pad at attach, through a `config:`
//!      key, on a chip whose GPIO map the descriptor knows nothing about;
//!   2. the queue reaching the bus at all — an I²C slave lives inside its
//!      CONTROLLER, so the drain goes device → controller
//!      (`Peripheral::drain_attached_pin_drives`) → bus;
//!   3. the per-tick service pass actually running, which means the walk-free
//!      fast path must NOT have been taken;
//!   4. the pad word the CPU reads actually changing.
//!
//! Every assertion below reads the pad through the BUS (`read_u32` of the input
//! register), never through the device. A test that asked the device what it
//! drove would pass with all four of those unwired — which is exactly the
//! "attach succeeds, set_input returns Ok, and the pin never moves" failure the
//! resident-device tests were written about, arriving through a new door.
//!
//! The chip is the NUCLEO-L476RG, deliberately: it has no absolute-µs counter,
//! so its device time is the Phase A clock DERIVED from `cpu_hz`. A timer-driven
//! interrupt line here is the honest hard case, not the easy one.

use labwired_config::{ChipDescriptor, ExternalDevice, SystemManifest};
use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;
use labwired_core::peripherals::i2c::I2cDevice;
use labwired_core::snapshot::{ArmCpuSnapshot, CpuSnapshot};
use labwired_core::{
    AdvanceRequest, Bus, Cpu, Machine, SimResult, SimulationConfig, SimulationObserver,
};
use std::path::PathBuf;
use std::sync::Arc;

mod common;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// A CPU that executes nothing useful: one step, one cycle, no instruction
/// stream. Every test here drives the machine's CLOCK — the point is that a
/// device's own oscillator moves a pin with no firmware involved.
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

/// Build a machine from the shipped NUCLEO-L476RG manifest plus one placed
/// device, optionally carrying a part pack.
fn machine(
    ext: ExternalDevice,
    pack: Option<labwired_config::DeviceDescriptor>,
) -> Machine<IdleCpu> {
    let system_path = repo_root().join("configs/systems/nucleo-l476rg.yaml");
    let mut manifest = SystemManifest::from_file(&system_path).expect("load nucleo-l476rg.yaml");
    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    if let Some(pack) = pack {
        manifest
            .parts
            .push(labwired_config::PartPack::Inline(Box::new(pack)));
    }
    manifest.external_devices.push(ext);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load stm32l476");
    let mut bus = labwired_core::bus::SystemBus::from_config(&chip, &manifest).expect("build bus");
    // What firmware does first, and what this test must do because it runs no
    // firmware: ungate the GPIO port clocks (RCC AHB2ENR). The STM32L4 model is
    // faithful here — with the clock off the port's whole register file is dead,
    // reads return 0 and writes are dropped. A test that skipped this would
    // measure the clock gate rather than the pin drive, and would keep passing
    // if the pin drive were removed.
    bus.write_u32(RCC_AHB2ENR, 0xFF).expect("RCC AHB2ENR");
    Machine::new(IdleCpu::default(), bus)
}

/// RCC AHB2ENR on the STM32L4: bit *n* ungates GPIO port *n* (A…H).
const RCC_AHB2ENR: u64 = 0x4002_104C;

fn placed(id: &str, device_type: &str, config: &[(&str, serde_yaml::Value)]) -> ExternalDevice {
    ExternalDevice {
        id: id.to_string(),
        r#type: device_type.to_string(),
        connection: "i2c1".to_string(),
        channel: None,
        route: Default::default(),
        config: config
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
    }
}

/// Read one pad's level THROUGH THE BUS — the same input register the CPU
/// would load. Never asks the device what it thinks it drove.
fn pad(m: &mut Machine<IdleCpu>, label: &str) -> bool {
    let (addr, bit) = labwired_core::bus::SystemBus::resolve_pin_idr_pub(&m.bus, label)
        .unwrap_or_else(|| panic!("{label} is not a GPIO input on this chip"));
    let word = m.bus.read_u32(addr).expect("input register reads back");
    (word >> bit) & 1 != 0
}

/// Reach the one attached declarative device, to drive its wire directly. (The
/// same helper `device_time_derived.rs` uses; see its SAFETY note.)
fn device(m: &mut Machine<IdleCpu>) -> &mut GenericI2cDevice {
    let ctrl = m
        .bus
        .peripherals
        .iter_mut()
        .filter_map(|p| {
            p.dev
                .as_any_mut()?
                .downcast_mut::<labwired_core::peripherals::i2c::I2c>()
        })
        .find(|i| !i.attached_devices().is_empty())
        .expect("a generic I2c controller hosting the device under test");
    let cell = &ctrl.attached_devices()[0];
    let ptr: *mut GenericI2cDevice = cell
        .borrow_mut()
        .as_any_mut()
        .and_then(|a| a.downcast_mut::<GenericI2cDevice>())
        .expect("attached declarative device") as *mut _;
    // SAFETY: identical to `device_time_derived::attached_generic_device` — the
    // RefCell borrow ends with the statement above and the box it points at is
    // owned by `ctrl`, which outlives the returned reference.
    unsafe { &mut *ptr }
}

/// One I²C-write transaction against the device under test.
fn write_bytes(m: &mut Machine<IdleCpu>, bytes: &[u8]) {
    let dev = device(m);
    dev.start();
    for &b in bytes {
        dev.write(b);
    }
    dev.stop();
}

/// Read one byte with NO pointer write — the PCF8574 framing, where a byte
/// written would be the port itself.
fn read_byte(m: &mut Machine<IdleCpu>) -> u8 {
    let dev = device(m);
    dev.start();
    let b = dev.read();
    dev.stop();
    b
}

/// Point at `reg` and read one byte, the ordinary driver framing.
fn read_reg(m: &mut Machine<IdleCpu>, reg: u8) -> u8 {
    let dev = device(m);
    dev.start();
    dev.write(reg);
    dev.start();
    let b = dev.read();
    dev.stop();
    b
}

/// Run the machine for `cycles`, which is what makes the per-tick service pass
/// happen. Nothing here depends on what the CPU executed.
fn run(m: &mut Machine<IdleCpu>, cycles: u64) {
    m.advance(AdvanceRequest::run(Some(cycles)))
        .expect("advance");
}

// ─── the expander: an I²C write moves eight pads ───────────────────────────

/// The shipped PCF8574 descriptor, placed on i2c1 with two of its eight pads
/// wired. A write to the port must show up on the pads the CPU samples.
#[test]
fn a_pcf8574_write_moves_the_pads_the_cpu_reads() {
    let mut m = machine(
        placed(
            "expander",
            "pcf8574",
            &[
                ("p0_pin", serde_yaml::Value::from("PC0")),
                ("p7_pin", serde_yaml::Value::from("PC1")),
            ],
        ),
        None,
    );

    // Power-on is all-ones, but nothing has been DRIVEN yet: the pads hold
    // whatever the board leaves them at. Run a tick so the (empty) queue is
    // drained, then latch a pattern.
    run(&mut m, 100);

    write_bytes(&mut m, &[0b1000_0001]); // P0 high, P7 high
    run(&mut m, 10);
    assert!(pad(&mut m, "PC0"), "P0 was written 1");
    assert!(pad(&mut m, "PC1"), "P7 was written 1");

    write_bytes(&mut m, &[0b1000_0000]); // P0 low, P7 still high
    run(&mut m, 10);
    assert!(
        !pad(&mut m, "PC0"),
        "⚠️ P0 was written 0 and the pad the CPU reads did not follow. The device \
         queues the transition (pcf8574_migration_parity proves that), so what \
         failed is the bus half: the attach-time pad binding, the controller \
         drain, or the per-tick service pass."
    );
    assert!(pad(&mut m, "PC1"), "P7 did not change");

    write_bytes(&mut m, &[0xFF]);
    run(&mut m, 10);
    assert!(
        pad(&mut m, "PC0") && pad(&mut m, "PC1"),
        "all released high"
    );
}

/// An `outputs:` role whose `config:` key the placement does not set is not an
/// error — an expander wired for two LEDs is an ordinary board. The part must
/// still work over the wire, and the unbound pads must simply go nowhere.
#[test]
fn an_unbound_output_role_is_not_an_error() {
    let mut m = machine(placed("expander", "pcf8574", &[]), None);
    run(&mut m, 100);
    write_bytes(&mut m, &[0x5A]);
    run(&mut m, 10);
    assert_eq!(read_byte(&mut m), 0x5A, "the wire still works");
}

// ─── the interrupt line: a timer inside the part raises a pad ──────────────

/// An MPU6050-shaped part, carried as a manifest part pack.
///
/// This is the plan's Phase C example verbatim in shape — sample timer, INT
/// output, `INT_STATUS` cleared by the read — and it is carried as a PACK
/// rather than an in-tree descriptor on purpose: the pack path is the one an
/// LLM-generated part arrives through, so proving the gate on it proves the
/// thing the phase is actually for.
///
/// It is not the shipped `mpu6050` part. That model is still hand-written Rust
/// (see the PR body for what its port needs that this phase does not add).
fn mpu6050_shaped_pack() -> labwired_config::DeviceDescriptor {
    labwired_config::DeviceDescriptor::from_yaml(
        r#"
schema: labwired.part/v1
type: tier2_imu_probe
behavior:
  primitive: i2c_device
  i2c:
    default_address: 0x68
    auto_increment: true
    unmapped_byte: 0x00
    registers:
      - { name: WHO_AM_I, addr: 0x75, width: 1, endian: be, access: r, reset: 0x68 }
      - name: PWR_MGMT_1
        addr: 0x6B
        width: 1
        endian: be
        access: rw
        reset: 0x40
        bits: [ { name: SLEEP, shift: 6 } ]
      - name: INT_ENABLE
        addr: 0x38
        width: 1
        endian: be
        access: rw
        reset: 0x00
        bits: [ { name: DATA_RDY_EN, shift: 0 } ]
      - name: INT_STATUS
        addr: 0x3A
        width: 1
        endian: be
        access: r
        reset: 0x00
        bits: [ { name: DATA_RDY, shift: 0 } ]
      - { name: ACCEL_XOUT, addr: 0x3B, width: 2, endian: be, access: r, signed: true,
          source: ax, encode: { scale: 16384 } }
  outputs: [INT]
  output_pins: { INT: int_pin }
  timers:
    # The part's own sample clock, idle until firmware leaves sleep. `on_fire:`
    # is empty on purpose: what a sample DOES here is a rule, not a register
    # action, and this proves a timer can drive rules alone.
    - { name: sample, period_us: 1000, start: manual, on_fire: [] }
  rules:
    # Leaving SLEEP starts the sample clock; entering it stops everything.
    - on: { write: PWR_MGMT_1 }
      when: "field(PWR_MGMT_1.SLEEP) == 0"
      do: [ { timer: sample, start: true } ]
    - on: { write: PWR_MGMT_1 }
      when: "field(PWR_MGMT_1.SLEEP) == 1"
      do: [ { timer: sample, start: false }, { clear: INT_STATUS.DATA_RDY }, { pin: INT, level: 0 } ]
    # A completed sample always sets the status bit; it only reaches the PIN
    # when firmware enabled the interrupt, which is what the datasheet says.
    - on: { timer: sample }
      do: [ { set: INT_STATUS.DATA_RDY } ]
    - on: { timer: sample }
      when: "field(INT_ENABLE.DATA_RDY_EN) == 1"
      do: [ { pin: INT, level: 1 } ]
    # Reading INT_STATUS clears the flag and drops the line.
    - on: { read: INT_STATUS }
      do: [ { clear: INT_STATUS.DATA_RDY }, { pin: INT, level: 0 } ]
metadata:
  label: "Tier-2 IMU probe"
  inputs:
    - { key: ax, label: "Accel X", unit: g, min: -16, max: 16, default: 0 }
"#,
    )
    .expect("the probe pack is a valid part document")
}

/// ⚠️ THE PHASE C GATE. Firmware-free: the machine's clock is the only thing
/// that moves, and the INT pad must rise on the part's own sample timer and
/// fall when firmware reads INT_STATUS.
///
/// Every level is read through the bus pad. On `origin/main` there is no
/// `outputs:`, no rule machine and no drain, so this cannot even be written.
#[test]
fn an_int_pad_rises_on_the_sample_timer_and_falls_when_int_status_is_read() {
    let mut m = machine(
        placed(
            "imu",
            "tier2_imu_probe",
            &[("int_pin", serde_yaml::Value::from("PC0"))],
        ),
        Some(mpu6050_shaped_pack()),
    );
    // The board manifest's MSI reset rate; the derived device clock is only
    // honest if it uses the number the product ships.
    assert_eq!(m.bus.cpu_hz, 4_000_000, "nucleo-l476rg.yaml declares 4 MHz");
    let cycles_per_us = m.bus.cpu_hz / 1_000_000;

    run(&mut m, 100);
    assert!(
        !pad(&mut m, "PC0"),
        "the line is idle before the part is woken"
    );

    // Enable the data-ready interrupt, then clear SLEEP to start sampling.
    write_bytes(&mut m, &[0x38, 0x01]); // INT_ENABLE.DATA_RDY_EN = 1
    write_bytes(&mut m, &[0x6B, 0x00]); // PWR_MGMT_1: leave sleep

    // Well inside the 1 ms sample period: nothing has been sampled yet.
    run(&mut m, 500 * cycles_per_us);
    assert!(
        !pad(&mut m, "PC0"),
        "⚠️ the INT line cannot be asserted before the first sample period \
         elapses — a line that is simply always high would pass every later \
         assertion in this test"
    );

    // Past it.
    run(&mut m, 600 * cycles_per_us);
    assert!(
        pad(&mut m, "PC0"),
        "the sample timer came due and must have raised INT"
    );
    assert_eq!(
        read_reg(&mut m, 0x3A) & 0x01,
        0x01,
        "INT_STATUS.DATA_RDY reads set at the same moment"
    );

    // The read above cleared the flag and dropped the line. The pad follows on
    // the next service pass.
    run(&mut m, 10);
    assert!(
        !pad(&mut m, "PC0"),
        "reading INT_STATUS drops the interrupt line"
    );
    assert_eq!(read_reg(&mut m, 0x3A) & 0x01, 0, "and the flag stays clear");

    // It re-arms on the next period, which is what makes a polling loop work.
    run(&mut m, 1_100 * cycles_per_us);
    assert!(pad(&mut m, "PC0"), "the next sample raises it again");
}

/// The interrupt ENABLE bit gates the pin, not the status bit.
///
/// ⚠️ Read the second assertion, not the first: "the pad stayed low" also holds
/// when the pin drive is entirely unwired, so only the STATUS-bit arm has teeth
/// here. The pad-drive arms with teeth are in
/// `a_pcf8574_write_moves_the_pads_the_cpu_reads`,
/// `an_int_pad_rises_on_the_sample_timer_and_falls_when_int_status_is_read` and
/// `sleep_stops_the_sample_timer` — all three go red when
/// `service_device_pin_drives` is removed from the tick (verified by hand). A model that
/// raised the pad regardless would let firmware that never enabled the
/// interrupt pass here and hang on silicon waiting for an edge.
#[test]
fn the_int_pad_stays_low_while_the_interrupt_is_disabled() {
    let mut m = machine(
        placed(
            "imu",
            "tier2_imu_probe",
            &[("int_pin", serde_yaml::Value::from("PC0"))],
        ),
        Some(mpu6050_shaped_pack()),
    );
    let cycles_per_us = m.bus.cpu_hz / 1_000_000;
    run(&mut m, 100);

    // Wake the part WITHOUT enabling the interrupt.
    write_bytes(&mut m, &[0x6B, 0x00]);
    run(&mut m, 5_000 * cycles_per_us);

    assert!(!pad(&mut m, "PC0"), "INT_ENABLE.DATA_RDY_EN is clear");
    assert_eq!(
        read_reg(&mut m, 0x3A) & 0x01,
        0x01,
        "but the status bit still reports the sample — the datasheet's split"
    );
}

/// Going back to sleep stops the part's clock: the line drops and stays down.
#[test]
fn sleep_stops_the_sample_timer() {
    let mut m = machine(
        placed(
            "imu",
            "tier2_imu_probe",
            &[("int_pin", serde_yaml::Value::from("PC0"))],
        ),
        Some(mpu6050_shaped_pack()),
    );
    let cycles_per_us = m.bus.cpu_hz / 1_000_000;
    run(&mut m, 100);
    write_bytes(&mut m, &[0x38, 0x01]);
    write_bytes(&mut m, &[0x6B, 0x00]);
    run(&mut m, 2_000 * cycles_per_us);
    assert!(pad(&mut m, "PC0"), "sampling");

    write_bytes(&mut m, &[0x6B, 0x40]); // SLEEP
    run(&mut m, 5_000 * cycles_per_us);
    assert!(!pad(&mut m, "PC0"), "a sleeping part asserts nothing");
    assert_eq!(
        read_reg(&mut m, 0x3A) & 0x01,
        0,
        "and reports no new sample"
    );
}

/// A rule cannot invent a pin. An `outputs:` role a rule drives must be
/// declared, and a typo must be a LOAD error — not a queue entry the bus
/// silently drops, which is how a mis-wired part passes its own tests.
#[test]
fn a_rule_driving_an_undeclared_pin_fails_to_load() {
    let mut pack = mpu6050_shaped_pack();
    pack.r#type = "tier2_imu_typo".into();
    pack.behavior.outputs = vec!["NOT_INT".into()];
    pack.behavior.output_pins.clear();
    let system_path = repo_root().join("configs/systems/nucleo-l476rg.yaml");
    let mut manifest = SystemManifest::from_file(&system_path).unwrap();
    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    manifest
        .parts
        .push(labwired_config::PartPack::Inline(Box::new(pack)));
    manifest
        .external_devices
        .push(placed("imu", "tier2_imu_typo", &[]));
    let chip = ChipDescriptor::from_file(&chip_path).unwrap();
    let err = match labwired_core::bus::SystemBus::from_config(&chip, &manifest) {
        Ok(_) => panic!("a rule naming an undeclared output must not load"),
        Err(e) => e,
    };
    let text = format!("{err:#}");
    assert!(
        text.contains("INT") && text.contains("outputs"),
        "the error must name the pin and where it should have been declared: {text}"
    );
}

// ─── framing: a command shell whose unit of work is a message ──────────────

/// A fixed-length frame closes on its LAST BYTE, not on the transaction
/// boundary, and a short message still reaches the rules at the boundary.
///
/// Both halves matter and they fail in opposite directions. Without the
/// length arm, a master that streams two commands inside one transaction gets
/// one frame and the second command is lost. Without the boundary arm, a
/// truncated command is swallowed and the part waits forever for a byte that
/// is not coming — the shape a command shell has to reject, not hang on.
#[test]
fn a_frame_closes_on_its_length_and_again_at_the_transaction_boundary() {
    use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;

    let mut dev = GenericI2cDevice::from_yaml(
        r#"
schema: labwired.part/v1
type: tier2_frame_probe
behavior:
  primitive: i2c_device
  i2c:
    default_address: 0x40
    pointer_bytes: 0
    registers:
      - { name: DATA, addr: 0x00, width: 1, endian: be, access: rw, reset: 0x00 }
  vars: { frames: 0, last: 0 }
  frames: { length: 3 }
  rules:
    - on: frame
      do: [ { var: frames, value: "var(frames) + 1" }, { var: last, value: "written" } ]
"#,
        0x40,
    )
    .expect("the frame probe is a valid part document");

    let machine = |d: &GenericI2cDevice| {
        let m = d.rule_machine().expect("the probe declares rules");
        (m.var("frames"), m.var("last"))
    };

    dev.start();
    dev.write(0x11);
    dev.write(0x22);
    assert_eq!(machine(&dev), (0, 0), "two bytes is not a frame");
    dev.write(0x33);
    assert_eq!(
        machine(&dev),
        (1, 0x33),
        "the third byte closes the frame, mid-transaction, and `written` is it"
    );

    // A second command in the SAME transaction is a second frame.
    dev.write(0x44);
    dev.write(0x55);
    dev.write(0x66);
    assert_eq!(machine(&dev), (2, 0x66));

    // A SHORT message: two bytes, then STOP. The boundary closes it.
    dev.write(0x77);
    dev.write(0x88);
    assert_eq!(machine(&dev), (2, 0x66), "still mid-frame");
    dev.stop();
    assert_eq!(
        machine(&dev).0,
        3,
        "the transaction boundary delivers the short frame rather than swallowing it"
    );

    // And the counter reset with it: the next three bytes are one frame, not
    // one byte's worth of leftover plus two.
    dev.start();
    dev.write(0x01);
    dev.write(0x02);
    assert_eq!(machine(&dev).0, 3, "two bytes into the next frame");
    dev.write(0x03);
    assert_eq!(machine(&dev), (4, 0x03));
}

/// A part that declares NO `frames:` must see no `frame` event at all — the
/// transaction boundary is not a frame for a register sensor, and raising one
/// there would fire a rule the author did not write.
#[test]
fn a_part_without_framing_never_sees_a_frame_event() {
    use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;

    let mut dev = GenericI2cDevice::from_yaml(
        r#"
schema: labwired.part/v1
type: tier2_no_frame_probe
behavior:
  primitive: i2c_device
  i2c:
    default_address: 0x40
    registers:
      - { name: DATA, addr: 0x00, width: 1, endian: be, access: rw, reset: 0x00 }
  vars: { frames: 0 }
  rules:
    - on: frame
      do: [ { var: frames, value: "var(frames) + 1" } ]
"#,
        0x40,
    )
    .expect("valid part document");

    for _ in 0..3 {
        dev.start();
        dev.write(0x00);
        dev.write(0x5A);
        dev.stop();
    }
    assert_eq!(
        dev.rule_machine().unwrap().var("frames"),
        0,
        "no `frames:` block ⇒ no frame events"
    );
}
