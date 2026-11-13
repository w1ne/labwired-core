use labwired_config::CosimModelConfig;
use labwired_core::analog::{parse_netlist, AnalogConfig, AnalogCosimAdapter};
use labwired_core::cosim::{CosimAdapter, CosimSignalValue, CosimSignals, CosimStep};
use serde_json::json;
use std::{env, fs, path::Path, time::Instant};
fn signals(values: CosimSignals) -> serde_json::Value {
    serde_json::Value::Object(values.into_iter().map(|(name,value)| (name,match value {
        CosimSignalValue::Bool(v)=>json!(v),CosimSignalValue::I64(v)=>json!(v),
        CosimSignalValue::F64(v)=>json!(v),CosimSignalValue::Text(v)=>json!(v)
    })).collect())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = env::args().nth(1).ok_or("model.json argument required")?;
    let text = fs::read_to_string(&path)?;
    let model: CosimModelConfig = serde_json::from_str(&text)?;
    let cfg = AnalogConfig::from_yaml(&model.config)?;
    let netlist = model.config.get("netlist_text").and_then(|v| v.as_str()).ok_or("inline netlist required")?;
    let mut circuit = parse_netlist(netlist)?;
    labwired_core::analog::controlled_buck::bind_stages(&mut circuit, &cfg.regulators)?;
    let bound: Vec<_> = circuit.controlled_bucks.iter().map(|b| {
        let c = b.config.initial.command();
        json!({"element":circuit.buck_macros[b.macro_index].name,
            "enabled":c.configuration.enabled,"target_voltage_v":c.configuration.target_voltage_v,
            "current_limit_a":c.configuration.current_limit_a,"rated_load_a":c.configuration.rated_load_a,
            "command_revision":c.command_revision})
    }).collect();
    let mut result = json!({"scope":"production analog constructor only; no MCU or firmware execution",
        "input":path,"model_id":model.id,"base_unknowns":circuit.unknowns(),
        "capacitors":circuit.capacitors.len(),"inductors":circuit.inductors.len(),
        "max_unknowns":cfg.max_unknowns,"requested_initialization":cfg.initialization,
        "bound_initial_commands_before_constructor":bound,"step_ns":model.step_ns});
    let start = Instant::now();
    match AnalogCosimAdapter::from_manifest_config(&model, Path::new(".")) {
        Ok(mut adapter) => {
            result["construction"] = json!("accepted");
            result["construction_seconds"] = json!(start.elapsed().as_secs_f64());
            result["initialization"] = serde_json::to_value(adapter.initialization_status())?;
            let snapshot = adapter.step(CosimStep{time_ns:0,dt_ns:0,inputs:CosimSignals::new()})?;
            result["initial_outputs"] = signals(snapshot.outputs);
            if env::args().nth(2).as_deref()==Some("one-step") {
                let start=Instant::now();
                match adapter.step(CosimStep{time_ns:model.step_ns,dt_ns:model.step_ns,inputs:CosimSignals::new()}) {
                    Ok(step) => {result["one_step"] = json!({"accepted":true,"seconds":start.elapsed().as_secs_f64(),"outputs":signals(step.outputs)});},
                    Err(error) => {result["one_step"] = json!({"accepted":false,"seconds":start.elapsed().as_secs_f64(),"error":error.to_string()});}
                }
            }
        },
        Err(error) => { result["construction"] = json!("refused");result["construction_seconds"] = json!(start.elapsed().as_secs_f64());result["error"] = json!(error.to_string()); }
    }
    println!("{}",serde_json::to_string_pretty(&result)?);
    Ok(())
}
