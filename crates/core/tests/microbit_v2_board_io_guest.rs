// SPDX-License-Identifier: MIT
//! Guest-driven board proof, distinct from host-MMIO and terminal-spin tests.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::CortexM;
use labwired_core::inspect::InspectOpts;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, DebugControl, Machine};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn machine() -> Machine<CortexM> {
    static NEXT_ARTIFACT: AtomicU64 = AtomicU64::new(0);
    let firmware = root().join("examples/microbit-v2");
    let elf = std::env::temp_dir().join(format!(
        "microbit-board-io-{}-{}.elf",
        std::process::id(),
        NEXT_ARTIFACT.fetch_add(1, Ordering::Relaxed)
    ));
    let status = Command::new("arm-none-eabi-gcc")
        .current_dir(&firmware)
        .args([
            "-mcpu=cortex-m4",
            "-mthumb",
            "-nostdlib",
            "-Wl,-T,board-io.ld",
            "-o",
        ])
        .arg(&elf)
        .arg("board-io.S")
        .status()
        .expect("ARM GCC required for executable board proof");
    assert!(status.success(), "source-built board guest must compile");
    println!(
        "MICROBIT_GUEST_SHA256={:x}",
        Sha256::digest(std::fs::read(&elf).unwrap())
    );
    let system = root().join("configs/systems/microbit-v2.yaml");
    let mut manifest = SystemManifest::from_file(&system).unwrap();
    let chip_path = system.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).unwrap();
    manifest.chip = chip_path.to_str().unwrap().to_string();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let interval = bus.max_safe_tick_interval();
    let (cpu, _) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.config.peripheral_tick_interval = interval;
    machine.bus.config.peripheral_tick_interval = interval;
    machine
        .load_firmware(&labwired_loader::load_elf(&elf).unwrap())
        .unwrap();
    std::fs::remove_file(elf).unwrap();
    machine
}

fn frame(machine: &Machine<CortexM>) -> Vec<u8> {
    let artifact = machine
        .bus
        .display_artifact(
            "led_matrix",
            &InspectOpts {
                include_bytes: true,
                peripheral: None,
            },
        )
        .expect("board has an actual GPIO-driven display");
    assert_eq!(artifact.meta["format"], "gray8");
    artifact.bytes.unwrap()
}

fn diagonal(machine: &Machine<CortexM>) {
    let pixels = frame(machine);
    assert_eq!(pixels.len(), 25);
    for y in 0..5 {
        for x in 0..5 {
            if x == y {
                assert!(
                    pixels[y * 5 + x] > 0,
                    "guest pixel({x},{y}) dark: {pixels:?}"
                );
            } else {
                assert_eq!(
                    pixels[y * 5 + x],
                    0,
                    "guest leaked pixel({x},{y}): {pixels:?}"
                );
            }
        }
    }
}

#[test]
fn guest_scans_all_rows_and_silicon_p1_column_and_reads_buttons() {
    let mut m = machine();
    m.run(Some(2_000_000)).unwrap();
    diagonal(&m);
    assert!(
        m.bus.read_u32(0x2000_0004).unwrap() > 0,
        "guest completed scans"
    );
    assert_eq!(
        m.bus.read_u32(0x2000_0000).unwrap(),
        0,
        "both buttons released"
    );
    for (a, b, expected) in [
        (true, false, 1),
        (false, true, 2),
        (true, true, 3),
        (false, false, 0),
    ] {
        assert!(SystemBus::drive_pin_input(&mut m.bus, "P0.14", !a));
        assert!(SystemBus::drive_pin_input(&mut m.bus, "P0.23", !b));
        let scans = m.bus.read_u32(0x2000_0004).unwrap();
        m.run(Some(50_000)).unwrap();
        assert_eq!(
            m.bus.read_u32(0x2000_0000).unwrap(),
            expected,
            "guest button mask"
        );
        assert!(
            m.bus.read_u32(0x2000_0004).unwrap() > scans,
            "guest kept scanning"
        );
    }
    diagonal(&m);
}

#[test]
#[ignore = "explicit release-mode active-workload performance qualification"]
fn active_display_button_workload_throughput() {
    if cfg!(debug_assertions) {
        panic!("performance qualification requires --release");
    }
    let mut m = machine();
    m.run(Some(8_000_000)).unwrap();
    diagonal(&m);
    let mut samples = Vec::new();
    for index in 0..5 {
        assert!(SystemBus::drive_pin_input(
            &mut m.bus,
            "P0.14",
            index % 2 == 0
        ));
        let before = m.total_cycles;
        let scans = m.bus.read_u32(0x2000_0004).unwrap();
        let start = Instant::now();
        // One modeled second at 64MHz reduces timer/scheduling noise versus
        // the original 4M-step windows. Receipts retain actual cycle counts.
        m.run(Some(64_000_000)).unwrap();
        let wall = start.elapsed().as_secs_f64();
        let cycles = m.total_cycles - before;
        let rtx = cycles as f64 / 64_000_000.0 / wall;
        assert!(cycles > 0 && m.bus.read_u32(0x2000_0004).unwrap() > scans);
        diagonal(&m);
        assert_eq!(
            m.bus.read_u32(0x2000_0000).unwrap(),
            if index % 2 == 0 { 0 } else { 1 }
        );
        samples.push(rtx);
        println!(
            "MICROBIT_ACTIVE_SAMPLE {}",
            serde_json::json!({
                "pass": index + 1, "cycles": cycles, "wallSeconds": wall,
                "rtxAt64MHz": rtx, "pixels": frame(&m),
                "guestButtonMask": m.bus.read_u32(0x2000_0000).unwrap(),
            })
        );
    }
    samples.sort_by(f64::total_cmp);
    println!("MICROBIT_ACTIVE_MEDIAN_RTX={:.6}", samples[2]);
    if std::env::var("LABWIRED_REQUIRE_REALTIME").as_deref() == Ok("1") {
        assert!(
            samples[2] >= 1.0,
            "active-board median below real time: {samples:?}"
        );
    }
}
