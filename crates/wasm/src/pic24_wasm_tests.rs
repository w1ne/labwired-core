use crate::WasmSimulator;
use labwired_config::BoardIoBinding;
use labwired_core::system::arch_policy::MachineFamily;
use std::fs;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn xc16_m0_inputs() -> (String, String, Vec<u8>) {
    let root = repo_root();
    let chip = fs::read_to_string(root.join("configs/chips/dspic33fj32mc204.yaml"))
        .expect("read PIC24 chip descriptor");
    let system = r#"
name: "dspic33fj32mc204-wasm-smoke"
chip: "../chips/dspic33fj32mc204.yaml"
external_devices: []
board_io:
  - id: "rb0-led"
    kind: "led"
    peripheral: "gpiob"
    pin: 0
    signal: "output"
    active_high: true
"#
    .to_owned();
    let firmware = fs::read(
        root.join("examples/dspic33fj32mc204/firmware/dspic33fj32mc204-xc16-uart-smoke.elf"),
    )
    .expect("read committed XC16 fixture");
    (system, chip, firmware)
}

fn rb0_state(sim: &WasmSimulator) -> bool {
    let machine = sim.machine.as_ref().expect("PIC24 machine");
    let binding: BoardIoBinding = sim
        .board_io
        .iter()
        .find(|binding| binding.id == "rb0-led")
        .expect("RB0 board IO binding")
        .clone();
    sim.read_board_io_state(machine, &binding)
}

#[test]
fn browser_constructor_runs_pic24_m0_xc16_uart_and_gpio() {
    let (system, chip, firmware) = xc16_m0_inputs();
    let mut sim =
        WasmSimulator::new_from_config(&system, &chip, &firmware, wasm_bindgen::JsValue::NULL)
            .expect("construct PIC24 browser simulator");

    assert_eq!(sim.arch, MachineFamily::Pic24);
    sim.step_batch(300).expect("step XC16 PIC24 firmware");
    assert_eq!(sim.drain_uart_output(), b"HI");

    let initial = rb0_state(&sim);
    let mut toggled = false;
    for _ in 0..12 {
        sim.step_single().expect("step PIC24 GPIO loop");
        if rb0_state(&sim) != initial {
            toggled = true;
            break;
        }
    }
    assert!(
        toggled,
        "the XC16 LATB0 loop must reach the browser board IO state"
    );
}
