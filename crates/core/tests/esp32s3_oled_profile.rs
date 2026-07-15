//! Reproducible native baseline for the ESP32-S3 OLED workload.
//!
//! This is intentionally a measurement-only test.  It uses the same faithful
//! S3 fast-boot constructor as the WASM path and runs the existing machine
//! interpreter; it does not add timing, clock, or simulator-only execution
//! behaviour.

use labwired_config::{Arch, ChipDescriptor, SystemManifest};
use labwired_core::boot::esp32s3::{fast_boot, BootOpts};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::XtensaLx7;
use labwired_core::peripherals::components::Ssd1306;
use labwired_core::peripherals::esp32s3::i2c::Esp32s3I2c;
use labwired_core::peripherals::esp32s3::usb_serial_jtag::UsbSerialJtag;
use labwired_core::system::xtensa::{configure_xtensa_esp32s3, Esp32s3Opts};
use labwired_core::{DebugControl, Machine};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

const CHUNK: u32 = 1_000_000;
const DEFAULT_MAX_CYCLES: u64 = 200_000_000;
const DEFAULT_SERIAL_MARKER: &str = "S3 OLED painted";
const GOLDEN_FIRST_PAINT_CYCLES: u64 = 1_139_600;
const GOLDEN_FIRST_PAINT_FNV1A: u64 = 0x41ac26506ebe964b;
const GOLDEN_SERIAL_FNV1A: u64 = 0xaf2df535cf6fd7e4;
const GOLDEN_FRAMEBUFFER_FNV1A: u64 = 0xc4eb9ef771b3ded8;
const GOLDEN_COMPLETION_CYCLES: u64 = 2_139_600;
// `tick_profile_entry_counts()` counts the active legacy index vector.  The
// faithful-fast-boot S3 wiring registers 37 such entries; this is a golden
// source-level invariant, not a performance range.
const GOLDEN_LEGACY_TICK_MULTIPLIER: u64 = 37;
const GOLDEN_LEGACY_TICK_ENTRIES: u64 = 79_165_200;

// Complete assembled-bus mapping for the OLED workload. The role labels make
// review failures actionable: entries marked "retained" have observable
// per-step behavior, while the removed inert classes must never reappear.
const EXPECTED_LEGACY_S3_OLED_ROLES: &[(&str, &str)] = &[
    ("intmatrix", "retained: interrupt routing"),
    ("crosscore_ipi", "retained: SMP doorbell"),
    ("core1_control", "retained: secondary-core release"),
    ("extmem", "retained: external-memory timing"),
    ("system_regs", "retained: S3 system register behavior"),
    ("system_regs_hi", "retained: S3 system register behavior"),
    ("rtc_cntl", "retained: RTC calibration/timing"),
    ("gpio", "retained: GPIO observer cycle timestamps"),
    ("sens_s3", "retained: S3 sensor model"),
    ("rng", "retained: RNG model"),
    ("sha", "retained: SHA accelerator model"),
    ("pcnt", "retained: pulse-counter timing"),
    ("ledc", "retained: PWM timing"),
    ("timg0_s3", "retained: timer-group timing"),
    ("timg1_s3", "retained: timer-group timing"),
    ("rmt_s3", "retained: waveform timing"),
    ("spi2_s3", "retained: SPI timing"),
    ("spi3_s3", "retained: SPI timing"),
    ("sar_adc_s3", "retained: ADC behavior"),
    ("gdma", "retained: DMA behavior"),
    ("i2s0_s3", "retained: I2S timing"),
    ("i2s1_s3", "retained: I2S timing"),
    ("twai", "retained: TWAI timing"),
    ("aes", "retained: AES accelerator model"),
    ("rsa", "retained: RSA accelerator model"),
    ("hmac", "retained: HMAC accelerator model"),
    ("ds", "retained: DS accelerator model"),
    ("mcpwm0", "retained: motor-PWM timing"),
    ("mcpwm1", "retained: motor-PWM timing"),
    ("sdmmc", "retained: SDMMC timing"),
    ("lcd_cam", "retained: LCD/CAM timing"),
    ("usb_otg", "retained: USB OTG behavior"),
    ("i2c1", "retained: I2C timing"),
    ("uart0_s3", "retained: UART timing"),
    ("uart1_s3", "retained: UART timing"),
    ("uart2_s3", "retained: UART timing"),
    ("i2c0", "retained: OLED I2C level IRQ timing"),
];

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

/// This is the faithful-fast-boot baseline used by the WASM constructor.  It
/// loads the real S3 ROM images through `Esp32s3Opts` and then uses the
/// existing `fast_boot` seam, which skips replaying the ROM reset/bootloader.
/// It must not be described as a full hardware-boot benchmark.
///
/// Full ROM boot is a separate benchmark boundary: it must use the
/// `boot::esp32s3_rom` path and its own fixture/trace golden values.  Keeping
/// that boundary separate prevents a fast-boot result from being silently
/// presented as ROM-boot fidelity.
fn build_machine(
    chip: &ChipDescriptor,
    manifest: &SystemManifest,
    elf: &[u8],
) -> (Machine<XtensaLx7>, Arc<Mutex<Vec<u8>>>) {
    assert_eq!(chip.name, "esp32s3");
    assert_eq!(chip.arch, Arch::Xtensa);
    assert!(
        chip.peripherals
            .iter()
            .any(|peripheral| peripheral.id == "i2c0"),
        "S3 chip descriptor must declare i2c0"
    );
    let oled = manifest
        .external_devices
        .iter()
        .find(|device| device.id == "oled")
        .expect("S3 OLED manifest must declare external device 'oled'");
    assert_eq!(oled.r#type, "oled-ssd1306");
    assert_eq!(oled.connection, "i2c0");
    assert_eq!(
        oled.config
            .get("i2c_address")
            .and_then(|value| value.as_u64()),
        Some(0x3c)
    );
    let oled_io = manifest
        .board_io
        .iter()
        .find(|binding| binding.id == "oled")
        .expect("S3 OLED manifest must declare board_io binding 'oled'");
    assert_eq!(oled_io.peripheral, "i2c0");
    assert_eq!(oled_io.i2c_address, Some(0x3c));
    assert_eq!(oled_io.device_type.as_deref(), Some("oled-ssd1306"));

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
    let chip = ChipDescriptor::from_file(&chip_yaml).expect("parse S3 chip descriptor");
    let manifest = SystemManifest::from_file(&system_yaml).expect("parse S3 OLED manifest");

    let elf_path = required_asset(
        "curated OLED ELF",
        "LABWIRED_ESP32S3_OLED_ELF",
        "packages/playground/public/wasm/demo-esp32s3-oled.elf",
    );
    let elf = std::fs::read(&elf_path).expect("read curated S3 OLED ELF");
    let (mut machine, serial) = build_machine(&chip, &manifest, &elf);
    let legacy_entries = machine.bus.legacy_tick_entry_descriptors();
    let mut actual_names: Vec<&str> = legacy_entries
        .iter()
        .map(|(name, _, _)| name.as_str())
        .collect();
    let mut expected_names: Vec<&str> = EXPECTED_LEGACY_S3_OLED_ROLES
        .iter()
        .map(|(name, _)| *name)
        .collect();
    actual_names.sort_unstable();
    expected_names.sort_unstable();
    assert_eq!(
        actual_names, expected_names,
        "complete S3 OLED legacy mapping changed; review each named role"
    );
    assert!(
        legacy_entries.iter().any(|(name, _, _)| name == "gpio"),
        "GPIO remains on the legacy path because observer timestamps are tick-driven"
    );
    assert!(
        legacy_entries.iter().any(|(name, _, _)| name == "i2c0"),
        "I2C0 remains on the legacy path because its level IRQ is tick-driven"
    );
    assert!(
        legacy_entries.iter().all(|(name, _, _)| name != "systimer"),
        "scheduler-owned SYSTIMER must not enter the legacy walk"
    );
    assert!(
        legacy_entries
            .iter()
            .all(|(name, _, _)| name != "usb_serial_jtag"),
        "polling USB serial sink must remain inert for the legacy walk"
    );
    for removed in [
        "iram",
        "dram",
        "rtc_slow",
        "rtc_fast",
        "rom",
        "drom",
        "system",
        "efuse",
        "low_mmio",
        "mmio_rest",
        "io_mux",
    ] {
        assert!(
            legacy_entries.iter().all(|(name, _, _)| name != removed),
            "inert {removed} must remain removed from the legacy walk"
        );
    }
    eprintln!("S3_OLED_LEGACY_ENTRIES {:?}", legacy_entries);
    let max_cycles = std::env::var("LABWIRED_ESP32S3_OLED_MAX_CYCLES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_MAX_CYCLES);
    let marker = std::env::var("LABWIRED_ESP32S3_OLED_SERIAL_MARKER")
        .unwrap_or_else(|_| DEFAULT_SERIAL_MARKER.to_string());

    machine.reset_step_profile();
    let started = Instant::now();
    // The first coarse checkpoint is a known blank state for this curated
    // workload.  After it, inspect the framebuffer after every retired
    // instruction.  This keeps one simulation pass while avoiding the
    // 1M-instruction timestamp quantisation of the ordinary profile chunks.
    machine.run(Some(CHUNK)).expect("run S3 OLED warmup");
    let mut retired = u64::from(CHUNK);
    assert!(
        oled_framebuffer(&machine).iter().all(|byte| *byte == 0),
        "curated S3 OLED painted during the coarse warmup; exact boundary must be re-baselined"
    );
    let first_paint = loop {
        assert!(
            retired < max_cycles,
            "S3 OLED did not paint within {max_cycles} cycles"
        );
        machine.run(Some(1)).expect("run S3 OLED instruction");
        retired += 1;
        let framebuffer = oled_framebuffer(&machine);
        if framebuffer.iter().any(|byte| *byte != 0) {
            break (machine.total_cycles, fnv1a(&framebuffer));
        }
    };
    while !String::from_utf8_lossy(&serial.lock().unwrap()).contains(&marker) {
        assert!(
            retired < max_cycles,
            "S3 OLED serial marker was not emitted"
        );
        let remaining = max_cycles - retired;
        // Machine::run accepts a u32 chunk. Clamp the u64 budget first so a
        // valid max_cycles above u32::MAX cannot panic or wrap during narrowing.
        let count = CHUNK.min(remaining.min(u64::from(u32::MAX)) as u32);
        machine.run(Some(count)).expect("run S3 OLED completion");
        retired += u64::from(count);
    }
    let elapsed = started.elapsed();
    let framebuffer = oled_framebuffer(&machine);
    let output = serial.lock().unwrap().clone();
    let output_text = String::from_utf8_lossy(&output);
    let profile = machine.step_profile();

    assert!(
        output_text.contains(&marker),
        "S3 OLED serial marker {marker:?} was not emitted; serial:\n{output_text}"
    );

    let (first_paint_cycles, first_paint_digest) = first_paint;
    assert_eq!(
        first_paint_cycles, GOLDEN_FIRST_PAINT_CYCLES,
        "first-paint cycle drifted from the faithful-fast-boot baseline"
    );
    assert_eq!(
        first_paint_digest, GOLDEN_FIRST_PAINT_FNV1A,
        "first-paint framebuffer digest drifted"
    );
    assert_eq!(fnv1a(&output), GOLDEN_SERIAL_FNV1A, "serial digest drifted");
    assert_eq!(
        fnv1a(&framebuffer),
        GOLDEN_FRAMEBUFFER_FNV1A,
        "final framebuffer digest drifted"
    );
    assert_eq!(
        machine.total_cycles, GOLDEN_COMPLETION_CYCLES,
        "completion cycle drifted from the faithful-fast-boot baseline"
    );

    // These counters are deterministic for this fixed warmup/completion,
    // interval-1 baseline. They are intentionally golden: a scheduler or
    // interpreter change must update this benchmark and its observable-output
    // review together, rather than silently changing the measured workload.
    assert_eq!(profile.cpu_instructions, machine.total_cycles);
    assert_eq!(profile.cpu_batches, profile.cpu_instructions);
    assert_eq!(profile.peripheral_ticks, machine.total_cycles);
    assert_eq!(profile.peripheral_ticked_entries, 0);
    assert_eq!(profile.bus_tick_entries, 0);
    assert_eq!(
        profile.legacy_tick_entries,
        machine.total_cycles * GOLDEN_LEGACY_TICK_MULTIPLIER
    );
    assert_eq!(
        profile.legacy_tick_entries, GOLDEN_LEGACY_TICK_ENTRIES,
        "legacy tick-entry golden drifted"
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
