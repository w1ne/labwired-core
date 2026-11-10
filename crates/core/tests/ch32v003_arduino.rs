#![cfg(feature = "ch32v003-arduino-l0-smoke")]

//! CH32V003 running the stock Arduino matrix L0 sketch built with WCH's own
//! Arduino core (`openwch/arduino_core_ch32`, PlatformIO `ch32v`, board
//! `genericCH32V003F4P6`). The matrix CI builds the pinned image in a temporary
//! workspace and supplies its path via `CH32V003_ARDUINO_L0_ELF`; WCH's
//! unlicensed binary is not checked into the repository.
//!
//! The image needs every QingKe piece this file gates:
//! * `csrw 0x804` (INTSYSCR) in `startup_ch32v00x.S` — without the CSR the
//!   write is an illegal instruction and the core loops through reset;
//! * WCH XW compressed loads/stores (`-march=rv32ecxw`);
//! * the PFIC, the STK interrupt (IRQ 12) and the `mtvec` mode-3 absolute
//!   vector table — `delay(1)` waits for `SysTick_Handler` to bump `msTick`;
//! * HPE: `SysTick_Handler` is `interrupt("WCH-Interrupt-fast")` and clobbers
//!   a0/a4/t1/t2 without saving them.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{bus::SystemBus, system::riscv::configure_riscv, Machine};

fn root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn firmware_path() -> std::path::PathBuf {
    std::env::var_os("CH32V003_ARDUINO_L0_ELF")
        .map(std::path::PathBuf::from)
        .expect("matrix CI must build and set CH32V003_ARDUINO_L0_ELF")
}

fn descriptor() -> String {
    std::fs::read_to_string(root().join("configs/chips/ch32v003f4p6.yaml"))
        .expect("read CH32V003 descriptor")
}

struct L0Run {
    serial: String,
    systicks: u32,
    reached_main: bool,
    last_mcause: u32,
}

/// Run the L0 image until `LW_L0_OK\r\n` or `max_steps`.
fn run_l0(chip: &ChipDescriptor, max_steps: usize) -> L0Run {
    let mut bus = SystemBus::from_config(chip, &SystemManifest::default())
        .expect("CH32V003 peripherals and clock gates");
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);
    let elf_path = firmware_path();
    let elf_bytes = std::fs::read(&elf_path).expect("read WCH Arduino L0 ELF");
    let main_pc = labwired_loader::resolve_symbol_in_elf(&elf_bytes, "main")
        .expect("main symbol in WCH Arduino L0 ELF");
    let systick_pc = labwired_loader::resolve_symbol_in_elf(&elf_bytes, "SysTick_Handler")
        .expect("SysTick_Handler symbol in WCH Arduino L0 ELF");
    let image = labwired_loader::load_elf(&elf_path).expect("WCH Arduino L0 ELF");
    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&image).expect("load CH32V003 ELF");
    let mut systicks = 0;
    let mut reached_main = false;
    for _ in 0..max_steps {
        let pc_before = machine.cpu.pc;
        if machine.step().is_err() {
            break;
        }
        reached_main |= pc_before == main_pc || machine.cpu.pc == main_pc;
        // Entering the handler through vector 12 of the mode-3 table.
        if machine.cpu.mcause == 0x8000_000C && machine.cpu.pc == systick_pc {
            systicks += 1;
        }
        if sink.lock().unwrap().ends_with(b"LW_L0_OK\r\n") {
            break;
        }
    }
    let serial = {
        let bytes = sink.lock().unwrap();
        String::from_utf8_lossy(&bytes).into_owned()
    };
    L0Run {
        serial,
        systicks,
        reached_main,
        last_mcause: machine.cpu.mcause,
    }
}

#[test]
fn ch32v003_wch_arduino_l0_boots_and_prints() {
    let chip: ChipDescriptor = serde_yaml::from_str(&descriptor()).expect("CH32V003 descriptor");
    let run = run_l0(&chip, 2_000_000);
    assert!(run.reached_main, "firmware never reached main");
    assert_eq!(run.serial, "LW_L0_OK\r\n", "USART1 TX stream");
    // `delay(1)` returns on the first msTick change, so at least one
    // SysTick_Handler entry, taken through the mode-3 table, preceded it.
    assert!(run.systicks >= 1, "no SysTick interrupt was taken");
    // mcause retains the last taken interrupt after mret; it must be the
    // expected SysTick cause, not a synchronous exception.
    assert_eq!(run.last_mcause, 0x8000_000C, "unexpected final trap cause");
}

#[test]
fn ch32v003_wch_arduino_l0_stalls_when_stk_irq_is_not_routed() {
    // Keep the PFIC attached so INTSYSCR, XW decoding, and startup still work.
    // Removing only STK IRQ 12 isolates interrupt delivery: firmware reaches
    // main, but SysTick_Handler cannot advance msTick and delay(1) never ends.
    let mut yaml: serde_yaml::Value =
        serde_yaml::from_str(&descriptor()).expect("parse descriptor");
    let peripherals_key = serde_yaml::Value::String("peripherals".into());
    let id_key = serde_yaml::Value::String("id".into());
    let irq_key = serde_yaml::Value::String("irq".into());
    let peripherals = yaml
        .as_mapping_mut()
        .and_then(|root| root.get_mut(&peripherals_key))
        .and_then(serde_yaml::Value::as_sequence_mut)
        .expect("descriptor peripherals");
    let stk = peripherals
        .iter_mut()
        .find(|part| {
            part.as_mapping()
                .and_then(|map| map.get(&id_key))
                .and_then(serde_yaml::Value::as_str)
                == Some("stk")
        })
        .expect("STK descriptor entry");
    let stk_map = stk.as_mapping_mut().expect("STK descriptor mapping");
    assert_eq!(
        stk_map.get(&irq_key).and_then(serde_yaml::Value::as_i64),
        Some(12),
        "STK IRQ is the only field changed by this negative control"
    );
    stk_map.remove(&irq_key);
    let chip: ChipDescriptor =
        serde_yaml::from_value(yaml).expect("parse descriptor without the STK IRQ route");
    let run = run_l0(&chip, 1_000_000);
    assert!(run.reached_main, "negative control failed before main");
    assert_eq!(run.last_mcause, 0, "negative control trapped before delay");
    assert_eq!(run.systicks, 0, "SysTick taken without its PFIC route");
    assert!(
        run.serial.is_empty(),
        "printed past delay(1) without the STK IRQ route: {:?}",
        run.serial
    );
}
