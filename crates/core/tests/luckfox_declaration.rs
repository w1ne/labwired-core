//! The declared Mini A must not acquire guessed Cortex-M or RV1103 devices.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::system::arch_policy::{machine_family, MachineFamily};
use std::path::PathBuf;

#[test]
fn mini_a_declares_vendor_memory_without_inventing_storage_or_controllers() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let chip = ChipDescriptor::from_file(root.join("configs/chips/rv1103.yaml")).unwrap();
    assert_eq!(machine_family(&chip).unwrap(), MachineFamily::CortexA);
    assert_eq!(
        chip.flash.size, 0,
        "Mini A boots external MicroSD, not on-chip flash"
    );
    assert_eq!((chip.ram.base, chip.ram.size), (0, 64 * 1024 * 1024));
    let sram = chip
        .memory_regions
        .iter()
        .find(|r| r.name == "sram")
        .unwrap();
    assert_eq!((sram.base, sram.size), (0xff6c0000, 256 * 1024));
    assert!(
        chip.peripherals.is_empty(),
        "no borrowed Cortex-M controllers"
    );
    for path in [
        "configs/systems/luckfox-pico-mini-a.yaml",
        "examples/luckfox-pico-mini-a/system.yaml",
    ] {
        let path = root.join(path);
        let system = SystemManifest::from_file(&path).unwrap();
        let declared =
            ChipDescriptor::from_file(path.parent().unwrap().join(&system.chip)).unwrap();
        assert_eq!(declared.name, chip.name);
        assert!(
            system.external_devices.is_empty(),
            "wiring plans are not modeled parts"
        );
        assert!(system.board_io.is_empty());
    }
}
