// Synthetic domain fixture. These names deliberately do not encode a chip or
// project; exact device descriptors and hardware qualification are separate.
use labwired_config::PowerDomainsConfig;
pub const DOMAIN_YAML: &str = "
rails:
  core: {nominal_v: 1.1, min_v: 1.05, max_v: 1.16, pins: [CORE]}
  io: {nominal_v: 3.3, min_v: 1.62, max_v: 3.63, pins: [IO]}
  detector: {nominal_v: 3.3, min_v: 3.135, max_v: 3.63, pins: [DETECTOR]}
  flash: {nominal_v: 3.3, min_v: 2.97, max_v: 3.63, pins: [FLASH]}
io_domain: io
execution_domains: [core]
system_reset_scope: core_only
latched_por: {sense_domain: core, detector_domain: detector, rearm_below_v: 1.1, release_at_v: 0.957, release_delay_ns: 3000}
brown_out: {sense_domain: core, detector_domain: detector, enabled: true, falling_v: 0.946, rising_v: 0.95546, startup_delay_ns: 55000, assert_delay_ns: 3000, release_delay_ns: 55000, enable_delay_ns: 35000, reprogram_delay_ns: 20000}
";
pub fn config() -> PowerDomainsConfig {
    serde_yaml::from_str(DOMAIN_YAML).unwrap()
}

#[allow(dead_code)] // Used only by the World integration fixture.
pub fn firmware() -> Vec<u8> {
    // ELF32 ARM, vector table, ADD r0,#1; B loop. The register witnesses hold.
    let code = [0x3001u16, 0xe7fd];
    let length = 8 + code.len() as u32 * 2;
    let mut bytes = vec![0; 84 + length as usize];
    bytes[0..7].copy_from_slice(b"\x7fELF\x01\x01\x01");
    for (offset, value) in [(16, 2u16), (18, 40), (40, 52), (42, 32), (44, 1)] {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }
    for (offset, value) in [
        (20, 1u32),
        (24, 0x08000009),
        (28, 52),
        (36, 0x05000000),
        (52, 1),
        (56, 84),
        (60, 0x08000000),
        (64, 0x08000000),
        (68, length),
        (72, length),
        (76, 5),
        (80, 4),
        (84, 0x20001000),
        (88, 0x08000009),
    ] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    for (i, instruction) in code.iter().enumerate() {
        bytes[92 + i * 2..94 + i * 2].copy_from_slice(&instruction.to_le_bytes());
    }
    bytes
}
