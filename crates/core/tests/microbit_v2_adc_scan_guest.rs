// SPDX-License-Identifier: MIT
//! Real ARM guest proof of held-input scan triggering and latched EasyDMA.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, DebugControl, Machine};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, process::Command};

const STATE: u64 = 0x2000_0100;
const BUFFER: u64 = 0x2000_0200;

fn inputs(m: &mut Machine<CortexM>, ain3: u16, ain2: u16) {
    let index = m.bus.find_peripheral_index_by_name("saadc").unwrap();
    let adc = &mut m.bus.peripherals[index].dev;
    assert!(adc.set_adc_channel_input(3, ain3));
    assert!(adc.set_adc_channel_input(2, ain2));
}

fn stage(m: &mut Machine<CortexM>, expected: u32) {
    for _ in 0..100 {
        m.run(Some(1_000)).unwrap();
        assert_eq!(m.bus.read_u32(STATE + 0x18).unwrap(), 0, "guest timeout");
        if m.bus.read_u32(STATE + 4).unwrap() == expected {
            return;
        }
    }
    panic!("guest did not reach stage {expected}");
}

#[test]
fn guest_scans_once_per_trigger_and_latches_next_buffer_at_start() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let elf = std::env::temp_dir().join(format!("microbit-adc-scan-{}.elf", std::process::id()));
    assert!(Command::new("arm-none-eabi-gcc")
        .current_dir(root.join("examples/microbit-v2"))
        .args([
            "-mcpu=cortex-m4",
            "-mthumb",
            "-nostdlib",
            "-Wl,-T,board-io.ld",
            "-o"
        ])
        .arg(&elf)
        .arg("adc-scan.S")
        .status()
        .expect("ARM GCC required")
        .success());
    let payload = std::fs::read(&elf).unwrap();
    let hash = format!("{:x}", Sha256::digest(&payload));
    println!("MICROBIT_ADC_SCAN_GUEST_SHA256={hash}");
    if let Ok(directory) = std::env::var("LABWIRED_GUEST_ARTIFACT_DIR") {
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            PathBuf::from(directory).join(format!("adc-scan-{hash}.elf")),
            &payload,
        )
        .unwrap();
    }
    let path = root.join("configs/systems/microbit-v2.yaml");
    let mut manifest = SystemManifest::from_file(&path).unwrap();
    let chip_path = path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).unwrap();
    manifest.chip = chip_path.to_str().unwrap().to_owned();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let interval = bus.max_safe_tick_interval();
    let (cpu, _) = configure_cortex_m(&mut bus);
    let mut m = Machine::new(cpu, bus);
    m.config.peripheral_tick_interval = interval;
    m.bus.config.peripheral_tick_interval = interval;
    m.load_firmware(&labwired_loader::load_elf(&elf).unwrap())
        .unwrap();
    std::fs::remove_file(elf).unwrap();
    for address in [BUFFER + 4, BUFFER + 8, BUFFER + 0x20, BUFFER + 0x24] {
        m.bus.write_u32(address, 0x55aa_55aa).unwrap();
    }
    inputs(&mut m, 1350, 900);
    stage(&mut m, 1);
    assert_eq!(m.bus.read_u32(STATE + 8).unwrap(), 2);
    assert_eq!(
        m.bus.read_u32(STATE + 0x10).unwrap(),
        0,
        "not full after one scan"
    );
    assert_eq!(m.bus.read_u16(BUFFER).unwrap(), 1536);
    assert_eq!(m.bus.read_u16(BUFFER + 2).unwrap(), 1024);
    assert_eq!(m.bus.read_u32(BUFFER + 4).unwrap(), 0x55aa_55aa);
    assert_eq!(m.bus.read_u32(BUFFER + 0x20).unwrap(), 0x55aa_55aa);
    inputs(&mut m, 2700, 1800);
    m.bus.write_u32(STATE, 2).unwrap();
    stage(&mut m, 3);
    for (offset, expected) in [
        (0, 1536),
        (2, 1024),
        (4, 3072),
        (6, 2048),
        (0x20, 3072),
        (0x22, 2048),
    ] {
        assert_eq!(m.bus.read_u16(BUFFER + offset).unwrap(), expected);
    }
    for (offset, expected) in [(0xc, 4), (0x14, 1), (0x1c, 0), (0x20, 2), (0x24, 1)] {
        assert_eq!(m.bus.read_u32(STATE + offset).unwrap(), expected);
    }
    for address in [BUFFER + 8, BUFFER + 0x24] {
        assert_eq!(m.bus.read_u32(address).unwrap(), 0x55aa_55aa, "DMA guard");
    }
}
