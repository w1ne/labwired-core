#![allow(dead_code)]
use labwired_config::InternalRegulatorConfig;
use labwired_core::analog::{controlled_buck::BuckStageConfig, AnalogConfig};
use labwired_core::cosim::{CosimSignalValue, CosimSignals};
use labwired_core::regulator::{RegulatorCommand, REGULATOR_INPUT_FIELDS};
use std::collections::BTreeMap;
pub const DESCRIPTOR: &str = "
id: converter
kind: buck
control_peripheral: witness
input_domain: input
enable_domain: enable
pins: {switch_output: [LX], feedback: [FB], ground: [PGND]}
initial: {enabled: true, target_voltage_v: 1.1, current_limit_a: 0.24, rated_load_a: 0.2}
model: {max_duty: 1, quiescent_current_a: 0.001, output_resistance_ohms: 0.01, efficiency: 0.8, open_loop_gain: 100000}
";
pub fn descriptor() -> InternalRegulatorConfig {
    serde_yaml::from_str(DESCRIPTOR).unwrap()
}
pub fn stage() -> BuckStageConfig {
    serde_yaml::from_str("element: Xbuck
enable_node: avdd
input_min_v: 2.7
input_max_v: 5.5
enable_min_v: 3.135
enable_max_v: 3.63
initial: {enabled: true, target_voltage_v: 1.1, current_limit_a: 0.24, rated_load_a: 0.2, command_revision: 0}
").unwrap()
}
pub fn config() -> AnalogConfig {
    AnalogConfig {
        substeps: 1,
        regulators: BTreeMap::from([("reg".into(), stage())]),
        sources: BTreeMap::from([
            ("avdd".into(), "Vavdd".into()),
            ("vin".into(), "Vin".into()),
        ]),
        probes: BTreeMap::from([("feedback".into(), "v(out)".into())]),
        ..Default::default()
    }
}
pub fn deck(vin: f64, avdd: f64, load: f64) -> String {
    format!(
        "* Controlled BUCK fixture, explicit ideal model assumptions
Vin in 0 {vin}
Vavdd avdd 0 {avdd}
.model buck BUCK(VREF=1.1 ILIM=0.24 IQ=0.001 ROUT=0.01 EFF=0.8 AOL=100000 DMAX=1)
Xbuck in sw 0 out buck
Lexternal sw lr 4.7u
Rdcr lr out 0.01
Cexternal out 0 10u
Rload out 0 {load}
.end
"
    )
}
pub fn frame(command: RegulatorCommand) -> CosimSignals {
    REGULATOR_INPUT_FIELDS
        .iter()
        .zip(labwired_core::analog::controlled_buck::command_inputs(
            command,
        ))
        .map(|(field, value)| (format!("reg.{field}"), value))
        .collect()
}
pub fn number(signals: &CosimSignals, name: &str) -> f64 {
    match signals[name] {
        CosimSignalValue::F64(value) => value,
        _ => panic!("not numeric {name}"),
    }
}
