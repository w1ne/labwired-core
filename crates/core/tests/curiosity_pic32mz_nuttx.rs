// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Stock upstream NuttX images for the Curiosity PIC32MZ EF 2.0 on the MIPS
//! core over the PIC32MZ2048EFM144 descriptor, through the same session
//! builder the CLI uses. Fixtures and their provenance:
//! crates/firmware-curiosity-pic32mz-nuttx/build.sh. The nsh banner is the
//! `curiosity_pic32mz_nuttx` survival gate; these need the Timer1 tick, the
//! core software interrupt that switches tasks, and the interrupt-driven
//! UART2 in both directions.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::session::{OpenOptions, Session};
use labwired_core::system::builder::{
    BlobMap, BootMode, BuildOptions, BuildRequest, FirmwareSource,
};
use std::path::Path;
use std::time::Duration;

fn open(fixture: &str) -> Session {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let sys_path = root.join("examples/curiosity-pic32mz-ef/system.yaml");
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
        .unwrap_or_else(|e| panic!("{e}\n--- UART2 ---\n{}", session.uart_transcript()));
}

#[test]
fn nsh_runs_typed_commands() {
    // Bytes sent to the session land in UART2's RX queue with no bus call;
    // the receiver's character-time check must still raise U2RX.
    let mut s = open("curiosity-pic32mz-ef-nuttx-nsh.elf");
    expect(&mut s, "NuttShell (NSH)", Duration::from_millis(100));
    expect(&mut s, "nsh> ", Duration::from_millis(100));
    s.send(b"hello\r");
    expect(&mut s, "Hello, World!!", Duration::from_millis(200));
    expect(&mut s, "nsh> ", Duration::from_millis(100));
    // `ps` walks the task list: the Idle task and the shell must be there
    // with sane states after the hello task has come and gone.
    s.send(b"ps\r");
    expect(&mut s, "Idle_Task", Duration::from_millis(200));
    expect(&mut s, "nsh_main", Duration::from_millis(200));
    // The board's bring-up registered the console and the user LEDs.
    s.send(b"ls /dev\r");
    expect(&mut s, "ttyS0", Duration::from_millis(200));
    expect(&mut s, "userleds", Duration::from_millis(200));
}

#[test]
#[ignore = "about 10 minutes: ostest spins the CPU by design (hog_cpu to INT_MAX)"]
fn ostest_passes_every_group() {
    // The full stock ostest as the init task: signals, POSIX timers, message
    // queues, mutexes/semaphores/condition variables, cancellation,
    // round-robin preemption on the Timer1 tick and priority inheritance.
    let mut s = open("curiosity-pic32mz-ef-nuttx-ostest.elf");
    let started = std::time::Instant::now();
    expect(
        &mut s,
        "ostest_main: Exiting with status 0",
        Duration::from_secs(600),
    );
    eprintln!(
        "ostest: {:?} simulated, {} cycles, {:?} wall",
        s.time(),
        s.cycles(),
        started.elapsed()
    );
    let out = s.uart_transcript();
    // 34 groups: 33 "user_main: <name> test" lines plus "libc tests".
    let groups = out
        .lines()
        .map(str::trim_end)
        .filter(|l| l.starts_with("user_main: ") && (l.ends_with(" test") || l.ends_with(" tests")))
        .count();
    assert_eq!(groups, 34, "{out}");
    assert!(!out.contains("ERROR"), "{out}");
}
