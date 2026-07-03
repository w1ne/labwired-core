// IO-Link station full master-stack service-coverage integration tests.
//
// Task 4: a master chip running the phased service-script firmware
// (`master-fw-svc`) drives a service-rich device chip (`device-fw-svc`) over the
// UartCrossLink and, after reaching OPERATE, exercises the full iolinki-master
// feature surface on the wire: ISDU read (vendor name), cyclic PD output echo,
// event trigger + read, and data-storage write/readback. Every result is
// mirrored into `volatile` master globals that this test reads by ELF symbol.

use labwired_config::EnvironmentManifest;
use labwired_core::world::World;
use std::path::{Path, PathBuf};

fn station_root() -> PathBuf {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/iolink-station"
    ))
    .to_path_buf()
}

// See world_multichip.rs for the rationale: a missing ELF skips by default so the
// toolchain-less workspace gate stays green, but hard-fails when
// LABWIRED_REQUIRE_IOLINK_ELFS is set (the dedicated CI job builds the ELFs).
fn require_iolink_elfs() -> bool {
    std::env::var_os("LABWIRED_REQUIRE_IOLINK_ELFS").is_some()
}

// Returns true if the caller should `return` (skip). Panics — failing the test —
// when ELFs are required but absent.
fn skip_or_fail_missing_elfs(build_hint: &str) -> bool {
    if require_iolink_elfs() {
        panic!(
            "required IO-Link station ELF(s) missing while LABWIRED_REQUIRE_IOLINK_ELFS \
             is set; build them: {build_hint}"
        );
    }
    eprintln!("SKIP: IO-Link station ELF(s) not built; build them: {build_hint}");
    true
}

fn sym(elf_bytes: &[u8], name: &str) -> u64 {
    labwired_loader::resolve_symbol_in_elf(elf_bytes, name)
        .unwrap_or_else(|| panic!("symbol {name} not in ELF")) as u64
}

// Full service script on the wire: ISDU vendor-name read == "LABWIRED", cyclic
// PD-out echo, event trigger/read (code 0x8CA0), and data-storage round-trip.
#[test]
fn master_services_isdu_pdout_event_ds_all_pass_on_wire() {
    let root = station_root();
    let master_elf = root.join("master-fw-svc/master.elf");
    let device_elf = root.join("device-fw-svc/device.elf");
    if !master_elf.exists() || !device_elf.exists() {
        skip_or_fail_missing_elfs(
            "STM32CUBE_L4_DIR=$HOME/projects/STM32CubeL4 bash examples/iolink-station/ci/build.sh",
        );
        return;
    }

    let env = EnvironmentManifest::from_file(root.join("env-svc.yaml")).expect("parse env-svc.yaml");
    let mut world = World::from_manifest(env, &root).expect("build station-svc world");

    // Resolve observability globals straight from the master ELF symbol table.
    let mb = std::fs::read(&master_elf).expect("read master elf");
    let a_done = sym(&mb, "g_svc_done");
    let a_phase = sym(&mb, "g_phase");
    let a_isdu = sym(&mb, "g_isdu_ok");
    let a_vlen = sym(&mb, "g_isdu_vendor_len");
    let a_vbuf = sym(&mb, "g_isdu_vendor");
    let a_pd = sym(&mb, "g_pd_echo_ok");
    let a_ev = sym(&mb, "g_event_ok");
    let a_ev_hi = sym(&mb, "g_event_code_hi");
    let a_ev_lo = sym(&mb, "g_event_code_lo");
    let a_ds = sym(&mb, "g_ds_ok");

    // The ISDU vendor-name transfer is multi-frame segmented and the whole
    // script runs only after OPERATE, so this needs more headroom than the plain
    // OPERATE proof (5M in world_multichip). Bound is ~3x the measured
    // completion iteration; the loop early-exits the instant g_svc_done flips.
    const MAX_STEPS: u64 = 30_000_000;
    let mut done = false;
    let mut done_at = 0u64;
    for i in 0..MAX_STEPS {
        world.step_all();
        if world.machines.get("master").unwrap().read_u8(a_done).unwrap() == 1 {
            done = true;
            done_at = i;
            break;
        }
    }

    let m = world.machines.get("master").unwrap();
    let phase = m.read_u8(a_phase).unwrap();
    let isdu = m.read_u8(a_isdu).unwrap();
    let pd = m.read_u8(a_pd).unwrap();
    let ev = m.read_u8(a_ev).unwrap();
    let ds = m.read_u8(a_ds).unwrap();
    let code = ((m.read_u8(a_ev_hi).unwrap() as u16) << 8) | m.read_u8(a_ev_lo).unwrap() as u16;
    let vlen = m.read_u8(a_vlen).unwrap() as usize;
    let vendor: Vec<u8> = (0..vlen.min(8))
        .map(|i| m.read_u8(a_vbuf + i as u64).unwrap())
        .collect();

    // Flag convention: 1 = proven on wire, 0xEE = service returned an error,
    // 0 = never reached. Surface phase + all flags on any failure so a reader
    // sees exactly which service broke.
    let flags = format!(
        "phase={phase} isdu={isdu:#04x} pd={pd:#04x} event={ev:#04x} ds={ds:#04x} \
         event_code={code:#06x} vendor={vendor:02x?}"
    );

    assert!(
        done,
        "service script never finished (g_svc_done never set): {flags} (ran {MAX_STEPS} steps)"
    );
    eprintln!("g_svc_done flipped at iteration {done_at}; {flags}");

    assert_eq!(isdu, 1, "ISDU vendor-name read failed on wire; {flags}");
    assert_eq!(&vendor, b"LABWIRED", "vendor name mismatch; {flags}");
    assert_eq!(pd, 1, "PD-out echo failed; {flags}");
    assert_eq!(ev, 1, "event read failed; {flags}");
    assert_eq!(code, 0x8CA0, "event code mismatch; {flags}");
    assert_eq!(ds, 1, "data-storage write/readback failed; {flags}");

    eprintln!("services on wire: ISDU+PDOUT+EVENT+DS all green (phase {phase}, done at {done_at})");
}
