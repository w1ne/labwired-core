//! CH32V003 running the stock Arduino matrix L0 sketch built with WCH's own
//! Arduino core (`openwch/arduino_core_ch32`, PlatformIO `ch32v`, board
//! `genericCH32V003F4P6`). Provenance: `tests/fixtures/ch32v003f4p6-arduino-l0.md`.
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

fn descriptor() -> String {
    std::fs::read_to_string(root().join("configs/chips/ch32v003f4p6.yaml"))
        .expect("read CH32V003 descriptor")
}

/// Run the L0 image until `LW_L0_OK\r\n` or `max_steps`; return the UART bytes
/// and the number of SysTick interrupts taken.
fn run_l0(chip: &ChipDescriptor, max_steps: usize) -> (String, u32) {
    let mut bus = SystemBus::from_config(chip, &SystemManifest::default())
        .expect("CH32V003 peripherals and clock gates");
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);
    let image =
        labwired_loader::load_elf(&root().join("tests/fixtures/ch32v003f4p6-arduino-l0.elf"))
            .expect("WCH Arduino L0 ELF");
    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&image).expect("load CH32V003 ELF");
    let mut systicks = 0;
    for _ in 0..max_steps {
        if machine.step().is_err() {
            break;
        }
        // Entering SysTick_Handler (vector 12 of the table at 0).
        if machine.cpu.mcause == 0x8000_000C && machine.cpu.pc == 0xaba {
            systicks += 1;
        }
        if sink.lock().unwrap().ends_with(b"LW_L0_OK\r\n") {
            break;
        }
    }
    let out = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
    (out, systicks)
}

#[test]
fn ch32v003_wch_arduino_l0_boots_and_prints() {
    let chip: ChipDescriptor = serde_yaml::from_str(&descriptor()).expect("CH32V003 descriptor");
    let (out, systicks) = run_l0(&chip, 2_000_000);
    assert_eq!(out, "LW_L0_OK\r\n", "USART1 TX stream");
    // `delay(1)` returns on the first msTick change, so at least one
    // SysTick_Handler entry, taken through the mode-3 table, preceded it.
    assert!(systicks >= 1, "no SysTick interrupt was taken");
}

#[test]
fn ch32v003_wch_arduino_l0_hangs_in_delay_without_the_pfic() {
    // Negative control: stub the PFIC. IENR then enables nothing the core
    // can see, SysTick is never taken, msTick never moves, and `delay(1)`
    // before the print never returns.
    let yaml = descriptor();
    let pfic = "  - id: \"pfic\"\n    type: \"wch_pfic\"\n";
    assert_eq!(yaml.matches(pfic).count(), 1, "PFIC entry is unique");
    let chip: ChipDescriptor =
        serde_yaml::from_str(&yaml.replace(pfic, "  - id: \"pfic\"\n    type: \"stub\"\n"))
            .expect("parse modified descriptor");
    let (out, systicks) = run_l0(&chip, 1_000_000);
    assert_eq!(systicks, 0, "SysTick taken with no PFIC");
    assert!(
        out.is_empty(),
        "printed past delay(1) with no PFIC: {out:?}"
    );
}
