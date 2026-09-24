//! Modulshop order #49213 device descriptors parse and register.

use crate::DeviceDescriptor;

#[test]
fn logic_level_shifter_4ch_parses_and_embeds() {
    let yaml = crate::embedded_device_yaml("logic_level_shifter_4ch")
        .expect("logic_level_shifter_4ch is embedded");
    let d = DeviceDescriptor::from_yaml(yaml).expect("parses");
    assert_eq!(d.r#type, "logic_level_shifter_4ch");
    assert_eq!(d.behavior.primitive, "logic_gate");
    let logic = d.behavior.logic.as_ref().expect("logic block");
    let dir = logic
        .direction
        .as_ref()
        .expect("DIR transceiver shape required");
    assert_eq!(dir.a.len(), 4);
    assert_eq!(dir.b.len(), 4);
    assert!(crate::embedded_device_yaml("logic-level-shifter-4ch").is_some());
}

#[test]
fn xl4015_parses_and_embeds() {
    let yaml = crate::embedded_device_yaml("xl4015").expect("xl4015 is embedded");
    let d = DeviceDescriptor::from_yaml(yaml).expect("parses");
    assert_eq!(d.r#type, "xl4015");
    assert_eq!(d.behavior.primitive, "analog_source");
    assert!(d.behavior.analog.is_some());
    let inputs: Vec<_> = d
        .metadata
        .as_ref()
        .unwrap()
        .inputs
        .iter()
        .map(|i| i.key.as_str())
        .collect();
    assert!(inputs.contains(&"vout_set_v"));
    assert!(inputs.contains(&"vin_v"));
    assert!(inputs.contains(&"ilim_a"));
}

#[test]
fn bldc_hall_driver_parses_and_embeds() {
    let yaml =
        crate::embedded_device_yaml("bldc_hall_driver").expect("bldc_hall_driver is embedded");
    let d = DeviceDescriptor::from_yaml(yaml).expect("parses");
    assert_eq!(d.r#type, "bldc_hall_driver");
    assert_eq!(d.behavior.primitive, "gpio_device");
    assert!(
        d.behavior.outputs.iter().any(|o| o == "HALL_A"),
        "Hall outputs present"
    );
    assert!(crate::embedded_device_yaml("bldc-hall-driver").is_some());
    let plant = crate::embedded_device_yaml("bldc-motor")
        .or_else(|| crate::embedded_device_yaml("bldc_motor"));
    if let Some(plant_yaml) = plant {
        let p = DeviceDescriptor::from_yaml(plant_yaml).unwrap();
        assert_ne!(p.r#type, d.r#type);
    }
}
