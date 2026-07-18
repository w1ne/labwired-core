// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// Plan 4 Task 9: e2e test that builds examples/esp32s3-i2c-tmp102 and runs
// it in the simulator, asserting JTAG output + GPIO threshold transitions.

#![cfg(feature = "esp32s3-fixtures")]

use labwired_core::boot::esp32s3::{fast_boot, BootOpts};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::esp32s3::gpio::GpioObserver;
use labwired_core::peripherals::esp32s3::usb_serial_jtag::UsbSerialJtag;
use labwired_core::system::xtensa::{configure_xtensa_esp32s3, Esp32s3Opts};
use labwired_core::{Cpu, SimulationError};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};

#[derive(Debug, Default)]
struct RecordingObserver {
    events: Mutex<Vec<(u8, bool, bool, u64)>>,
}

impl GpioObserver for RecordingObserver {
    fn on_pin_change(&self, pin: u8, from: bool, to: bool, sim_cycle: u64) {
        self.events.lock().unwrap().push((pin, from, to, sim_cycle));
    }
}

fn firmware_path() -> PathBuf {
    PathBuf::from(
        "../../examples/esp32s3-i2c-tmp102/target/xtensa-esp32s3-none-elf/release/esp32s3-i2c-tmp102",
    )
}

fn ensure_firmware_built() -> PathBuf {
    let elf = firmware_path();
    let src = PathBuf::from("../../examples/esp32s3-i2c-tmp102/src/main.rs");
    if elf.exists() {
        if let (Ok(elf_meta), Ok(src_meta)) = (std::fs::metadata(&elf), std::fs::metadata(&src)) {
            if elf_meta.modified().unwrap() >= src_meta.modified().unwrap() {
                return elf;
            }
        }
    }
    let status = Command::new("cargo")
        .args(["+esp", "build", "--release"])
        .current_dir("../../examples/esp32s3-i2c-tmp102")
        .status()
        .expect("cargo +esp build (is the ESP toolchain installed and ~/export-esp.sh sourced?)");
    assert!(status.success(), "esp32s3-i2c-tmp102 build failed");
    assert!(elf.exists(), "ELF not found at {elf:?} after build");
    elf
}

#[test]
fn i2c_tmp102_firmware_runs_and_prints_temperature() {
    let elf_path = ensure_firmware_built();
    let elf_bytes = std::fs::read(&elf_path).expect("read firmware ELF");

    let mut bus = SystemBus::new();
    let wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());

    // Wire the TMP102 from a board manifest through the generic factory — the
    // same path app/CLI use — instead of relying on a hardcoded builder attach.
    let manifest: labwired_config::SystemManifest = serde_yaml::from_str(
        r#"
name: esp32s3-tmp102
chip: esp32s3
external_devices:
  - id: tmp102
    type: tmp102
    connection: i2c0
"#,
    )
    .expect("parse tmp102 manifest");
    labwired_core::system::xtensa::attach_esp32_external_devices(&mut bus, &manifest)
        .expect("attach TMP102 from manifest");
    bus.refresh_peripheral_index();

    let obs = Arc::new(RecordingObserver::default());
    wiring.add_gpio_observer(&mut bus, obs.clone());

    // Sink JTAG output for assertions.
    let jtag = Arc::new(Mutex::new(Vec::<u8>::new()));
    for p in bus.peripherals.iter_mut() {
        if let Some(any) = p.dev.as_any_mut() {
            if let Some(uj) = any.downcast_mut::<UsbSerialJtag>() {
                uj.set_sink(Some(jtag.clone()), false);
            }
        }
    }

    let icache_backing = wiring.icache_backing.clone();
    let dcache_backing = wiring.dcache_backing.clone();
    let mut cpu = wiring.cpu;

    fast_boot(
        &elf_bytes,
        &mut bus,
        &mut cpu,
        &BootOpts {
            stack_top_fallback: 0x3FCD_FFF0,
            icache_backing: Some(icache_backing),
            dcache_backing: Some(dcache_backing),
        },
    )
    .expect("fast_boot");

    // Run for up to ~14 simulated seconds at 80 MHz = 1.12 G steps. Each
    // SYSTIMER tick fires once per simulated second. The TMP102 model starts
    // at 25 °C and drifts +0.5 °C per read, so reaching the firmware's 30 °C
    // threshold (and seeing GPIO2 toggle) needs at least 11 reads.
    const MAX_STEPS: u64 = 1_120_000_000;
    let observers: Vec<Arc<dyn labwired_core::SimulationObserver>> = Vec::new();
    let cfg = labwired_core::SimulationConfig::default();

    for _ in 0..MAX_STEPS {
        match cpu.step(&mut bus, &observers, &cfg) {
            Ok(()) => {}
            Err(SimulationError::BreakpointHit(_)) => break,
            Err(e) => panic!("simulator error at pc=0x{:08x}: {e}", cpu.get_pc()),
        }
        let _ = bus.tick_peripherals_with_costs();

        // Early-out once we have ≥4 complete "T = " lines AND have seen the
        // GPIO2 0→1 transition that the firmware drives once temp exceeds the
        // 30 °C threshold. The temp_lines>=12 cap guards against runaway
        // execution if GPIO routing breaks.
        let bytes = jtag.lock().unwrap();
        let text = String::from_utf8_lossy(&bytes).to_string();
        let temp_lines: usize = text.lines().filter(|l| l.starts_with("T = ")).count();
        let gpio2_rose = obs
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|&(p, f, t, _)| p == 2 && !f && t);
        if temp_lines >= 5 && gpio2_rose {
            break;
        }
        if temp_lines >= 14 {
            break;
        }
    }

    let bytes = jtag.lock().unwrap();
    let text = String::from_utf8_lossy(&bytes).to_string();
    let temp_lines: Vec<&str> = text.lines().filter(|l| l.starts_with("T = ")).collect();
    assert!(
        temp_lines.len() >= 4,
        "expected ≥ 4 'T = ...' JTAG lines; got {}: {text:?}",
        temp_lines.len(),
    );

    // Parse temperatures and verify they're roughly drifting upward
    // (allow one wraparound at 35 °C → 20 °C per the TMP102 model).
    let temps: Vec<f32> = temp_lines
        .iter()
        .filter_map(|l| {
            l.strip_prefix("T = ")
                .and_then(|rest| rest.split(' ').next())
                .and_then(|n| n.parse::<f32>().ok())
        })
        .collect();
    assert!(
        temps.len() >= 4,
        "could not parse temperatures: {temp_lines:?}"
    );
    for window in temps.windows(2) {
        if window[1] >= window[0] {
            // monotonic up — OK
        } else {
            assert!(
                window[1] < 21.0 && window[0] > 34.0,
                "non-monotonic without wrap: {} → {}",
                window[0],
                window[1],
            );
        }
    }

    // GPIO2 must have transitioned 0→1 once temperature crossed 30 °C.
    let events = obs.events.lock().unwrap();
    let has_rising = events
        .iter()
        .any(|&(pin, from, to, _)| pin == 2 && !from && to);
    assert!(
        has_rising,
        "expected GPIO2 to rise (0→1) once temp > 30 °C; events: {events:?}"
    );
}
