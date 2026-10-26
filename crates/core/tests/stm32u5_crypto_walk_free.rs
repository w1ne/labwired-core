use std::path::PathBuf;
use std::sync::atomic::Ordering;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{bus::SystemBus, Bus};

const RCC_ENR2: u64 = 0x4602_0c8c;
const HASH: u64 = 0x420c_0400;
const SAES: u64 = 0x420c_0c00;
const PKA: u64 = 0x420c_2000;

fn system_bus(relative_system: &str) -> SystemBus {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let system_path = root.join(relative_system);
    let mut manifest = SystemManifest::from_file(&system_path).unwrap();
    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).unwrap();
    manifest.chip = chip_path.to_string_lossy().into_owned();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let _ = configure_cortex_m(&mut bus);
    bus
}

#[test]
fn stm32u5_crypto_registers_preserve_wide_cpu_batching() {
    for system in [
        "configs/systems/nucleo-u575zi.yaml",
        "configs/systems/nucleo-u5a5zj.yaml",
    ] {
        let bus = system_bus(system);
        assert_eq!(
            bus.max_safe_tick_interval(),
            1024,
            "{system} should keep the event-scheduler batch path enabled"
        );
    }
}

#[test]
fn hash_level_irq_tracks_the_register_mask_and_status_on_writes() {
    let mut bus = system_bus("configs/systems/nucleo-u575zi.yaml");
    bus.write_u32(RCC_ENR2, 1 << 17).unwrap();

    // HASH starts with DINIS set. Enabling DINIE should assert its level IRQ
    // immediately; clearing DINIS should deassert it without waiting for a tick.
    bus.write_u32(HASH + 0x20, 1).unwrap();
    assert!(bus.is_nvic_irq_pending(16 + 96), "HASH IRQ should assert");
    assert_ne!(
        bus.nvic.as_ref().unwrap().level_pended[3].load(Ordering::SeqCst) & 1,
        0,
        "HASH pend should be marked as level-driven"
    );
    bus.write_u32(HASH + 0x24, 1).unwrap();
    assert_eq!(bus.read_u32(HASH + 0x24).unwrap() & 1, 0, "DINIS clears");
    assert_eq!(
        bus.nvic.as_ref().unwrap().level_pended[3].load(Ordering::SeqCst) & 1,
        0,
        "HASH deassert should remove the level marker"
    );
    assert!(!bus.is_nvic_irq_pending(16 + 96), "HASH IRQ should clear");
}

#[test]
fn saes_level_irq_tracks_the_register_mask_and_status_on_writes() {
    let mut bus = system_bus("configs/systems/nucleo-u5a5zj.yaml");
    bus.write_u32(RCC_ENR2, 1 << 20).unwrap();
    bus.write_u32(SAES, 1 | (2 << 1)).unwrap(); // AES-128, ECB, enabled

    let key = [
        0x2b, 0x7e, 0x15, 0x16, 0x28, 0xae, 0xd2, 0xa6, 0xab, 0xf7, 0x15, 0x88, 0x09, 0xcf, 0x4f,
        0x3c,
    ];
    let block = [
        0x6b, 0xc1, 0xbe, 0xe2, 0x2e, 0x40, 0x9f, 0x96, 0xe9, 0x3d, 0x7e, 0x11, 0x73, 0x93, 0x17,
        0x2a,
    ];
    for (index, word) in key.chunks_exact(4).enumerate() {
        bus.write_u32(
            SAES + 0x10 + index as u64 * 4,
            u32::from_le_bytes(word.try_into().unwrap()),
        )
        .unwrap();
    }
    for word in block.chunks_exact(4) {
        bus.write_u32(SAES + 8, u32::from_le_bytes(word.try_into().unwrap()))
            .unwrap();
    }

    assert_eq!(
        bus.read_u32(SAES + 4).unwrap() & 1,
        1,
        "SAES CCF should set"
    );
    assert_eq!(
        bus.read_u32(SAES + 0x304).unwrap() & 1,
        1,
        "SAES ISR CCF should set"
    );
    bus.write_u32(SAES + 0x300, 1).unwrap(); // CCF interrupt enable
    assert!(bus.is_nvic_irq_pending(16 + 28), "SAES IRQ should assert");
    bus.write_u32(SAES + 0x308, 1).unwrap(); // clear CCF
    assert_eq!(bus.read_u32(SAES + 0x304).unwrap() & 1, 0, "CCF clears");
    assert!(!bus.is_nvic_irq_pending(16 + 28), "SAES IRQ should clear");
}

#[test]
fn pka_level_irq_tracks_the_register_mask_and_status_on_writes() {
    let mut bus = system_bus("configs/systems/nucleo-u5a5zj.yaml");
    bus.write_u32(RCC_ENR2, (1 << 17) | (1 << 19) | (1 << 20))
        .unwrap();

    // 4^13 mod 497 = 445; PROCENDIE exposes the synchronous completion as a
    // level, which CLRFR must drop immediately.
    bus.write_u32(PKA + 0x400, 4).unwrap();
    bus.write_u32(PKA + 0x408, 9).unwrap();
    bus.write_u32(PKA + 0x0c68, 4).unwrap();
    bus.write_u32(PKA + 0x0e78, 13).unwrap();
    bus.write_u32(PKA + 0x1088, 497).unwrap();
    bus.write_u32(PKA, 1 | (1 << 1)).unwrap();
    assert_eq!(bus.read_u32(PKA + 0x0838).unwrap(), 445);
    bus.write_u32(PKA, 1 | (1 << 17)).unwrap(); // enable PROCENDIE
    assert!(bus.is_nvic_irq_pending(16 + 97), "PKA IRQ should assert");
    bus.write_u32(PKA + 0x08, 1 << 17).unwrap(); // clear PROCENDF
    assert_eq!(
        bus.read_u32(PKA + 4).unwrap() & (1 << 17),
        0,
        "PROCENDF clears"
    );
    assert!(!bus.is_nvic_irq_pending(16 + 97), "PKA IRQ should clear");
}
