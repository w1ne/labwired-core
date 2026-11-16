//! ESP32-C2 (ESP8684H4X) bare-ELF slice: a committed RV32IMC fixture built
//! from examples/esp32c2/main.c runs from the IROM window, keeps its stack at
//! the top of the 256 KiB DRAM view, prints on UART0 and toggles GPIO10.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{bus::SystemBus, system::riscv::configure_riscv, Bus, Machine};

const FIXTURE: &str = "tests/fixtures/esp32c2-smoke.elf";

fn root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn descriptor() -> ChipDescriptor {
    ChipDescriptor::from_file(root().join("configs/chips/esp32c2.yaml"))
        .expect("ESP32-C2 descriptor")
}

struct Run {
    uart: Vec<u8>,
    edges: [u32; 2],
    mcause: u32,
}

/// Runs the fixture and counts output edges on GPIO10 (the driven pin) and
/// GPIO9 (a negative control the firmware never touches).
fn run(chip: &ChipDescriptor, steps: usize) -> Run {
    let mut bus =
        SystemBus::from_config(chip, &SystemManifest::default()).expect("ESP32-C2 peripherals");
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);
    let image = labwired_loader::load_elf(&root().join(FIXTURE)).expect("compiled RV32IMC ELF");
    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&image).expect("load ESP32-C2 ELF");
    let gpio = machine
        .bus
        .find_peripheral_index_by_name("gpio")
        .expect("GPIO registered from descriptor");

    let mut previous = [None; 2];
    let mut edges = [0; 2];
    for _ in 0..steps {
        machine.step().expect("compiled RV32IMC instruction");
        for (slot, pin) in [10u8, 9].into_iter().enumerate() {
            if let Some(state) = machine.bus.peripherals[gpio].dev.read_gpio_output(pin) {
                if previous[slot].is_some_and(|prior| prior != state) {
                    edges[slot] += 1;
                }
                previous[slot] = Some(state);
            }
        }
    }
    let uart = sink.lock().unwrap().clone();
    Run {
        uart,
        edges,
        mcause: machine.cpu.mcause,
    }
}

#[test]
fn esp32c2_compiled_rv32imc_firmware_prints_on_uart0_and_toggles_gpio10() {
    // The firmware prints the hello only if its MUL/DIVU result and the
    // write/readback of the last DRAM word (0x3FCDFFFC) both match.
    let run = run(&descriptor(), 600_000);
    assert_eq!(
        String::from_utf8_lossy(&run.uart),
        "Hello from ESP32-C2\r\n",
        "UART0 TX stream"
    );
    assert!(
        run.edges[0] >= 2,
        "GPIO10 produced only {} edges",
        run.edges[0]
    );
    assert_eq!(run.edges[1], 0, "GPIO9 must stay idle");
    assert_eq!(run.mcause, 0, "compiled firmware must not trap");
}

#[test]
fn esp32c2_dram_ends_at_0x3fce0000() {
    // soc.h SOC_DRAM_HIGH = 0x3FCE0000: the last DRAM word is mapped, the next
    // word is not. The fixture's stack and readback depend on this edge.
    let chip = descriptor();
    let bus = SystemBus::from_config(&chip, &SystemManifest::default()).expect("bus");
    assert!(bus.read_u32(0x3FCD_FFFC).is_ok(), "last DRAM word mapped");
    assert!(
        bus.read_u32(0x3FCE_0000).is_err(),
        "0x3FCE0000 must be unmapped on the C2 (C3 DRAM continues to 0x3FCE4000)"
    );
}

#[test]
fn esp32c2_descriptor_matches_esp8684h4x_product_data() {
    // Espressif product selector, ESP8684H4X: 120 MHz, 4 MB flash, 576 KB ROM.
    // esp-idf v5.3 esp32c2 soc.h: DRAM 0x3FCA0000..0x3FCE0000.
    let chip = descriptor();
    assert_eq!(chip.cpu_hz, 120_000_000);
    assert_eq!(chip.flash.base, 0x4200_0000);
    assert_eq!(chip.flash.size, 4 * 1024 * 1024);
    assert_eq!(chip.ram.base, 0x3FCA_0000);
    assert_eq!(chip.ram.size, 256 * 1024);
    let rom = chip
        .memory_regions
        .iter()
        .find(|r| r.name == "rom")
        .expect("mask ROM window");
    assert_eq!((rom.base, rom.size), (0x4000_0000, 576 * 1024));
    // One timer group: reg_base.h declares no DR_REG_TIMERGROUP1_BASE.
    let ids: Vec<&str> = chip.peripherals.iter().map(|p| p.id.as_str()).collect();
    assert!(ids.contains(&"timg0") && !ids.contains(&"timg1"));
}
