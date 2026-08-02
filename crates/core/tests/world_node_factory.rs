//! Tests for the shared world-node factory (`system::node`).
//!
//! The factory is what lets a world hold nodes of any modelled architecture.
//! These tests cover the parts that are easy to get subtly wrong: which boot
//! path a firmware file selects, and whether a node's image is genuinely its
//! own rather than a process-global.

use labwired_core::bus::SystemBus;
use labwired_core::system::node::NodeFirmware;
use labwired_core::system::xtensa::{configure_xtensa_esp32s3, Esp32s3Opts};

/// The boot path follows from the firmware file itself, so a node declares one
/// `firmware:` path and does not also have to declare how to boot it.
#[test]
fn firmware_kind_is_detected_from_elf_magic() {
    assert!(matches!(
        NodeFirmware::from_bytes(b"\x7fELF\x01\x01\x01\0rest".to_vec()),
        NodeFirmware::Elf(_)
    ));
    // A flash image starts with the ESP image magic (0xE9), not ELF magic.
    assert!(matches!(
        NodeFirmware::from_bytes(vec![0xE9, 0x04, 0x02, 0x20]),
        NodeFirmware::FlashImage(_)
    ));
    // Anything too short to carry ELF magic must not be mistaken for an ELF.
    assert!(matches!(
        NodeFirmware::from_bytes(vec![0x7f, b'E']),
        NodeFirmware::FlashImage(_)
    ));
}

/// An ESP32-S3's flash image is per-node state.
///
/// `configure_xtensa_esp32s3` used to read `LABWIRED_ESP32S3_FLASH`, a
/// process-global that can only ever name one image — so a world with two S3
/// nodes could not give them different firmware. This proves each node's bytes
/// reach its own flash backing.
#[test]
fn esp32s3_nodes_take_their_own_flash_image() {
    let backing_for = |marker: u8| {
        let mut image = vec![0u8; 4096];
        image[0] = 0xE9;
        image[64] = marker;
        let mut bus = SystemBus::new();
        let wiring = configure_xtensa_esp32s3(
            &mut bus,
            &Esp32s3Opts {
                // real_reset_boot aliases both cache windows onto the one
                // physical flash backing, which is the buffer under test.
                real_reset_boot: true,
                flash_image: Some(image),
                ..Esp32s3Opts::default()
            },
        );
        let bytes = wiring.icache_backing.lock().unwrap();
        (bytes[0], bytes[64])
    };

    assert_eq!(backing_for(0xAA), (0xE9, 0xAA));
    assert_eq!(backing_for(0xBB), (0xE9, 0xBB));
}

/// Leaving `flash_image` unset must keep the pre-existing env-var behaviour,
/// so every single-chip caller that relies on it is unaffected.
#[test]
fn esp32s3_without_an_injected_image_leaves_flash_erased() {
    let mut bus = SystemBus::new();
    let wiring = configure_xtensa_esp32s3(
        &mut bus,
        &Esp32s3Opts {
            real_reset_boot: true,
            ..Esp32s3Opts::default()
        },
    );
    let bytes = wiring.icache_backing.lock().unwrap();
    // With no injected image and no env pin in the test environment, the
    // backing stays at the erased-flash pattern rather than picking up
    // whatever another test injected.
    assert_eq!(bytes[64], 0xFF);
}
