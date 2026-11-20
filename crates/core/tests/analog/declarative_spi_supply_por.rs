//! A declarative SPI part (BMA400) takes its powered state from a rail that an
//! analog circuit actually charges and discharges. No ideal UI-driven rail: the
//! MCU's PA8 output switches a source into an RC node, and the SOLVED node
//! voltage is routed to both BMA400 rails (`board.device.<id>.power.*_volts`).
//! SPI traffic is the controller's own register path (STM32 SPI1 + PA4 as CSB),
//! not a call into the device object.
use labwired_config::{ChipDescriptor, EnvironmentManifest, SystemManifest};
use labwired_core::system::node::NodeFirmware;
use labwired_core::world::{MachineTrait, ResolvedWorldNode, World};
use std::path::Path;

const GPIO: u64 = 0x4002_0000;
const RCC: u64 = 0x4002_3800;
const SPI1: u64 = 0x4001_3000;
const ID: &str = "U1";
const RAILS: &str = "{vdd: {min_v: 1.72, max_v: 3.6}, vddio: {min_v: 1.2, max_v: 3.6}}";
const SETTLE_US: u64 = 10;
const CS: u32 = 1 << 4; // PA4
const SWITCH: u32 = 1 << 8; // PA8

fn firmware() -> Vec<u8> {
    // ELF32 ARM with reset vectors and `b .`; the test drives the peripherals.
    let code = [0xe7feu16];
    let length = 8 + code.len() as u32 * 2;
    let mut bytes = vec![0; 84 + length as usize];
    bytes[0..7].copy_from_slice(b"\x7fELF\x01\x01\x01");
    for (offset, value) in [(16, 2u16), (18, 40), (40, 52), (42, 32), (44, 1)] {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }
    for (offset, value) in [
        (20, 1u32),
        (24, 0x0800_0009),
        (28, 52),
        (36, 0x0500_0000),
        (52, 1),
        (56, 84),
        (60, 0x0800_0000),
        (64, 0x0800_0000),
        (68, length),
        (72, length),
        (76, 5),
        (80, 4),
        (84, 0x2000_1000),
        (88, 0x0800_0009),
    ] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    for (i, instruction) in code.iter().enumerate() {
        bytes[92 + i * 2..94 + i * 2].copy_from_slice(&instruction.to_le_bytes());
    }
    bytes
}

/// PA8 -> switch -> 1k -> node (1 uF to ground, 2k bleed). With `source_v` = 3.3
/// the node settles at 2.2 V (above the 1.72 V VDD minimum); with 2.0 it settles
/// at 1.33 V, below it. Both rails are the same solved node.
fn world(source_v: f64) -> World {
    let manifest: EnvironmentManifest = serde_yaml::from_str(&format!(
        r#"
name: bma400-rc-supply
nodes:
  - {{id: a, system: a.yaml, firmware: a.elf}}
cosim_models:
  - id: circuit
    adapter: analog
    step_ns: 1000
    inputs:
      ctl: node.a.board.gpio_pad.pa8
    outputs:
      vdd: node.a.board.device.{ID}.power.vdd_volts
      vddio: node.a.board.device.{ID}.power.vddio_volts
    config:
      substeps: 1
      netlist_text: |
        Vsrc src 0 dc {source_v}
        S1 src drive ctl ron=0.1 roff=1e12
        R1 drive rail 1k
        C1 rail 0 1u
        R2 rail 0 2k
        .end
      probes:
        vdd: v(rail)
        vddio: v(rail)
"#
    ))
    .unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let chip = ChipDescriptor::from_file(root.join("configs/chips/stm32f401.yaml")).unwrap();
    let system: SystemManifest = serde_yaml::from_str(&format!(
        "name: sensor-node\nchip: stm32f401\nexternal_devices:\n  - id: {ID}\n    type: bma400\n    connection: spi1\n    config: {{cs_pin: PA4, supply_rails: {RAILS}}}\n"
    ))
    .unwrap();
    World::from_resolved(
        manifest,
        vec![ResolvedWorldNode {
            id: "a".into(),
            system,
            chip,
            firmware: NodeFirmware::from_bytes(firmware()),
            blobs: Default::default(),
        }],
    )
    .unwrap()
}

struct Bench {
    w: World,
    t_ns: u64,
    switch_on: bool,
}

impl Bench {
    fn new(source_v: f64) -> Self {
        let mut b = Self {
            w: world(source_v),
            t_ns: 0,
            switch_on: false,
        };
        let m = b.m();
        wr(m, RCC + 0x30, 1); // GPIOAEN
        wr(m, RCC + 0x44, 1 << 12); // SPI1EN
        wr(m, GPIO + 0x14, CS); // CSB idles high
        wr(m, GPIO, (1 << 16) | (1 << 8)); // PA8 and PA4 outputs
                                           // Master, software slave management, SPE.
        wr(m, SPI1, (1 << 2) | (1 << 6) | (1 << 8) | (1 << 9));
        b
    }
    fn m(&mut self) -> &mut dyn MachineTrait {
        self.w.machines.get_mut("a").unwrap().as_mut()
    }
    fn odr(&mut self, cs_high: bool) {
        let v = (if cs_high { CS } else { 0 }) | (if self.switch_on { SWITCH } else { 0 });
        wr(self.m(), GPIO + 0x14, v);
        // The controller samples PA4 on its next bus tick, and a frame takes
        // device time: let the world run.
        self.run_for_us(SETTLE_US);
    }
    /// The MCU GPIO output that switches the rail source.
    fn gpio(&mut self, on: bool) {
        self.switch_on = on;
        self.odr(true);
    }
    fn run_for_us(&mut self, us: u64) {
        self.t_ns += us * 1000;
        let results = self.w.run_until_ps(self.t_ns * 1000).expect("analog world");
        assert!(results.values().all(Result::is_ok), "{results:?}");
    }
    fn rail_v(&self) -> f64 {
        let channels = self.w.analog_channels();
        let channel = channels
            .iter()
            .position(|c| c.name == "circuit.vdd")
            .unwrap();
        f64::from(
            self.w
                .analog_trace_snapshot(0)
                .samples
                .last()
                .unwrap()
                .values[channel],
        )
    }
    /// Run until the solved node reaches `volts`; returns the world time (us).
    fn run_until_rail_at_least(&mut self, volts: f64) -> u64 {
        for _ in 0..2000 {
            if self.rail_v() >= volts {
                return self.t_ns / 1000;
            }
            self.run_for_us(5);
        }
        panic!("rail never reached {volts} V (at {})", self.rail_v());
    }
    fn xfer(&mut self, byte: u8) -> u8 {
        self.m().write_u8(SPI1 + 0x0C, byte).unwrap();
        self.run_for_us(SETTLE_US);
        self.m().read_u8(SPI1 + 0x0C).unwrap()
    }
    fn cs_pulse(&mut self) {
        self.odr(false);
        self.odr(true);
    }
    /// Read one register through SPI1 with PA4 as CSB: command, dummy, data.
    fn read(&mut self, addr: u8) -> u8 {
        self.odr(false);
        let _ = self.xfer(0x80 | addr);
        let _ = self.xfer(0);
        let data = self.xfer(0);
        self.odr(true);
        data
    }
    fn write(&mut self, addr: u8, value: u8) {
        self.odr(false);
        self.xfer(addr);
        self.xfer(value);
        self.odr(true);
    }
}

fn wr(machine: &mut dyn MachineTrait, address: u64, value: u32) {
    for byte in 0..4 {
        machine
            .write_u8(address + byte, (value >> (byte * 8)) as u8)
            .unwrap();
    }
}

#[test]
fn rc_charged_rails_power_on_reset_and_startup_gate_the_part() {
    let mut b = Bench::new(3.3);

    // GPIO low: no source, node at 0 V, nothing answers.
    b.run_for_us(1_000);
    assert!(b.rail_v() < 0.01);
    assert_eq!(b.read(0x00), 0xFF);

    // GPIO high: still 0xFF while the node charges toward the 1.72 V minimum.
    b.gpio(true);
    b.run_for_us(100);
    assert!(b.rail_v() < 1.72);
    assert_eq!(b.read(0x00), 0xFF);
    let t_cross = b.run_until_rail_at_least(1.72);
    assert!(
        t_cross > 1_000 + 100,
        "the node, not an ideal source, sets the timing"
    );

    // Inside the 1 ms start-up: powered, but the part does not answer and a
    // CSB rising edge here must not select SPI.
    b.run_for_us(500);
    assert_eq!(b.read(0x00), 0xFF);
    // After start-up, still in I2C mode: the earlier edge was ignored.
    b.run_for_us(700);
    assert!(b.t_ns / 1000 > t_cross + 1_000);
    assert_eq!(b.read(0x00), 0xFF, "start-up CSB edge must not select SPI");
    // Dummy CSB pulse (its rising edge selects SPI), then CHIP_ID.
    b.cs_pulse();
    assert_eq!(b.read(0x00), 0x90);

    // A written register reads back, then the rail is cut.
    b.write(0x1A, 0x09);
    assert_eq!(b.read(0x1A), 0x09);
    b.gpio(false);
    b.run_for_us(10_000);
    assert!(b.rail_v() < 0.1);
    assert_eq!(b.read(0x00), 0xFF, "unpowered");

    // Re-powered: power-on reset, back in I2C mode, registers at reset values.
    b.gpio(true);
    b.run_until_rail_at_least(1.72);
    b.run_for_us(1_500);
    assert_eq!(
        b.read(0x00),
        0xFF,
        "no new CSB edge since power-up: I2C mode, not 0x90"
    );
    assert_eq!(b.read(0x1A), 0x49, "ACC_CONFIG1 back at its reset value");
}

/// Where the 1 ms ts_up boundary falls, measured on the same time base: a CSB
/// edge shortly before it is ignored, one shortly after it selects SPI.
#[test]
fn csb_edge_activates_spi_only_after_the_one_millisecond_startup() {
    for (offset_us, activated) in [(700u64, false), (1_300, true)] {
        let mut b = Bench::new(3.3);
        b.run_for_us(500);
        b.gpio(true);
        let t_cross = b.run_until_rail_at_least(1.72);
        let target = t_cross + offset_us;
        let now = b.t_ns / 1000;
        b.run_for_us(target - now);
        b.cs_pulse();
        b.run_for_us(2_000);
        assert_eq!(
            b.read(0x00) == 0x90,
            activated,
            "CSB edge {offset_us} us after the rail qualified"
        );
    }
}

/// Negative control: the same circuit capped at 1.33 V never reaches the 1.72 V
/// VDD minimum, so the part never answers however long it waits.
#[test]
fn rail_capped_below_vdd_minimum_never_answers() {
    let mut b = Bench::new(2.0);
    b.gpio(true);
    b.run_for_us(20_000);
    let v = b.rail_v();
    assert!(v > 1.2 && v < 1.72, "node settled at {v} V");
    b.cs_pulse();
    assert_eq!(b.read(0x00), 0xFF);
    b.run_for_us(5_000);
    b.cs_pulse();
    assert_eq!(b.read(0x00), 0xFF);
}
