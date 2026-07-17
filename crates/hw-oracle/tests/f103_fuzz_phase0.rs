// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Firmware-fuzzing **Phase 0** check: the fuzz target + crash oracle.
//!
//! Proves the injection contract and the oracle end to end: write an input byte
//! stream into the firmware's RAM input buffer, run `firmware-f103-fuzztarget`,
//! read the verdict — a clean input reaches DONE, the planted `C`-overflow input
//! reaches FAULT — in sim, and (feature-gated) the same on the bench F103 via
//! openocd (RAM-injected so the crashing input is replayable on real silicon).
//!
//! ```text
//! cargo build -p firmware-f103-fuzztarget --target thumbv7m-none-eabi --release
//! cargo test  -p labwired-hw-oracle --test f103_fuzz_phase0
//! STM32_TARGET=stm32f1x cargo test -p labwired-hw-oracle --test f103_fuzz_phase0 \
//!     --features hw-oracle-stm32 -- --ignored --test-threads=1
//! ```

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::Bus;
use labwired_core::Machine;
use labwired_loader::load_elf;
use std::path::{Path, PathBuf};

const FUZZ_LEN: u32 = 0x2000_2800;
const FUZZ_DATA: u32 = 0x2000_2804;
const VERDICT: u32 = 0x2000_3000;
const DONE: u32 = 0xC0DE_F022;
const FAULT: u32 = 0xDEAD_FA17;

/// A clean frame stream (ping + add) — parses to DONE.
const CLEAN: &[u8] = &[b'P', 0, b'A', 3, 1, 2, 3];

/// `C` with length 64 (>16) followed by 64 `0xFF` bytes — the planted stack
/// overflow smashes the saved return address to 0xFFFFFFFF, so the handler's
/// return faults. (Zero filler wouldn't fault: PC=0 is a valid flash alias.)
fn crash_input() -> Vec<u8> {
    let mut v = vec![b'C', 64];
    v.extend(std::iter::repeat_n(0xFF, 64));
    v
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn firmware_elf() -> Option<PathBuf> {
    let base = repo_root().join("target/thumbv7m-none-eabi");
    ["release", "debug"]
        .iter()
        .map(|p| base.join(p).join("firmware-f103-fuzztarget"))
        .find(|p| p.exists())
}

/// LE-pack `input` into 32-bit words for the RAM buffer.
fn packed(input: &[u8]) -> Vec<u32> {
    input
        .chunks(4)
        .map(|c| {
            let mut w = [0u8; 4];
            w[..c.len()].copy_from_slice(c);
            u32::from_le_bytes(w)
        })
        .collect()
}

/// Run the fuzz target in sim on `input`; return the verdict word.
fn run_sim(elf: &Path, input: &[u8]) -> u32 {
    let chip = ChipDescriptor::from_file(repo_root().join("configs/chips/stm32f103.yaml")).unwrap();
    let system_path = repo_root().join("configs/systems/stm32f103-bare.yaml");
    let mut manifest = SystemManifest::from_file(&system_path).unwrap();
    manifest.chip = system_path
        .parent()
        .unwrap()
        .join(&manifest.chip)
        .to_str()
        .unwrap()
        .to_string();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&load_elf(elf).unwrap()).unwrap();

    // Inject the input AFTER loading the firmware, BEFORE running.
    machine
        .bus
        .write_u32(FUZZ_LEN as u64, input.len() as u32)
        .unwrap();
    for (i, w) in packed(input).iter().enumerate() {
        machine
            .bus
            .write_u32((FUZZ_DATA + (i as u32) * 4) as u64, *w)
            .unwrap();
    }

    for _ in 0..1_000_000 {
        // A CPU fault (bad fetch/access from the smashed return) surfaces as a
        // step error in sim rather than vectoring to the HardFault handler the
        // way silicon does — either way it's a crash. (The sim-vs-silicon
        // representation gap is a fidelity item: ideally the sim would take the
        // HardFault exception so the firmware's handler runs identically.)
        if machine.step().is_err() {
            return FAULT;
        }
        match machine.bus.read_u32(VERDICT as u64) {
            Ok(v) if v != 0 => return v,
            _ => {}
        }
    }
    0 // timeout = hang
}

#[test]
fn fuzz_oracle_sim() {
    let Some(elf) = firmware_elf() else {
        eprintln!(
            "skip: build firmware-f103-fuzztarget --target thumbv7m-none-eabi --release first"
        );
        return;
    };
    assert_eq!(run_sim(&elf, CLEAN), DONE, "clean input should reach DONE");
    assert_eq!(
        run_sim(&elf, &crash_input()),
        FAULT,
        "planted C-overflow should reach FAULT"
    );
}

#[cfg(feature = "hw-oracle-stm32")]
fn run_hw(elf: &PathBuf, input: &[u8]) -> u32 {
    use labwired_hw_oracle::openocd::OpenOcd;
    use std::time::{Duration, Instant};

    let target = std::env::var("STM32_TARGET").unwrap_or_else(|_| "stm32f1x".to_string());
    let mut oc = OpenOcd::spawn_stm32(&target).expect("openocd");
    oc.reset_halt().expect("reset_halt");
    let resp = oc
        .tcl(&format!(
            "flash write_image erase {}",
            elf.to_str().unwrap()
        ))
        .expect("flash");
    assert!(!resp.contains("Error"), "flash: {resp}");
    // reset_halt keeps RAM; inject the input while halted, then run.
    oc.reset_halt().expect("reset_halt 2");
    oc.write_memory(FUZZ_LEN, &[input.len() as u32])
        .expect("len");
    oc.write_memory(FUZZ_DATA, &packed(input)).expect("data");
    oc.write_memory(VERDICT, &[0]).expect("clear verdict");
    oc.resume().expect("resume");

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Ok(v) = oc.read_memory(VERDICT, 1) {
            if v[0] != 0 {
                oc.shutdown().ok();
                return v[0];
            }
        }
        if Instant::now() >= deadline {
            oc.shutdown().ok();
            return 0; // hang
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(feature = "hw-oracle-stm32")]
#[test]
#[ignore]
fn fuzz_oracle_hw() {
    let elf = firmware_elf().expect("build firmware-f103-fuzztarget first");
    assert_eq!(run_hw(&elf, CLEAN), DONE, "clean input → DONE on silicon");
    assert_eq!(
        run_hw(&elf, &crash_input()),
        FAULT,
        "C-overflow → FAULT on silicon"
    );
}
