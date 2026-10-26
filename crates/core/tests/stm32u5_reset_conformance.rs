use std::path::PathBuf;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;

// Coverage source: configs/chips/stm32u5a5zj.yaml.

#[test]
fn u5a5_chip_and_crypto_peripheral_resets_match_the_selected_profile() {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip_path = crate_root.join("../../configs/chips/stm32u5a5zj.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).unwrap();
    assert_eq!(chip.flash.size, 4 * 1024 * 1024);
    assert_eq!(chip.ram.size, 768 * 1024);
    assert_eq!(chip.memory_regions.len(), 5);
    let peripherals: Vec<_> = chip.peripherals.iter().map(|p| p.id.as_str()).collect();
    for id in ["hash", "saes", "pka", "gpdma1", "icache", "fdcan1", "rtc"] {
        assert!(peripherals.contains(&id), "U5A5 map is missing {id}");
    }

    let system_path = crate_root.join("../../configs/systems/nucleo-u5a5zj.yaml");
    let mut manifest = SystemManifest::from_file(&system_path).unwrap();
    let resolved_chip = system_path.parent().unwrap().join(&manifest.chip);
    let resolved = ChipDescriptor::from_file(&resolved_chip).unwrap();
    manifest.chip = resolved_chip.to_string_lossy().into_owned();
    let mut bus = SystemBus::from_config(&resolved, &manifest).unwrap();
    bus.write_u32(0x4602_0c00 + 0x8c, (1 << 17) | (1 << 19) | (1 << 20))
        .unwrap();
    assert_eq!(bus.read_u32(0x420c_0400 + 0x24).unwrap() & 1, 1); // HASH DINIS
    assert_eq!(bus.read_u32(0x420c_0c00 + 4).unwrap(), 0); // SAES idle
    assert_eq!(bus.read_u32(0x420c_2000 + 4).unwrap() & 1, 1); // PKA INITOK
}
