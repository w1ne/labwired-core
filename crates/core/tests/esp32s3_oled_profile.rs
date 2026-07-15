//! Reproducible native baseline for the ESP32-S3 OLED workload.
//!
//! This is intentionally a measurement-only test.  It uses the same faithful
//! S3 fast-boot constructor as the WASM path and runs the existing machine
//! interpreter; it does not add timing, clock, or simulator-only execution
//! behaviour.

use labwired_core::boot::esp32s3::{fast_boot, BootOpts};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::XtensaLx7;
use labwired_core::peripherals::components::Ssd1306;
use labwired_core::peripherals::esp32s3::i2c::Esp32s3I2c;
use labwired_core::peripherals::esp32s3::usb_serial_jtag::UsbSerialJtag;
use labwired_core::system::xtensa::{configure_xtensa_esp32s3, Esp32s3Opts};
use labwired_core::{Cpu, DebugControl, Machine};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

const CHUNK: u32 = 1_000_000;
const DEFAULT_MAX_CYCLES: u64 = 200_000_000;
const DEFAULT_SERIAL_MARKER: &str = "S3 OLED painted";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("resolve repository root")
}

fn required_asset(name: &str, env_name: &str, default_relative: &str) -> PathBuf {
    let path = std::env::var_os(env_name)
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join(default_relative));
    assert!(
        path.is_file(),
        "ESP32-S3 OLED profile requires {name} at {}. Set {env_name} to the curated asset path; refusing to substitute the tier-1 self-test fixture.",
        path.display()
    );
    path
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

fn oled_framebuffer(machine: &Machine<XtensaLx7>) -> Vec<u8> {
    let index = machine
        .bus
        .find_peripheral_index_by_name("i2c0")
        .expect("S3 machine exposes i2c0");
    machine.bus.peripherals[index]
        .dev
        .as_any()
        .expect("i2c0 is downcastable")
        .downcast_ref::<Esp32s3I2c>()
        .expect("i2c0 is the ESP32-S3 command-list controller")
        .attached_slaves()
        .iter()
        .filter(|slave| slave.address() == 0x3c)
        .find_map(|slave| slave.as_any().and_then(|any| any.downcast_ref::<Ssd1306>()))
        .expect("SSD1306 is attached at I2C address 0x3C")
        .framebuffer()
        .to_vec()
}

fn build_machine(elf: &[u8]) -> (Machine<XtensaLx7>, Arc<Mutex<Vec<u8>>>) {
    let mut bus = SystemBus::new();
    let wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
    let serial = Arc::new(Mutex::new(Vec::new()));

    for peripheral in &mut bus.peripherals {
        if peripheral.name == "usb_serial_jtag" {
            if let Some(any) = peripheral.dev.as_any_mut() {
                if let Some(jtag) = any.downcast_mut::<UsbSerialJtag>() {
                    jtag.set_sink(Some(serial.clone()), false);
                }
            }
        }
    }
    bus.attach_uart_tx_sink(serial.clone(), false);
    bus.refresh_peripheral_index();

    let mut cpu = wiring.cpu;
    fast_boot(
        elf,
        &mut bus,
        &mut cpu,
        &BootOpts {
            stack_top_fallback: 0x3fcd_fff0,
            icache_backing: Some(wiring.icache_backing),
            dcache_backing: Some(wiring.dcache_backing),
        },
    )
    .expect("fast-boot curated ESP32-S3 OLED ELF");

    (Machine::new(cpu, bus), serial)
}

#[test]
#[ignore = "native S3 workload benchmark; run explicitly with --release --ignored"]
fn esp32s3_oled_native_baseline() {
    let root = repo_root();
    let chip_yaml = root.join("core/configs/chips/esp32s3.yaml");
    let system_yaml = root.join("core/configs/systems/esp32s3-oled-demo.yaml");
    assert!(
        chip_yaml.is_file(),
        "missing S3 chip config: {}",
        chip_yaml.display()
    );
    assert!(
        system_yaml.is_file(),
        "missing S3 OLED system config: {}",
        system_yaml.display()
    );
    let system = std::fs::read_to_string(&system_yaml).expect("read S3 OLED system config");
    assert!(
        system.contains("i2c_address: 0x3C"),
        "S3 OLED config must bind 0x3C"
    );

    let elf_path = required_asset(
        "curated OLED ELF",
        "LABWIRED_ESP32S3_OLED_ELF",
        "packages/playground/public/wasm/demo-esp32s3-oled.elf",
    );
    let elf = std::fs::read(&elf_path).expect("read curated S3 OLED ELF");
    let (mut machine, serial) = build_machine(&elf);
    let max_cycles = std::env::var("LABWIRED_ESP32S3_OLED_MAX_CYCLES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_MAX_CYCLES);
    let marker = std::env::var("LABWIRED_ESP32S3_OLED_SERIAL_MARKER")
        .unwrap_or_else(|_| DEFAULT_SERIAL_MARKER.to_string());

    machine.reset_step_profile();
    let started = Instant::now();
    let mut first_paint = None;
    let mut retired = 0u64;
    while retired < max_cycles {
        let count = CHUNK.min((max_cycles - retired) as u32);
        machine.run(Some(count)).expect("run S3 OLED workload");
        retired += u64::from(count);
        let framebuffer = oled_framebuffer(&machine);
        if first_paint.is_none() && framebuffer.iter().any(|byte| *byte != 0) {
            first_paint = Some((machine.total_cycles, fnv1a(&framebuffer)));
            break;
        }
    }
    let elapsed = started.elapsed();
    let framebuffer = oled_framebuffer(&machine);
    let output = serial.lock().unwrap().clone();
    let output_text = String::from_utf8_lossy(&output);
    let profile = machine.step_profile();

    assert!(
        first_paint.is_some(),
        "S3 OLED did not paint within {max_cycles} cycles (pc=0x{:08x}); serial:\n{output_text}",
        machine.cpu.get_pc()
    );
    assert!(
        output_text.contains(&marker),
        "S3 OLED serial marker {marker:?} was not emitted; serial:\n{output_text}"
    );

    let mips = machine.total_cycles as f64 / elapsed.as_secs_f64() / 1_000_000.0;
    eprintln!(
        "S3_OLED_PROFILE wall_ms={} total_cycles={} mips={:.3} cpu_instructions={} cpu_batches={} peripheral_ticks={} peripheral_ticked_entries={} bus_tick_entries={} legacy_tick_entries={} first_paint={:?} serial_fnv1a={:016x} framebuffer_fnv1a={:016x}",
        elapsed.as_millis(),
        machine.total_cycles,
        mips,
        profile.cpu_instructions,
        profile.cpu_batches,
        profile.peripheral_ticks,
        profile.peripheral_ticked_entries,
        profile.bus_tick_entries,
        profile.legacy_tick_entries,
        first_paint,
        fnv1a(&output),
        fnv1a(&framebuffer),
    );
}
