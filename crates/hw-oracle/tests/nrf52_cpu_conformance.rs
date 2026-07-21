// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! nRF52840 **CPU**-conformance differential: run `firmware-nrf52840-cpu-conformance`
//! (which exercises ARMv7-M *core* behaviours — SVC delivery, `MRS IPSR`, a
//! switch-table dispatch, and `MPU_TYPE`) on the simulator AND on real silicon,
//! then diff the digests. Unlike the peripheral conformance firmware, every word
//! here is architecture-defined, so sim and silicon must agree exactly.
//!
//! These behaviours were modelled (or fixed) while bringing up Zephyr; this test
//! locks the silicon-measured values so the models can never silently regress.
//!
//! ```text
//! cargo build -p firmware-nrf52840-cpu-conformance --target thumbv7em-none-eabi --release
//! cargo test  -p labwired-hw-oracle --test nrf52_cpu_conformance              # sim only
//! NRF52_TARGET=nrf52 cargo test -p labwired-hw-oracle --test nrf52_cpu_conformance \
//!     --features hw-oracle-nrf52 -- --ignored --test-threads=1               # sim vs silicon
//! ```

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::Bus;
use labwired_core::Machine;
use labwired_loader::load_elf;
use std::path::{Path, PathBuf};

/// RAM address of the verdict block written by the firmware.
const VERDICT_ADDR: u32 = 0x2000_3000;
/// Written to `VERDICT[0]` last, after every check completes.
const DONE_MAGIC: u32 = 0x5284_0D0E;
const DIGEST_WORDS: usize = 16;

// Digest layout (must match the firmware).
const IDX_IPSR_IN_SVC: usize = 1;
const IDX_SWITCH_ACC: usize = 2;
const IDX_MPU_DREGION: usize = 3;
const IDX_LDRPC_ACC: usize = 4;
const IDX_BASEPRI: usize = 5;
const IDX_FAULTMASK: usize = 6;
const IDX_MSP_PSP: usize = 7;
const IDX_ICSR: usize = 8;

// Architecture-defined expectations (identical on sim and silicon).
//
// IPSR read inside the SVCall handler == 11 (the SVCall exception number): this
// requires BOTH that `svc` pends/takes SVCall (not a NOP) AND that `MRS IPSR`
// returns the active exception number (not 0).
const EXPECT_IPSR_IN_SVC: u32 = 11;
// XOR-fold of every switch arm 0..=11 plus the default 0xDEADBEEF — proves each
// input dispatched to the correct arm.
const EXPECT_SWITCH_ACC: u32 = 0x59C8_2174;
// MPU_TYPE.DREGION on the nRF52840 Cortex-M4F.
const EXPECT_MPU_DREGION: u32 = 8;
// Index-weighted fold of a hand-emitted `ldr.w pc,[rn,rm,lsl#2]` jump table over
// 6 cases (0x1001..0x6006). A load-to-PC that mis-suppresses pc_increment lands
// one halfword past the case body and changes this value.
const EXPECT_LDRPC_ACC: u32 = 0x0005_B05B;

// Words 5..8 each pack (TAG << 4) | bool_nibble, where every bool is an
// architecture-defined observation; all-correct yields the value below. A
// regression in the modelled behaviour flips a bool and changes the word.
//
// BASEPRI (#355): masked SysTick stays pending, unmasks once; BASEPRI readback;
// BASEPRI_MAX only-raises (weaker ignored, stronger applied). TAG 0xB.
const EXPECT_BASEPRI: u32 = 0x0000_00BF;
// FAULTMASK (#356): masks a pended SysTick, clears to fire once, and auto-clears
// on exception return (set in-handler → reads 0 after). TAG 0xF.
const EXPECT_FAULTMASK: u32 = 0x0000_00FF;
// MSP/PSP banking (#354): PSP banked + distinct from MSP, exception from
// thread/PSP sees EXC_RETURN=0xFFFFFFFD, PSP restored on return. TAG 0xD.
const EXPECT_MSP_PSP: u32 = 0x0000_00DF;
// ICSR write-only/self-clearing (#358): PENDSVSET self-clears on take, PENDSVCLR
// reads back 0, the `|= PENDSVSET` RMW re-pends exactly once. TAG 0xC.
const EXPECT_ICSR: u32 = 0x0000_00CF;

#[allow(dead_code)]
const LABELS: [&str; DIGEST_WORDS] = [
    "DONE",
    "ipsr_in_svc",
    "switch_acc",
    "mpu_dregion",
    "ldrpc_acc",
    "basepri",
    "faultmask",
    "msp_psp",
    "icsr",
    "rsv9",
    "rsv10",
    "rsv11",
    "rsv12",
    "rsv13",
    "rsv14",
    "rsv15",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn firmware_elf() -> Option<PathBuf> {
    let base = repo_root().join("target/thumbv7em-none-eabi");
    for profile in ["release", "debug"] {
        let p = base.join(profile).join("firmware-nrf52840-cpu-conformance");
        if p.exists() {
            return Some(p);
        }
    }
    None
}

fn build_sim_bus() -> SystemBus {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip_path = manifest_dir.join("../../configs/chips/nrf52840.yaml");
    let system_path = manifest_dir.join("../../configs/systems/seeed-xiao-nrf52840-sense.yaml");

    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load chip {chip_path:?}: {e}"));
    let mut manifest = SystemManifest::from_file(&system_path)
        .unwrap_or_else(|e| panic!("load manifest {system_path:?}: {e}"));
    let anchored = system_path.parent().unwrap().join(&manifest.chip);
    manifest.chip = anchored.to_str().unwrap().to_string();

    SystemBus::from_config(&chip, &manifest).unwrap_or_else(|e| panic!("build sim bus: {e}"))
}

fn run_sim(elf: &Path) -> Vec<u32> {
    let mut bus = build_sim_bus();
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let image = load_elf(elf).expect("load firmware ELF");
    machine.load_firmware(&image).expect("map firmware");

    const MAX_STEPS: usize = 50_000_000;
    let mut done = false;
    for _ in 0..MAX_STEPS {
        machine.step().expect("sim step");
        if matches!(machine.bus.read_u32(VERDICT_ADDR as u64), Ok(DONE_MAGIC)) {
            done = true;
            break;
        }
    }
    assert!(
        done,
        "firmware did not reach DONE in sim within {MAX_STEPS} steps; \
         digest state: {:08X?}",
        read_block(&machine.bus)
    );
    read_block(&machine.bus)
}

fn read_block(bus: &SystemBus) -> Vec<u32> {
    (0..DIGEST_WORDS)
        .map(|i| {
            bus.read_u32((VERDICT_ADDR + (i as u32) * 4) as u64)
                .unwrap_or(0)
        })
        .collect()
}

fn assert_architectural(d: &[u32]) {
    assert_eq!(d[0], DONE_MAGIC, "DONE sentinel — not all checks ran");
    assert_eq!(
        d[IDX_IPSR_IN_SVC], EXPECT_IPSR_IN_SVC,
        "IPSR read inside the SVCall handler must be 11 (SVC delivery + MRS IPSR)"
    );
    assert_eq!(
        d[IDX_SWITCH_ACC], EXPECT_SWITCH_ACC,
        "switch-table dispatch accumulator — a wrong-arm dispatch changes it"
    );
    assert_eq!(
        d[IDX_MPU_DREGION], EXPECT_MPU_DREGION,
        "MPU_TYPE.DREGION must be 8 on the nRF52840"
    );
    assert_eq!(
        d[IDX_LDRPC_ACC], EXPECT_LDRPC_ACC,
        "ldr.w pc,[rn,rm,lsl#2] dispatch — a mis-modelled load-to-PC changes it"
    );
    assert_eq!(
        d[IDX_BASEPRI], EXPECT_BASEPRI,
        "BASEPRI priority masking (#355) — masked SysTick must stay pending, \
         unmask once; BASEPRI/BASEPRI_MAX semantics"
    );
    assert_eq!(
        d[IDX_FAULTMASK], EXPECT_FAULTMASK,
        "FAULTMASK masking + auto-clear-on-return (#356)"
    );
    assert_eq!(
        d[IDX_MSP_PSP], EXPECT_MSP_PSP,
        "MSP/PSP banking + EXC_RETURN=0xFFFFFFFD from thread/PSP (#354)"
    );
    assert_eq!(
        d[IDX_ICSR], EXPECT_ICSR,
        "ICSR write-only/self-clearing PENDSV RMW (#358)"
    );
}

// ── Sim-only test (CI) ────────────────────────────────────────────────────────

#[test]
fn cpu_conformance_sim() {
    let Some(elf) = firmware_elf() else {
        eprintln!(
            "skip: firmware ELF not built — run \
             `cargo build -p firmware-nrf52840-cpu-conformance \
             --target thumbv7em-none-eabi --release` first"
        );
        return;
    };
    let d = run_sim(&elf);
    println!("\nnRF52840 sim CPU-conformance digest:");
    for (i, (&v, label)) in d.iter().zip(LABELS.iter()).enumerate().take(9) {
        println!("  [{i:02}] {label:<14} = 0x{v:08X}");
    }
    assert_architectural(&d);
}

// ── HW + diff (silicon) ───────────────────────────────────────────────────────

/// Silicon capture from the real nRF52840 (FICR.INFO.PART=0x00052840), ST-LINK
/// V2. Words 0..4 captured 2026-06-23; words 5..8 captured 2026-06-24. The
/// firmware digest read over SWD at 0x2000_3000 was:
///   [0]=0x52840D0E (DONE)      [1]=0x0000000B (IPSR-in-SVC = 11)
///   [2]=0x59C82174 (switch)    [3]=0x00000008 (DREGION = 8)
///   [4]=0x0005B05B (ldr.w pc)  [5]=0x000000BF (BASEPRI #355)
///   [6]=0x000000FF (FAULTMASK #356)  [7]=0x000000DF (MSP/PSP banking #354)
///   [8]=0x000000CF (ICSR write-only/self-clearing #358)
/// Every architectural word equals the sim. Lock it: this must never regress.
#[cfg(feature = "hw-oracle-nrf52")]
const SILICON_DIGEST: [u32; 9] = [
    DONE_MAGIC,
    0x0000_000B,
    0x59C8_2174,
    0x0000_0008,
    0x0005_B05B,
    0x0000_00BF,
    0x0000_00FF,
    0x0000_00DF,
    0x0000_00CF,
];

#[cfg(feature = "hw-oracle-nrf52")]
fn run_hw(elf: &PathBuf) -> Vec<u32> {
    use labwired_hw_oracle::openocd::OpenOcd;
    use std::time::{Duration, Instant};

    let mut oc = OpenOcd::spawn_nrf52().expect("spawn openocd for nRF52");
    oc.reset_halt().expect("reset_halt");
    let elf_str = elf.to_str().unwrap();
    let resp = oc
        .tcl(&format!("flash write_image erase {elf_str}"))
        .expect("flash write_image");
    assert!(!resp.contains("Error"), "flash failed: {resp}");
    oc.tcl("reset run").expect("reset run");

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(v) = oc.read_memory(VERDICT_ADDR, 1) {
            if v.first().copied() == Some(DONE_MAGIC) {
                break;
            }
        }
        assert!(
            Instant::now() < deadline,
            "firmware did not reach DONE on silicon within 5 s"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    oc.halt().expect("halt");
    let block = oc
        .read_memory(VERDICT_ADDR, DIGEST_WORDS)
        .expect("read digest block");
    oc.shutdown().ok();
    block
}

#[cfg(feature = "hw-oracle-nrf52")]
#[test]
#[ignore]
fn cpu_conformance_diff() {
    let elf = firmware_elf().expect(
        "build firmware-nrf52840-cpu-conformance first: \
         cargo build -p firmware-nrf52840-cpu-conformance --target thumbv7em-none-eabi --release",
    );
    let sim = run_sim(&elf);
    let hw = run_hw(&elf);

    println!("\nnRF52840 CPU-conformance sim-vs-silicon:");
    for (i, label) in LABELS.iter().enumerate().take(9) {
        let mark = if sim[i] == hw[i] { "ok" } else { "MISMATCH" };
        println!(
            "  [{i:02}] {label:<14} sim=0x{:08X} hw=0x{:08X}  {mark}",
            sim[i], hw[i]
        );
    }

    // Architectural words must equal the locked silicon capture AND the sim.
    for i in 0..SILICON_DIGEST.len() {
        assert_eq!(
            hw[i], SILICON_DIGEST[i],
            "silicon word {i} drifted from the 2026-06-23 capture"
        );
        assert_eq!(sim[i], hw[i], "sim word {i} diverged from silicon");
    }
}
