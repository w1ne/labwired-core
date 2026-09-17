use super::*;

fn chip(flash: &str, ram: &str) -> Result<ChipDescriptor, serde_yaml::Error> {
    serde_yaml::from_str(&format!(
        "name: t\narch: arm\nflash: {{ base: 0, size: \"{flash}\" }}\n\
             ram: {{ base: 0x20000000, size: \"{ram}\" }}\nperipherals: []\n"
    ))
}

/// The wire format is unchanged: every spelling the chip yamls use still loads.
#[test]
fn the_human_forms_still_load() {
    assert_eq!(chip("64KB", "16KiB").unwrap().flash.size, 64 * 1024);
    assert_eq!(chip("64KB", "16KiB").unwrap().ram.size, 16 * 1024);
    assert_eq!(chip("1MiB", "131072").unwrap().flash.size, 1024 * 1024);
    assert_eq!(chip("1MiB", "131072").unwrap().ram.size, 131_072);
}

/// KB is BINARY and MB is DECIMAL, in the same parser. Pinned here because
/// it is the opposite of what the spelling suggests and nothing else states
/// it.
///
/// No committed chip relies on the `MB` arm any more. Nine of them used to,
/// and every one modelled less flash than its part has; esp32s3 was
/// rewritten to `"16384KB"` first, and the remaining eight (esp32c3,
/// rp2040, rp2350, stm32f103/f405/f407/f767/l476, plus the C3's DROM
/// window) followed. The multipliers still cannot move — they are the wire
/// format every out-of-tree descriptor and every hosted manifest was
/// written against — so the spelling is policed instead, over the shipped
/// corpus, by `labwired_core::tests::chip_memory_sizes`.
#[test]
fn kb_is_1024_and_mb_is_1000000() {
    assert_eq!(chip("1KB", "1KB").unwrap().flash.size, 1024);
    assert_eq!(chip("1MB", "1KB").unwrap().flash.size, 1_000_000);
    assert_eq!(chip("1MiB", "1KB").unwrap().flash.size, 1_048_576);
}

/// The point of moving the parse to the boundary: a size that does not
/// parse is now a load error. It used to be stored verbatim and then hit
/// `parse_size(..).unwrap_or(0)` at the point of use — so a typo'd unit
/// gave the machine ZERO bytes of RAM and ran anyway.
#[test]
fn an_unparseable_size_fails_the_load_instead_of_becoming_zero() {
    let err = chip("64K", "16KB").expect_err("a bare `K` is not a unit human_size accepts");
    let msg = err.to_string();
    assert!(
        msg.contains("flash"),
        "the error must name the field: {msg}"
    );
    assert!(
        msg.contains("Invalid size format"),
        "the error must say what was wrong: {msg}"
    );
}

/// Sizes round-trip through serde without changing value. They serialise as
/// a bare byte count precisely so this holds — re-rendering `1048576` as
/// `1MB` would read back as 1_000_000.
#[test]
fn a_size_round_trips_without_shrinking() {
    let c = chip("1MiB", "192KB").unwrap();
    let back: ChipDescriptor = serde_yaml::from_str(&serde_yaml::to_string(&c).unwrap()).unwrap();
    assert_eq!(back.flash.size, c.flash.size);
    assert_eq!(back.ram.size, c.ram.size);
    assert_eq!(back.flash.size, 1_048_576);
}
