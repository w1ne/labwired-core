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

fn ka102_inputs() -> (String, String, Vec<u8>) {
    let root = repo_root();
    let chip = fs::read_to_string(root.join("configs/chips/pic24f16ka102.yaml"))
        .expect("read PIC24F16KA102 chip descriptor");
    let mut system = fs::read_to_string(root.join("examples/pic24f16ka102/system.yaml"))
        .expect("read PIC24F16KA102 system manifest");
    // The checked-in system describes a bare chip. Add only a Playground-style
    // RA0 observer so the browser constructor test can assert the firmware's
    // actual package GPIO output without inventing a development board.
    system.push_str(
        r#"
board_io:
  - id: "ra0-led"
    kind: "led"
    peripheral: "gpioa"
    pin: 0
    signal: "output"
    active_high: true
"#,
    );
    let firmware =
        fs::read(root.join("examples/pic24f16ka102/firmware/pic24f16ka102-xc16-uart-smoke.elf"))
            .expect("read committed PIC24F16KA102 XC16 fixture");
    (system, chip, firmware)
}

fn ra0_state(sim: &WasmSimulator) -> bool {
    let machine = sim.machine.as_ref().expect("PIC24 machine");
    let binding: BoardIoBinding = sim
        .board_io
        .iter()
        .find(|binding| binding.id == "ra0-led")
        .expect("RA0 board IO binding")
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

#[test]
fn browser_constructor_runs_pic24f16ka102_xc16_uart_and_gpio() {
    let (system, chip, firmware) = ka102_inputs();
    let mut sim =
        WasmSimulator::new_from_config(&system, &chip, &firmware, wasm_bindgen::JsValue::NULL)
            .expect("construct PIC24F16KA102 browser simulator");

    assert_eq!(sim.arch, MachineFamily::Pic24);
    sim.step_batch(400)
        .expect("step PIC24F16KA102 XC16 firmware");
    assert_eq!(sim.drain_uart_output(), b"HI");

    let initial = ra0_state(&sim);
    let mut toggled = false;
    for _ in 0..12 {
        sim.step_single().expect("step PIC24F16KA102 RA0 loop");
        if ra0_state(&sim) != initial {
            toggled = true;
            break;
        }
    }
    assert!(
        toggled,
        "PIC24F16KA102 XC16 LATA0 loop must reach browser board IO state"
    );
}
