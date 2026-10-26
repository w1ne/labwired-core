use std::path::PathBuf;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;

// Coverage source: configs/chips/stm32u5a5zj.yaml.

const RCC: u64 = 0x4602_0c00;
const HASH: u64 = 0x420c_0400;
const SAES: u64 = 0x420c_0c00;
const PKA: u64 = 0x420c_2000;
const PKA_MODE_ARITH_SUB: u32 = 0x0a;

fn u5a5_bus() -> SystemBus {
    let system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("configs/systems/nucleo-u5a5zj.yaml");
    let mut manifest = SystemManifest::from_file(&system_path).unwrap();
    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).unwrap();
    manifest.chip = chip_path.to_string_lossy().into_owned();
    SystemBus::from_config(&chip, &manifest).unwrap()
}

#[test]
fn u5a5_hash_saes_and_pka_execute_register_level_operations() {
    let mut bus = u5a5_bus();
    bus.write_u32(RCC + 0x8c, (1 << 17) | (1 << 19) | (1 << 20))
        .unwrap();

    // HASH SHA-256 of "abc". U5 DATATYPE_8B reverses the CPU word byte order.
    bus.write_u32(HASH, (3 << 17) | (2 << 4) | (1 << 2))
        .unwrap();
    bus.write_u32(HASH + 4, u32::from_le_bytes(*b"abc\0"))
        .unwrap();
    bus.write_u32(HASH + 8, (1 << 8) | (3 * 8)).unwrap();
    assert_eq!(bus.read_u32(HASH + 0x24).unwrap() & (1 << 1), 1 << 1);
    assert_eq!(bus.read_u32(HASH + 0x310).unwrap(), 0xba78_16bf);

    // SAES AES-128 ECB, NIST SP 800-38A first block.
    let key = [
        0x2b, 0x7e, 0x15, 0x16, 0x28, 0xae, 0xd2, 0xa6, 0xab, 0xf7, 0x15, 0x88, 0x09, 0xcf, 0x4f,
        0x3c,
    ];
    let plaintext = [
        0x6b, 0xc1, 0xbe, 0xe2, 0x2e, 0x40, 0x9f, 0x96, 0xe9, 0x3d, 0x7e, 0x11, 0x73, 0x93, 0x17,
        0x2a,
    ];
    bus.write_u32(SAES, 1 | (2 << 1)).unwrap();
    for (index, word) in key.chunks_exact(4).enumerate() {
        bus.write_u32(
            SAES + 0x10 + index as u64 * 4,
            u32::from_le_bytes(word.try_into().unwrap()),
        )
        .unwrap();
    }
    for word in plaintext.chunks_exact(4) {
        bus.write_u32(SAES + 8, u32::from_le_bytes(word.try_into().unwrap()))
            .unwrap();
    }
    let mut ciphertext = Vec::new();
    for _ in 0..4 {
        ciphertext.extend_from_slice(&bus.read_u32(SAES + 0x0c).unwrap().to_le_bytes());
    }
    assert_eq!(
        ciphertext,
        [
            0x3a, 0xd7, 0x7b, 0xb4, 0x0d, 0x7a, 0x36, 0x60, 0xa8, 0x9e, 0xca, 0xf3, 0x24, 0x66,
            0xef, 0x97,
        ]
    );
    assert_eq!(bus.read_u32(SAES + 4).unwrap() & 1, 1);

    // PKA modular exponentiation, the usual RSA/DH primitive: 4^13 mod 497 = 445.
    bus.write_u32(PKA + 0x400, 4).unwrap();
    bus.write_u32(PKA + 0x408, 9).unwrap();
    bus.write_u32(PKA + 0x0c68, 4).unwrap();
    bus.write_u32(PKA + 0x0e78, 13).unwrap();
    bus.write_u32(PKA + 0x1088, 497).unwrap();
    bus.write_u32(PKA, 1 | (1 << 1)).unwrap();
    assert_eq!(bus.read_u32(PKA + 0x0838).unwrap(), 445);
    assert_eq!(bus.read_u32(PKA + 4).unwrap() & (1 << 17), 1 << 17);
}

#[test]
fn u5a5_pka_arithmetic_subtraction_returns_sign_extended_negative_result() {
    for operand_bits in [8, 32, 33, 4160] {
        let mut bus = u5a5_bus();
        bus.write_u32(RCC + 0x8c, 1 << 19).unwrap();

        // RM0456 §53.4.10: 3-5 is represented in the containing 32-bit word
        // as two's complement; following words are sign-extended. U5 supports
        // operand lengths through 4160 bits.
        bus.write_u32(PKA + 0x408, operand_bits).unwrap(); // operand length in bits
        bus.write_u32(PKA + 0x0a50, 3).unwrap(); // A
        bus.write_u32(PKA + 0x0c68, 5).unwrap(); // B
        bus.write_u32(PKA, 1 | (1 << 1) | (PKA_MODE_ARITH_SUB << 8))
            .unwrap();

        let result_words = (operand_bits as usize).div_ceil(32);
        assert_eq!(bus.read_u32(PKA + 0x0e78).unwrap(), u32::MAX - 1);
        for index in 1..=result_words {
            assert_eq!(
                bus.read_u32(PKA + 0x0e78 + index as u64 * 4).unwrap(),
                u32::MAX,
                "operand length {operand_bits}: negative result sign extension"
            );
        }
        let status = bus.read_u32(PKA + 4).unwrap();
        assert_ne!(status & (1 << 17), 0, "valid subtraction completes");
        assert_eq!(status & (1 << 21), 0, "valid underflow is not OPERRF");
    }
}

#[test]
fn u5a5_pka_rejects_operand_widths_above_the_hardware_limit() {
    let mut arithmetic_bus = u5a5_bus();
    arithmetic_bus.write_u32(RCC + 0x8c, 1 << 19).unwrap();
    arithmetic_bus.write_u32(PKA + 0x408, 4161).unwrap();
    arithmetic_bus.write_u32(PKA + 0x0a50, 3).unwrap();
    arithmetic_bus.write_u32(PKA + 0x0c68, 5).unwrap();
    arithmetic_bus
        .write_u32(PKA, 1 | (1 << 1) | (PKA_MODE_ARITH_SUB << 8))
        .unwrap();
    let arithmetic_status = arithmetic_bus.read_u32(PKA + 4).unwrap();
    assert_ne!(arithmetic_status & (1 << 17), 0);
    assert_ne!(arithmetic_status & (1 << 21), 0);
    assert_eq!(arithmetic_bus.read_u32(PKA + 0x0e78).unwrap(), 0);

    let mut exponent_bus = u5a5_bus();
    exponent_bus.write_u32(RCC + 0x8c, 1 << 19).unwrap();
    exponent_bus.write_u32(PKA + 0x400, 4161).unwrap();
    exponent_bus.write_u32(PKA + 0x408, 8).unwrap();
    exponent_bus.write_u32(PKA + 0x0c68, 4).unwrap();
    exponent_bus.write_u32(PKA + 0x0e78, 13).unwrap();
    exponent_bus.write_u32(PKA + 0x1088, 497).unwrap();
    exponent_bus.write_u32(PKA, 1 | (1 << 1)).unwrap();
    let exponent_status = exponent_bus.read_u32(PKA + 4).unwrap();
    assert_ne!(exponent_status & (1 << 17), 0);
    assert_ne!(exponent_status & (1 << 21), 0);
    assert_eq!(exponent_bus.read_u32(PKA + 0x0838).unwrap(), 0);
}
