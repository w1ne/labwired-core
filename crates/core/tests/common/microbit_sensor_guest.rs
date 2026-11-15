// SPDX-License-Identifier: MIT
//! Source-built micro:bit DRDY guest support. No prebuilt firmware or skip path.
#![allow(dead_code)]

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, DebugControl, Machine};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

pub type Guest = Machine<CortexM>;
pub const RAM: u64 = 0x2000_0100;

pub fn profiles() -> &'static [u32] {
    if labwired_core::cycle_clock::event_scheduler_enabled() {
        &[1, 512]
    } else {
        &[1]
    }
}

/// Build a pinned source fixture, then construct a fresh machine. A missing
/// compiler fails the test; CI must install gcc-arm-none-eabi.
pub fn machine(source: &str, interval: u32, configure: impl FnOnce(&mut SystemManifest)) -> Guest {
    let scheduler = labwired_core::cycle_clock::event_scheduler_enabled();
    if std::env::var("LABWIRED_REQUIRE_SENSOR_FEATURE_OFF").as_deref() == Ok("1") {
        assert!(
            !scheduler,
            "feature-off guest must not enable event-scheduler"
        );
        assert_eq!(
            interval, 1,
            "feature-off guest must use the per-cycle profile"
        );
    }
    assert!(
        scheduler || interval == 1,
        "the feature-off walk requires interval 1"
    );
    println!("MICROBIT_DRDY_GUEST_PROFILE source={source} event_scheduler={scheduler} interval={interval}");

    static NEXT: AtomicU64 = AtomicU64::new(0);
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let stem = source.strip_suffix(".S").expect("assembly fixture");
    let elf = std::env::temp_dir().join(format!(
        "microbit-{stem}-{}-{interval}-{sequence}.elf",
        std::process::id()
    ));
    let output = Command::new("arm-none-eabi-gcc")
        .current_dir(root.join("crates/core/tests/fixtures/microbit-drdy"))
        .args(["-mcpu=cortex-m4", "-mthumb", "-nostdlib", "-Wl,-T,guest.ld", "-o"])
        .arg(&elf)
        .arg(source)
        .output()
        .expect("arm-none-eabi-gcc is required; install gcc-arm-none-eabi for source-built guest acceptance");
    assert!(
        output.status.success(),
        "{source} must compile from source: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let bytes = std::fs::read(&elf).expect("compiled guest ELF");
    let hash = format!("{:x}", Sha256::digest(&bytes));
    println!("MICROBIT_DRDY_GUEST_SHA256={hash};source={source};interval={interval}");
    if let Ok(directory) = std::env::var("LABWIRED_GUEST_ARTIFACT_DIR") {
        std::fs::create_dir_all(&directory).expect("guest artifact directory");
        std::fs::write(
            PathBuf::from(directory).join(format!("{stem}-{hash}-{interval}-{sequence}.elf")),
            &bytes,
        )
        .expect("retain guest ELF");
    }
    let firmware = labwired_loader::load_elf(&elf).expect("load guest ELF");
    std::fs::remove_file(&elf).expect("remove temporary guest ELF");

    let system = root.join("configs/systems/microbit-v2.yaml");
    let mut manifest = SystemManifest::from_file(&system).expect("micro:bit board manifest");
    let chip_path = system.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("nRF52833 chip descriptor");
    manifest.chip = chip_path.to_string_lossy().into_owned();
    configure(&mut manifest);
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("guest board construction");
    assert_eq!(bus.cpu_hz, 64_000_000, "guest timing assumes 64 MHz");
    let (cpu, _) = configure_cortex_m(&mut bus);
    let mut guest = Machine::new(cpu, bus);
    guest.config.peripheral_tick_interval = interval;
    guest.bus.config.peripheral_tick_interval = interval;
    guest
        .load_firmware(&firmware)
        .expect("install guest firmware");
    guest
}

pub fn observed(guest: &mut Guest, level: u32, falls: u32) {
    assert_eq!(guest.bus.read_u32(RAM).unwrap(), level, "guest GPIO P0.25");
    assert_eq!(
        guest.bus.read_u32(RAM + 8).unwrap(),
        falls,
        "guest falling events"
    );
    assert!(
        guest.bus.read_u32(RAM + 4).unwrap() > 0,
        "guest sampled the pad"
    );
    assert_eq!(guest.bus.read_u32(RAM + 12).unwrap(), 0, "guest bus errors");
}

/// The host supplies firmware intent in RAM. Firmware owns every TWIM write
/// and read. Wait for completion and a subsequent pad sample, with a finite
/// 1-ms simulated-cycle budget that cannot mask the 10-ms sensor conversion.
pub fn command(guest: &mut Guest, code: u32) {
    guest.bus.write_u32(RAM + 0x14, 0).unwrap();
    guest.bus.write_u32(RAM + 0x10, code).unwrap();
    let start = guest.total_cycles;
    loop {
        assert!(
            guest.total_cycles - start < 64_000,
            "guest command {code} timed out"
        );
        guest.run(Some(32)).unwrap();
        if guest.bus.read_u32(RAM + 0x14).unwrap() == code
            && guest.bus.read_u32(RAM + 0x10).unwrap() == 0
        {
            break;
        }
    }
    let samples = guest.bus.read_u32(RAM + 4).unwrap();
    // Pin drives drain on one peripheral tick and Nordic input changes are
    // observed on the next. Leave two tick intervals plus one guest loop,
    // without allowing the following 10-ms conversion to hide an ack release.
    let settle_until =
        guest.total_cycles + 2 * u64::from(guest.config.peripheral_tick_interval.max(1)) + 256;
    while guest.total_cycles < settle_until {
        assert!(
            guest.total_cycles - start < 64_000,
            "guest did not sample after command {code}"
        );
        guest.run(Some(32)).unwrap();
    }
    assert!(
        guest.bus.read_u32(RAM + 4).unwrap() > samples,
        "fresh sample after command {code}"
    );
    assert_eq!(
        guest.bus.read_u32(RAM + 12).unwrap(),
        0,
        "guest command {code} bus errors"
    );
}
