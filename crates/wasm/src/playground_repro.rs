#[cfg(test)]
mod playground_secure_boot_repro {
    //! Does the secure-boot lab reach boot 3 the way the PLAYGROUND drives it?
    //!
    //! The CLI runs it through a test script and sees all three boots. The
    //! browser reportedly sits on boot 1 forever. This drives the same entry
    //! points the playground bridge uses -- `new_from_config_arm`, then
    //! `feed_uart_input`, then `step_batch` in a loop -- so a divergence shows
    //! up here rather than only in a browser nobody can attach a debugger to.
    use crate::{ChipDescriptor, SystemManifest, WasmSimulator};

    const OTA_PACKAGES: &[u8] = &[
        // package 1 (140 bytes)
        0x4C, 0x57, 0x4F, 0x54, 0x02, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x4C, 0x61, 0x62,
        0x57, 0x69, 0x72, 0x65, 0x64, 0x20, 0x4F, 0x54, 0x41, 0x20, 0x69, 0x6D, 0x61, 0x67, 0x65,
        0x20, 0x76, 0x32, 0x20, 0x28, 0x64, 0x65, 0x6D, 0x6F, 0x20, 0x70, 0x61, 0x79, 0x6C, 0x6F,
        0x61, 0x64, 0x2C, 0x20, 0x6E, 0x6F, 0x74, 0x20, 0x65, 0x78, 0x65, 0x63, 0x75, 0x74, 0x61,
        0x62, 0x6C, 0x65, 0x29, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x0D, 0xA7, 0x58, 0xA8, 0x3B, 0x83, 0xA9, 0x84, 0x98, 0x17, 0x55, 0x00, 0xFA, 0xD3,
        0x79, 0xC0, 0x86, 0x95, 0xBB, 0xD1, 0xEB, 0xE2, 0xE6, 0x30, 0xB2, 0x0C, 0xAE, 0xB0, 0x86,
        0x4D, 0xCB, 0x3A, 0x8E, 0xBB, 0x8F, 0x50, 0xB8, 0x47, 0x25, 0xBA, 0x5B, 0x54, 0x64, 0x19,
        0x1E, 0x2B, 0xBB, 0x02, 0xFD, 0xDB, 0xFD, 0x1B, 0x57, 0x81, 0x5C, 0x06, 0xB9, 0x44, 0xA7,
        0x3A, 0xA9, 0x04, 0xA1, 0x6E, // package 2 (140 bytes)
        0x4C, 0x57, 0x4F, 0x54, 0x01, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x4C, 0x61, 0x62,
        0x57, 0x69, 0x72, 0x65, 0x64, 0x20, 0x4F, 0x54, 0x41, 0x20, 0x69, 0x6D, 0x61, 0x67, 0x65,
        0x20, 0x76, 0x31, 0x20, 0x2D, 0x20, 0x61, 0x75, 0x74, 0x68, 0x65, 0x6E, 0x74, 0x69, 0x63,
        0x20, 0x62, 0x75, 0x74, 0x20, 0x4F, 0x4C, 0x44, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x31, 0xEF, 0xD2, 0xC3, 0x12, 0x19, 0xE7, 0x99, 0x09, 0x23, 0x1B, 0x46, 0x76, 0x8E,
        0x08, 0xF5, 0xAB, 0x29, 0xA2, 0x8D, 0x7E, 0xC8, 0x91, 0xA6, 0x8C, 0x82, 0x66, 0xBD, 0xC6,
        0xC4, 0x8E, 0x0B, 0x8D, 0x77, 0xD3, 0xF1, 0x28, 0x21, 0x94, 0x7E, 0xF4, 0xF5, 0xC1, 0xD0,
        0x09, 0x45, 0x8F, 0x25, 0xBF, 0xB5, 0x05, 0x79, 0x9A, 0xF0, 0x45, 0x41, 0x22, 0xA5, 0xC3,
        0x7F, 0xB9, 0xF2, 0xF5, 0x71, // package 3 (140 bytes)
        0x4C, 0x57, 0x4F, 0x54, 0x03, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x4C, 0x61, 0x62,
        0x57, 0x69, 0x72, 0x65, 0x64, 0x20, 0x4F, 0x54, 0x41, 0x20, 0x69, 0x6D, 0x61, 0x67, 0x65,
        0x20, 0x76, 0x33, 0x20, 0x2D, 0x20, 0x46, 0x4F, 0x52, 0x47, 0x45, 0x44, 0x20, 0x73, 0x69,
        0x67, 0x6E, 0x61, 0x74, 0x75, 0x72, 0x65, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x89, 0xE4, 0x9C, 0x72, 0x50, 0xF9, 0x41, 0x48, 0x09, 0x42, 0xC6, 0x2C, 0x45, 0xF3,
        0x0E, 0xC0, 0xF4, 0x35, 0xB0, 0x1A, 0x61, 0x95, 0xDB, 0x6A, 0x0E, 0x7E, 0x08, 0xE7, 0xC1,
        0x86, 0x3B, 0xDB, 0x38, 0x7E, 0x48, 0xC5, 0x84, 0x71, 0x42, 0xF5, 0x27, 0x27, 0xCA, 0x97,
        0x3B, 0xBF, 0x73, 0x3F, 0x87, 0x80, 0x07, 0x57, 0x9F, 0xE6, 0x2A, 0x05, 0x59, 0x5D, 0xAE,
        0x3F, 0x34, 0x38, 0x23, 0xDE,
    ];

    /// Drive the lab exactly as `simulator-bridge.ts` does and return the UART.
    /// Build the lab firmware if the source is newer than the ELF.
    ///
    /// Same shape as `e2e_nrf52840_proximity`. CI runs `cargo test -p
    /// labwired-wasm --lib` without building any firmware first, so a test that
    /// only READS the ELF fails on a clean checkout — which is exactly how the
    /// first version of this file took pr-gate red.
    ///
    /// `CARGO_ENCODED_RUSTFLAGS`/`RUSTFLAGS` are cleared for the reason the core
    /// e2e tests clear them: coverage instrumentation flags leak into the no_std
    /// cross-build and fail it with E0463 (can't find crate `core`).
    fn ensure_firmware_built() -> std::path::PathBuf {
        let elf = labwired_core::test_support::target_dir()
            .join("thumbv7em-none-eabi/release/firmware-nrf52840-secure-boot");
        let src = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/firmware-nrf52840-secure-boot/src/main.rs");
        if let (Ok(e), Ok(s)) = (std::fs::metadata(&elf), std::fs::metadata(&src)) {
            if e.modified().unwrap() >= s.modified().unwrap() {
                return elf;
            }
        }
        let status = std::process::Command::new("cargo")
            .args([
                "build",
                "-p",
                "firmware-nrf52840-secure-boot",
                "--target",
                "thumbv7em-none-eabi",
                "--release",
            ])
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .env_remove("RUSTFLAGS")
            .current_dir(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
            .status()
            .expect("cargo build firmware-nrf52840-secure-boot");
        assert!(status.success(), "secure-boot firmware build failed");
        assert!(elf.exists(), "ELF not found at {elf:?}");
        elf
    }

    fn run_playground_path(idle_fast_forward: bool, auto_tick: bool) -> (String, Vec<u8>) {
        let chip_yaml = include_str!("../../../configs/chips/nrf52840.yaml");
        let system_yaml = include_str!("../../../examples/nrf52840-secure-boot-lab/system.yaml");
        let firmware = std::fs::read(ensure_firmware_built()).expect("read firmware ELF");

        let chip: ChipDescriptor = serde_yaml::from_str(chip_yaml).unwrap();
        let manifest: SystemManifest = serde_yaml::from_str(system_yaml).unwrap();
        let mut sim = WasmSimulator::new_from_config_arm(&chip, &manifest, &firmware)
            .expect("simulator builds");

        // The bridge injects right after construction (simulator-bridge.ts:583).
        sim.feed_uart_input(OTA_PACKAGES);
        if idle_fast_forward {
            sim.set_idle_fast_forward_enabled(true);
        }
        if auto_tick {
            let interval = sim.recommended_tick_interval();
            if interval > 1 {
                sim.set_peripheral_tick_interval(interval);
            }
        }

        let mut uart = String::new();
        for _ in 0..400 {
            sim.step_batch(200_000).expect("batch advances");
            uart.push_str(&String::from_utf8_lossy(&sim.drain_uart_output()));
            if uart.contains("ATTESTATION OK") {
                break;
            }
        }

        // RESULTS.key_words lives at 0x20000058 (see the smoke script header).
        let key = sim.read_memory(0x2000_0058, 16);
        (uart, key)
    }

    fn assert_all_three_boots(uart: &str, label: &str) {
        for marker in [
            "ROT: PROVISIONED",
            "SECURE BOOT OK (v1)",
            "OTA v2 COMMITTED",
            "SECURE BOOT OK (v2)",
            "ROLLBACK REJECTED",
            "ATTESTATION OK",
        ] {
            assert!(
                uart.contains(marker),
                "{label}: never reached {marker:?}. UART so far:\n{uart}"
            );
        }
    }

    #[test]
    fn matrix() {
        for (idle, tick) in [(false, false), (false, true), (true, false), (true, true)] {
            let (uart, key) = run_playground_path(idle, tick);
            let last = uart.lines().last().unwrap_or("(no uart)").to_string();
            let reached = uart.contains("ATTESTATION OK");
            let hex: String = key.iter().map(|b| format!("{b:02X} ")).collect();
            println!(
                "idle_ff={idle:<5} auto_tick={tick:<5} boot3={reached:<5} key={hex}last={last:?}"
            );
        }
    }

    /// At interval 1 the lab walks all three boots, which is what the CLI does.
    #[test]
    fn playground_path_reaches_boot_three() {
        let (uart, _) = run_playground_path(false, false);
        assert_all_three_boots(&uart, "tick interval 1");
    }

    /// KNOWN BUG, ignored so it documents rather than blocks.
    ///
    /// The browser applies `recommended_tick_interval()` at init
    /// (simulator-bridge.ts), and `max_safe_tick_interval` reports > 1 for this
    /// bus because it only rules out HC-SR04 and the legacy walk. The nRF52 RNG
    /// is not safe at that interval: `advance_cycles` produces one byte per
    /// BYTE_PERIOD in a loop, overwriting `value` and raising VALRDY once, so a
    /// batch spanning N periods hands firmware only the last byte and drops the
    /// other N-1.
    ///
    /// The provisioned root key comes out shifted four bytes:
    ///
    ///   interval 1   22 CA B3 3F 02 30 A2 F2 0B EE F7 8A 31 63 B7 56
    ///   interval >1  02 30 A2 F2 0B EE F7 8A 31 63 B7 56 B6 16 91 9C
    ///
    /// so the AES boot challenge misses GOLDEN_CIPHERTEXT, boot 2 prints
    /// SECURE BOOT FAILED, and the lab never reaches boots 2 or 3 in the
    #[test]
    fn browser_tick_interval_reaches_boot_three() {
        let (uart, _) = run_playground_path(false, true);
        assert_all_three_boots(&uart, "auto-raised tick interval");
    }
}
