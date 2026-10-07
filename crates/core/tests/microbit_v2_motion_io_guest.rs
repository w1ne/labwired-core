// SPDX-License-Identifier: MIT
//! Source-built ARM guest proof: TWIM/EasyDMA, physical motion, display/buttons.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::CortexM;
use labwired_core::inspect::InspectOpts;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, DebugControl, Machine};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};
use std::{path::PathBuf, process::Command, time::Instant};

const STATE: u64 = 0x2000_0100;
const FIRST: ([f64; 3], [f64; 3], [i16; 3], [i16; 3]) = (
    [1.0, -0.5, 0.25],
    [30.0, -15.0, 7.5],
    [16384, -8192, 4096],
    [200, -100, 50],
);
const SECOND: ([f64; 3], [f64; 3], [i16; 3], [i16; 3]) = (
    [-0.25, 0.125, -0.75],
    [-30.0, 0.0, 60.0],
    [-4096, 2048, -12288],
    [-200, 0, 400],
);

fn machine() -> Machine<CortexM> {
    static NEXT_ARTIFACT: AtomicU64 = AtomicU64::new(0);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let elf = std::env::temp_dir().join(format!(
        "microbit-motion-{}-{}.elf",
        std::process::id(),
        NEXT_ARTIFACT.fetch_add(1, Ordering::Relaxed)
    ));
    assert!(Command::new("arm-none-eabi-gcc")
        .current_dir(root.join("examples/microbit-v2"))
        .args([
            "-mcpu=cortex-m4",
            "-mthumb",
            "-nostdlib",
            "-DMICROBIT_MOTION_IO",
            "-Wl,-T,board-io.ld",
            "-o"
        ])
        .arg(&elf)
        .arg("board-io.S")
        .status()
        .expect("ARM GCC required")
        .success());
    let payload = std::fs::read(&elf).unwrap();
    let hash = format!("{:x}", Sha256::digest(&payload));
    println!("MICROBIT_MOTION_GUEST_SHA256={hash}");
    if let Ok(directory) = std::env::var("LABWIRED_GUEST_ARTIFACT_DIR") {
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            PathBuf::from(directory).join(format!("motion-{hash}.elf")),
            &payload,
        )
        .unwrap();
    }
    let system = root.join("configs/systems/microbit-v2.yaml");
    let mut manifest = SystemManifest::from_file(&system).unwrap();
    let chip_path = system.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).unwrap();
    manifest.chip = chip_path.to_str().unwrap().to_string();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let interval = bus.max_safe_tick_interval();
    let (cpu, _) = configure_cortex_m(&mut bus);
    let mut m = Machine::new(cpu, bus);
    m.config.peripheral_tick_interval = interval;
    m.bus.config.peripheral_tick_interval = interval;
    m.load_firmware(&labwired_loader::load_elf(&elf).unwrap())
        .unwrap();
    std::fs::remove_file(elf).unwrap();
    m
}

fn pose(m: &mut Machine<CortexM>, accel: [f64; 3], mag: [f64; 3]) {
    m.set_inputs(&[
        (Some("accelerometer"), "x", accel[0]),
        (Some("accelerometer"), "y", accel[1]),
        (Some("accelerometer"), "z", accel[2]),
        (Some("magnetometer"), "x", mag[0]),
        (Some("magnetometer"), "y", mag[1]),
        (Some("magnetometer"), "z", mag[2]),
    ])
    .unwrap();
}

fn words(m: &mut Machine<CortexM>, offset: u64) -> [i16; 3] {
    std::array::from_fn(|i| m.bus.read_u16(STATE + offset + i as u64 * 2).unwrap() as i16)
}

fn pixels(m: &Machine<CortexM>) -> Vec<u8> {
    let artifact = m
        .bus
        .display_artifact(
            "led_matrix",
            &InspectOpts {
                include_bytes: true,
                peripheral: None,
            },
        )
        .unwrap();
    assert_eq!(artifact.meta["format"], "gray8");
    let pixels = artifact.bytes.unwrap();
    assert_eq!(pixels.len(), 25);
    for (i, pixel) in pixels.iter().enumerate() {
        assert_eq!(*pixel > 0, i / 5 == i % 5, "diagonal {pixels:?}");
    }
    pixels
}

fn proof(m: &mut Machine<CortexM>, accel: [i16; 3], mag: [i16; 3]) {
    assert_eq!(
        m.bus.read_u8(STATE).unwrap(),
        0x33,
        "actual accelerometer WHO_AM_I"
    );
    assert_eq!(
        m.bus.read_u8(STATE + 1).unwrap(),
        0x40,
        "actual magnetometer WHO_AM_I"
    );
    assert_eq!(
        m.bus.read_u32(STATE + 12).unwrap(),
        0,
        "no guest timeout/NACK"
    );
    assert!(m.bus.read_u32(STATE + 4).unwrap() > 0);
    assert!(m.bus.read_u32(STATE + 8).unwrap() > 0);
    assert_eq!(
        m.bus.read_u32(STATE + 0x24).unwrap(),
        0x60001,
        "accelerometer DMA amounts"
    );
    assert_eq!(
        m.bus.read_u32(STATE + 0x28).unwrap(),
        0x60001,
        "magnetometer DMA amounts"
    );
    assert_eq!(words(m, 0x10), accel);
    assert_eq!(words(m, 0x18), mag);
    pixels(m);
}

#[test]
fn guest_reads_changing_motion_through_production_twim_easydma() {
    let mut m = machine();
    pose(&mut m, FIRST.0, FIRST.1);
    m.run(Some(2_000_000)).unwrap();
    proof(&mut m, FIRST.2, FIRST.3);
    for (address, expected) in [(0x40003508, 8), (0x4000350c, 16), (0x40003500, 6)] {
        assert_eq!(m.bus.read_u32(address).unwrap(), expected);
    }
    let counts = [
        m.bus.read_u32(STATE + 4).unwrap(),
        m.bus.read_u32(STATE + 8).unwrap(),
    ];
    pose(&mut m, SECOND.0, SECOND.1);
    assert!(SystemBus::drive_pin_input(&mut m.bus, "P0.14", false));
    assert!(SystemBus::drive_pin_input(&mut m.bus, "P0.23", false));
    m.run(Some(2_000_000)).unwrap();
    proof(&mut m, SECOND.2, SECOND.3);
    assert!(m.bus.read_u32(STATE + 4).unwrap() > counts[0]);
    assert!(m.bus.read_u32(STATE + 8).unwrap() > counts[1]);
    assert_eq!(m.bus.read_u32(0x20000000).unwrap(), 3);
}

#[test]
#[ignore = "explicit release-mode active-motion performance qualification"]
fn active_motion_display_button_workload_throughput() {
    if cfg!(debug_assertions) {
        panic!("requires --release");
    }
    let mut m = machine();
    pose(&mut m, FIRST.0, FIRST.1);
    m.run(Some(8_000_000)).unwrap();
    proof(&mut m, FIRST.2, FIRST.3);
    let mut samples = Vec::new();
    for index in 0..5 {
        let p = if index % 2 == 0 { FIRST } else { SECOND };
        pose(&mut m, p.0, p.1);
        assert!(SystemBus::drive_pin_input(
            &mut m.bus,
            "P0.14",
            index % 2 == 0
        ));
        let counts = [
            m.bus.read_u32(STATE + 4).unwrap(),
            m.bus.read_u32(STATE + 8).unwrap(),
            m.bus.read_u32(0x20000004).unwrap(),
        ];
        let before = m.total_cycles;
        let start = Instant::now();
        m.run(Some(64_000_000)).unwrap();
        let wall = start.elapsed().as_secs_f64();
        let cycles = m.total_cycles - before;
        let rtx = cycles as f64 / 64_000_000.0 / wall;
        proof(&mut m, p.2, p.3);
        let accel_samples = m.bus.read_u32(STATE + 4).unwrap();
        let mag_samples = m.bus.read_u32(STATE + 8).unwrap();
        let scans = m.bus.read_u32(0x20000004).unwrap();
        assert!(accel_samples > counts[0] && mag_samples > counts[1] && scans > counts[2]);
        let buttons = m.bus.read_u32(0x20000000).unwrap();
        assert_eq!(buttons, index % 2);
        samples.push(rtx);
        println!(
            "MICROBIT_MOTION_SAMPLE {}",
            serde_json::json!({
                "pass":index+1,"cycles":cycles,"wallSeconds":wall,"rtxAt64MHz":rtx,
                "pixels":pixels(&m),"guestButtonMask":buttons,"accelInput":p.0,"magInput":p.1,
                "accelRaw":words(&mut m,0x10),"magRaw":words(&mut m,0x18),
                "accelSamples":accel_samples,"magSamples":mag_samples,"scans":scans,
                "error":m.bus.read_u32(STATE+12).unwrap(),
                "accelDma":m.bus.read_u32(STATE+0x24).unwrap(),"magDma":m.bus.read_u32(STATE+0x28).unwrap(),
            })
        );
    }
    samples.sort_by(f64::total_cmp);
    println!("MICROBIT_MOTION_MEDIAN_RTX={:.6}", samples[2]);
    if std::env::var("LABWIRED_REQUIRE_REALTIME").as_deref() == Ok("1") {
        assert!(
            samples[2] >= 1.0,
            "motion median below real time: {samples:?}"
        );
    }
}
