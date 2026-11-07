//! CH32V003 STK (QingKe V2 SysTick) driven by WCH's own `Delay_Ms`.
//!
//! `examples/ch32v003f4p6/systick_blink.c` runs `Delay_Ms(1)` from WCH EVT
//! `Debug/debug.c` between PC0 toggles at HCLK = 24 MHz (HSI). The STK counts
//! HCLK/8, so `p_ms` is 3000 counts and one delay is 24,000 CPU cycles. The
//! gate measures that spacing on the pin itself.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{bus::SystemBus, system::riscv::configure_riscv, Machine};

const HALF_PERIOD_CYCLES: u64 = 24_000;
/// Toggle store, call/return and the poll loop's last lap.
const LOOP_OVERHEAD_MAX: u64 = 64;

fn root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Step the firmware and return the `total_cycles` stamp of each PC0 edge.
fn pc0_edges(chip: &ChipDescriptor, want: usize, max_steps: usize) -> (Vec<u64>, u32) {
    let mut bus = SystemBus::from_config(chip, &SystemManifest::default())
        .expect("CH32V003 peripherals and clock gates");
    let image =
        labwired_loader::load_elf(&root().join("tests/fixtures/ch32v003f4p6-systick-blink.elf"))
            .expect("compiled RV32EC firmware ELF");
    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&image).expect("load CH32V003 ELF");
    let gpioc = machine
        .bus
        .find_peripheral_index_by_name("gpioc")
        .expect("GPIOC registered from descriptor");

    let mut previous = None;
    let mut edges = Vec::new();
    for _ in 0..max_steps {
        if machine.step().is_err() {
            break;
        }
        if let Some(state) = machine.bus.peripherals[gpioc].dev.read_gpio_output(0) {
            if previous.is_some_and(|prior| prior != state) {
                edges.push(machine.total_cycles);
                if edges.len() >= want {
                    break;
                }
            }
            previous = Some(state);
        }
    }
    (edges, machine.cpu.mcause)
}

#[test]
fn ch32v003_wch_delay_ms_paces_pc0_at_one_millisecond() {
    let chip = ChipDescriptor::from_file(&root().join("configs/chips/ch32v003f4p6.yaml"))
        .expect("CH32V003 descriptor");
    let (edges, mcause) = pc0_edges(&chip, 4, 200_000);
    assert_eq!(edges.len(), 4, "PC0 edges: {edges:?}");
    for pair in edges.windows(2) {
        let gap = pair[1] - pair[0];
        assert!(
            (HALF_PERIOD_CYCLES..=HALF_PERIOD_CYCLES + LOOP_OVERHEAD_MAX).contains(&gap),
            "Delay_Ms(1) at 24 MHz must take 24,000 cycles plus loop overhead, got {gap} \
             (edges {edges:?})"
        );
    }
    assert_eq!(mcause, 0, "compiled firmware must not trap");
}

#[test]
fn ch32v003_delay_ms_never_returns_without_the_stk() {
    // Negative control: drop the STK from the descriptor. CNTIF can then never
    // set, so the first Delay_Ms must not return and PC0 makes no edge after
    // the first set.
    let yaml = std::fs::read_to_string(root().join("configs/chips/ch32v003f4p6.yaml"))
        .expect("read CH32V003 descriptor");
    let stk = "  - id: \"stk\"\n    type: \"wch_stk\"\n";
    assert_eq!(yaml.matches(stk).count(), 1, "STK entry is unique");
    let chip: ChipDescriptor =
        serde_yaml::from_str(&yaml.replace(stk, "  - id: \"stk\"\n    type: \"stub\"\n"))
            .expect("parse modified descriptor");
    let (edges, _) = pc0_edges(&chip, 2, 100_000);
    assert!(
        edges.len() < 2,
        "Delay_Ms returned with no STK model: edges {edges:?}"
    );
}
