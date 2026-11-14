use labwired_config::{BoardIoKind, BoardIoSignal, ChipDescriptor, SystemManifest};
use std::path::PathBuf;

#[test]
fn controllino_mini_system_loads_as_atmega328p_with_distinct_io() {
    let system = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../configs/systems/controllino-mini.yaml");
    let m = SystemManifest::from_file(&system).expect("CONTROLLINO MINI YAML must parse");
    assert_eq!(m.name, "controllino-mini");
    assert_eq!(m.cpu_hz, Some(16_000_000));
    assert_eq!(m.board_io.len(), 10);
    let in0 = m.board_io.iter().find(|b| b.id == "in0").unwrap();
    assert_eq!(in0.kind, BoardIoKind::Button);
    assert_eq!(in0.signal, BoardIoSignal::Input);
    assert_eq!(in0.peripheral, "portd");
    assert_eq!(in0.pin, 2);
    let d0 = m.board_io.iter().find(|b| b.id == "d0").unwrap();
    assert_eq!(d0.kind, BoardIoKind::Led);
    assert_eq!(d0.signal, BoardIoSignal::Output);
    assert_eq!(d0.pin, 4);
    let d7 = m.board_io.iter().find(|b| b.id == "d7").unwrap();
    assert_eq!(d7.peripheral, "portc");
    assert_eq!(d7.pin, 5);
    let chip = system.parent().unwrap().join(&m.chip);
    let c = ChipDescriptor::from_file(&chip).expect("referenced MCU YAML must parse");
    assert_eq!(c.name, "atmega328p");
}
