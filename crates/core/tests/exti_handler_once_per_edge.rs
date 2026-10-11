//! An EXTI handler runs exactly ONCE per external edge, even when it clears the
//! pending bit without checking it first.
//!
//! On silicon, writing 1 to EXTI_PR (RPR1/FPR1 on G0/U5) clears the pending
//! latch and the NVIC line drops; the NVIC pending bit was already cleared on
//! exception entry, so nothing is left to re-enter the handler. The simulator
//! used to re-pend the EXTI IRQ from its held level on every cycle the handler
//! spent before the PR write, and nothing dropped that pend when PR cleared:
//! the handler then ran twice per edge. Firmware that re-checks PR (as HAL
//! does) hid it; firmware that just clears and counts saw double counts.
//!
//! Fixture: `tests/fixtures/exti-once` (one C source, one ELF per family; see
//! its `build.sh`). PA0 drives EXTI line 0 on both edges.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{bus::SystemBus, system::cortex_m::configure_cortex_m, Bus, Machine};
use std::path::{Path, PathBuf};

const EDGES: u32 = 10;
const READY: u32 = 0x600D_F00D;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn machine(chip_file: &str, elf: &str) -> Machine<impl labwired_core::Cpu> {
    let chip = ChipDescriptor::from_file(root().join("configs/chips").join(chip_file)).unwrap();
    let manifest: SystemManifest = serde_yaml::from_str(&format!(
        "name: exti-once\nchip: {chip_file}\nexternal_devices: []\nboard_io: []\n"
    ))
    .unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let (cpu, _) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let image = labwired_loader::load_elf(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/exti-once")
            .join(elf),
    )
    .unwrap();
    machine.load_firmware(&image).unwrap();
    machine
}

/// Boot, drive `EDGES` alternating levels on PA0 with time for the handler to
/// run between them, and return the handler's entry count.
fn entries(chip: &str, elf: &str) -> u32 {
    let mut m = machine(chip, elf);
    for _ in 0..20_000 {
        m.step().unwrap();
        if m.bus.read_u32(0x2000_0104).unwrap() == READY {
            break;
        }
    }
    assert_eq!(
        m.bus.read_u32(0x2000_0104).unwrap(),
        READY,
        "{chip}: firmware never armed EXTI"
    );
    let gpioa = m.bus.find_peripheral_index_by_name("gpioa").unwrap();
    for i in 0..EDGES {
        m.bus.set_peripheral_gpio_input(gpioa, 0, i % 2 == 0);
        for _ in 0..2_000 {
            m.step().unwrap();
        }
    }
    m.bus.read_u32(0x2000_0100).unwrap()
}

#[test]
fn f103_exti_handler_runs_once_per_edge() {
    assert_eq!(entries("stm32f103.yaml", "exti-once-f1.elf"), EDGES);
}

#[test]
fn f401_exti_handler_runs_once_per_edge() {
    assert_eq!(entries("stm32f401.yaml", "exti-once-f4.elf"), EDGES);
}

#[test]
fn g0b1_exti_handler_runs_once_per_edge() {
    assert_eq!(entries("stm32g0b1re.yaml", "exti-once-g0.elf"), EDGES);
}

#[test]
fn u545_exti_handler_runs_once_per_edge() {
    assert_eq!(entries("stm32u545.yaml", "exti-once-u5.elf"), EDGES);
}

/// STM32L0: the F1 register file behind the Cortex-M0+ grouped vectors
/// (EXTI0_1 = IRQ 5), with the port select in `SYSCFG_EXTICR1`.
#[test]
fn l073_exti_handler_runs_once_per_edge() {
    assert_eq!(entries("stm32l073.yaml", "exti-once-l0.elf"), EDGES);
}

#[test]
fn g071_exti_handler_runs_once_per_edge() {
    assert_eq!(entries("stm32g071.yaml", "exti-once-g0.elf"), EDGES);
}
