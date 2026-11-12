use labwired_config::{ChipDescriptor, CosimAdapter, CosimModelConfig, SystemManifest};
use labwired_core::regulator::{
    RegulatorControl, RegulatorObservation, REGULATOR_INPUT_FIELDS, REGULATOR_OUTPUT_FIELDS,
};
use labwired_core::{bus::SystemBus, Peripheral, SimResult};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
};

pub const REGULATOR_YAML:&str="id: converter
kind: buck
control_peripheral: witness
input_domain: input
enable_domain: enable
pins: {switch_output: [LX], feedback: [FB], ground: [PGND]}
initial: {enabled: true, target_voltage_v: 1.1, current_limit_a: 0.24, rated_load_a: 0.2}
model: {max_duty: 1, quiescent_current_a: 0.001, output_resistance_ohms: 0.01, efficiency: 0.8, open_loop_gain: 100000}
";
pub const POWER_YAML:&str="rails:
  core: {nominal_v: 1.1, min_v: 1.05, max_v: 1.16, pins: [CORE]}
  io: {nominal_v: 3.3, min_v: 1.62, max_v: 3.63, pins: [IO]}
  input: {nominal_v: 3.3, min_v: 2.7, max_v: 5.5, pins: [VIN]}
  enable: {nominal_v: 3.3, min_v: 3.135, max_v: 3.63, pins: [AVDD]}
io_domain: io
execution_domains: [core]
system_reset_scope: core_only
latched_por: {sense_domain: core, detector_domain: enable, rearm_below_v: 1.1, release_at_v: 0.957, release_delay_ns: 3000}
";
pub type Observations = Arc<Mutex<Vec<RegulatorObservation>>>;
#[derive(Debug)]
struct Witness {
    control: Option<RegulatorControl>,
    observed: Observations,
}
impl Peripheral for Witness {
    fn read(&self, _: u64) -> SimResult<u8> {
        Ok(u8::from(self.control.as_ref().unwrap().pending_update()))
    }
    fn write(&mut self, _: u64, value: u8) -> SimResult<()> {
        let control = self.control.as_ref().unwrap();
        let mut config = control.configuration();
        config.enabled = value != 0;
        control
            .request(config)
            .map_err(labwired_core::SimulationError::Other)
    }
    fn attach_regulator_control(&mut self, control: RegulatorControl) -> Result<(), String> {
        self.control = Some(control);
        Ok(())
    }
    fn sync_regulator_control(&mut self) {
        self.observed.lock().unwrap().push(
            self.control
                .as_ref()
                .unwrap()
                .observation()
                .expect("published observation"),
        );
    }
    fn on_node_reset(&mut self) {
        self.control.as_ref().unwrap().reset_to_defaults().unwrap();
    }
}
pub fn bus() -> (SystemBus, RegulatorControl, Observations) {
    let mut chip = ChipDescriptor::from_file(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/stm32f401.yaml"),
    )
    .unwrap();
    chip.supply_monitor = None;
    chip.power_domains = None;
    let manifest: SystemManifest = serde_yaml::from_str(
        "name: regulator-fixture
chip: stm32f401
",
    )
    .unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    bus.cpu_hz = 16_000_000;
    let observed = Arc::new(Mutex::new(Vec::new()));
    bus.add_peripheral(
        "witness",
        0x6000_0000,
        4,
        None,
        Box::new(Witness {
            control: None,
            observed: observed.clone(),
        }),
    );
    chip.power_domains = Some(serde_yaml::from_str(POWER_YAML).unwrap());
    chip.internal_regulators = vec![serde_yaml::from_str(REGULATOR_YAML).unwrap()];
    bus.attach_mcu_power_domains(&chip).unwrap();
    bus.attach_internal_regulators(&chip).unwrap();
    let control = bus.regulators.get("converter").unwrap().control.clone();
    (bus, control, observed)
}
pub fn model(nodes: &[(&str, f64)], world: bool) -> CosimModelConfig {
    let mut inputs = HashMap::new();
    let mut outputs = HashMap::new();
    let mut sources = HashMap::new();
    let mut probes = HashMap::new();
    let mut regulators = serde_yaml::Mapping::new();
    let mut deck = "* Shared physical input, independently controlled internal stages
Vin in 0 3.3
.model buck BUCK(VREF=1.1 ILIM=0.24 IQ=0.001 ROUT=0.01 EFF=0.8 AOL=100000 DMAX=1)
"
    .to_string();
    for (index, (node, volts)) in nodes.iter().enumerate() {
        let prefix = format!("reg_{index}");
        let qualify = if world {
            format!("node.{node}.")
        } else {
            String::new()
        };
        deck.push_str(&format!(
            "Ven_{node} en_{node} 0 {volts}
X_{node} in sw_{node} 0 out_{node} buck
L_{node} sw_{node} lr_{node} 4.7u
Rdcr_{node} lr_{node} out_{node} 0.01
C_{node} out_{node} 0 10u
Rload_{node} out_{node} 0 11
"
        ));
        for field in REGULATOR_INPUT_FIELDS {
            inputs.insert(
                format!("{prefix}.{field}"),
                format!("{qualify}board.regulator.converter.{field}"),
            );
        }
        for field in REGULATOR_OUTPUT_FIELDS {
            outputs.insert(
                format!("{prefix}.{field}"),
                format!("{qualify}board.regulator.converter.{field}"),
            );
        }
        inputs.insert(format!("enable_{node}"), format!("ui.{node}.enable"));
        sources.insert(format!("enable_{node}"), format!("Ven_{node}"));
        for (domain, net) in [
            ("core", format!("out_{node}")),
            ("io", "in".to_string()),
            ("input", "in".to_string()),
            ("enable", format!("en_{node}")),
        ] {
            let port = format!("{node}_{domain}");
            outputs.insert(port.clone(), format!("{qualify}board.power.{domain}_volts"));
            probes.insert(port, format!("v({net})"));
        }
        let value:serde_yaml::Value=serde_yaml::from_str(&format!("element: X_{node}
enable_node: en_{node}
input_min_v: 2.7
input_max_v: 5.5
enable_min_v: 3.135
enable_max_v: 3.63
initial: {{enabled: true, target_voltage_v: 1.1, current_limit_a: 0.24, rated_load_a: 0.2, command_revision: 0}}
")).unwrap();
        regulators.insert(prefix.into(), value);
    }
    deck.push_str(
        ".end
",
    );
    let config = HashMap::from([
        ("netlist_text".into(), serde_yaml::Value::String(deck)),
        ("substeps".into(), 1.into()),
        ("sources".into(), serde_yaml::to_value(sources).unwrap()),
        ("probes".into(), serde_yaml::to_value(probes).unwrap()),
        ("regulators".into(), serde_yaml::Value::Mapping(regulators)),
    ]);
    CosimModelConfig {
        id: "power".into(),
        adapter: CosimAdapter::Analog,
        model: None,
        step_ns: 1000,
        inputs,
        outputs,
        config,
    }
}
pub fn machine() -> (
    labwired_core::Machine<labwired_core::cpu::CortexM>,
    RegulatorControl,
    Observations,
) {
    let (mut bus, control, observed) = bus();
    let (cpu, _) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    let mut machine = labwired_core::Machine::new(cpu, bus);
    let mut image = labwired_core::memory::ProgramImage::new(0x0800_0009, labwired_core::Arch::Arm);
    let mut bytes = Vec::new();
    bytes.extend(0x2000_1000u32.to_le_bytes());
    bytes.extend(0x0800_0009u32.to_le_bytes());
    bytes.extend(0x3001u16.to_le_bytes());
    bytes.extend(0xe7fdu16.to_le_bytes());
    image.add_segment(0x0800_0000, bytes);
    machine.load_firmware(&image).unwrap();
    (machine, control, observed)
}
