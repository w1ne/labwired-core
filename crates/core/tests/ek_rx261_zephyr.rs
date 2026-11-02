// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Stock Zephyr v4.3.0 `ek_rx261` images on the RX core over the EK-RX261
//! descriptor, through the same session builder the CLI uses. Fixtures and
//! their provenance: crates/firmware-ek-rx261-zephyr/build.sh. The hello
//! image is the `ek_rx261_zephyr` survival gate; these two need the CMT0
//! tick interrupt through the ICU.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::session::{OpenOptions, Session};
use labwired_core::system::builder::{
    BlobMap, BootMode, BuildOptions, BuildRequest, FirmwareSource,
};
use std::path::Path;
use std::time::Duration;

fn open(fixture: &str) -> Session {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let sys_path = root.join("examples/ek-rx261/system.yaml");
    let mut manifest = SystemManifest::from_file(&sys_path).unwrap();
    manifest.chip = sys_path
        .parent()
        .unwrap()
        .join(&manifest.chip)
        .to_string_lossy()
        .into_owned();
    let chip = ChipDescriptor::from_file(&manifest.chip).unwrap();
    let fw = std::fs::read(root.join("tests/fixtures").join(fixture)).unwrap();
    Session::open(
        BuildRequest {
            chip: &chip,
            system: &manifest,
            firmware: FirmwareSource::Elf(&fw),
            boot: BootMode::FastBoot,
            blobs: &BlobMap::new(),
            options: BuildOptions::default(),
        },
        OpenOptions::default(),
    )
    .unwrap()
}

fn expect(session: &mut Session, line: &str, simulated: Duration) {
    session
        .expect(&regex::escape(line), simulated)
        .unwrap_or_else(|e| panic!("{e}\n--- SCI6 ---\n{}", session.uart_transcript()));
}

#[test]
fn synchronization_alternates_threads_on_the_cmt0_tick() {
    // Each thread prints, busy-waits 100 ms on the CMT1 counter, then
    // k_msleep(500)s and hands a semaphore over: progress needs CMT0
    // compare-match interrupts through the ICU (vector 28), `INT #1`
    // context switches and the SMOVB/SMOVF frame copies.
    let mut s = open("ek-rx261-zephyr-sync.elf");
    expect(
        &mut s,
        "*** Booting Zephyr OS build v4.3.0 ***",
        Duration::from_millis(100),
    );
    for _ in 0..2 {
        expect(
            &mut s,
            "thread_a: Hello World from cpu 0 on ek_rx261!",
            Duration::from_secs(1),
        );
        expect(
            &mut s,
            "thread_b: Hello World from cpu 0 on ek_rx261!",
            Duration::from_secs(1),
        );
    }
}

#[test]
fn kernel_common_passes_every_suite() {
    let mut s = open("ek-rx261-zephyr-kernel-common.elf");
    expect(
        &mut s,
        "PROJECT EXECUTION SUCCESSFUL",
        Duration::from_secs(20),
    );
    let out = s.uart_transcript();
    let suites = out
        .lines()
        .filter(|l| l.starts_with("SUITE PASS - 100.00%"))
        .count();
    assert_eq!(suites, 12, "{out}");
    assert!(!out.contains("SUITE FAIL"), "{out}");
    assert!(
        out.contains("[common_1cpu]: pass = 1, fail = 0, skip = 1"),
        "{out}"
    );
}
