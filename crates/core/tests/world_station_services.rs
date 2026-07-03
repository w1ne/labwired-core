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

// Shared setup for the wire-fault tests: build the station-svc world and read the
// master ELF bytes, or return `None` (after emitting the skip/fail decision via
// the shared helper) when the firmwares are not built.
fn build_station_or_skip() -> Option<(World, Vec<u8>)> {
    let root = station_root();
    let master_elf = root.join("master-fw-svc/master.elf");
    let device_elf = root.join("device-fw-svc/device.elf");
    if !master_elf.exists() || !device_elf.exists() {
        skip_or_fail_missing_elfs(
            "STM32CUBE_L4_DIR=$HOME/projects/STM32CubeL4 bash examples/iolink-station/ci/build.sh",
        );
        return None;
    }
    let env =
        EnvironmentManifest::from_file(root.join("env-svc.yaml")).expect("parse env-svc.yaml");
    let world = World::from_manifest(env, &root).expect("build station-svc world");
    let mb = std::fs::read(&master_elf).expect("read master elf");
    Some((world, mb))
}

// Read a `volatile uint8_t` master global by its resolved ELF address.
fn master_u8(world: &World, addr: u64) -> u8 {
    world.machines.get("master").unwrap().read_u8(addr).unwrap()
}

// Reach the point-to-point C/Q wire as its concrete type for fault injection.
fn crosslink(world: &mut World) -> &mut labwired_core::network::UartCrossLink {
    world.interconnects[0]
        .as_any_mut()
        .expect("interconnect exposes as_any_mut")
        .downcast_mut::<labwired_core::network::UartCrossLink>()
        .expect("interconnect[0] is a UartCrossLink")
}

// Step every chip to OPERATE (raw master state 3). Returns the iteration it hit.
fn step_to_operate(world: &mut World, a_state: u64, max_steps: u64) -> u64 {
    for i in 0..max_steps {
        world.step_all();
        if master_u8(world, a_state) == 3 {
            return i;
        }
    }
    panic!("master never reached OPERATE (state 3) within {max_steps} steps");
}

// Fault test 1 — a single CRC-corrupted device->master frame must be counted as a
// checksum error AND survived in place: the master retries and stays in OPERATE,
// it does NOT fall through to ERROR. (Corrupting exactly 2 bytes flips one frame.)
#[test]
fn master_survives_single_crc_corrupted_frame() {
    let Some((mut world, mb)) = build_station_or_skip() else {
        return;
    };
    let a_state = sym(&mb, "g_master_state");
    let a_ck = sym(&mb, "g_diag_ck_errors");
    let a_err = sym(&mb, "g_error_seen");

    let op_at = step_to_operate(&mut world, a_state, 10_000_000);
    eprintln!("[crc] reached OPERATE at iteration {op_at}");

    // Corrupt the next 2 device->master bytes = exactly one malformed frame.
    crosslink(&mut world).set_corrupt_b_to_a(2);

    const MAX_AFTER: u64 = 2_000_000;
    let mut ck_at = None;
    for i in 0..MAX_AFTER {
        world.step_all();
        if master_u8(&world, a_ck) >= 1 {
            ck_at = Some(i);
            break;
        }
    }

    let ck = master_u8(&world, a_ck);
    let state = master_u8(&world, a_state);
    let err = master_u8(&world, a_err);
    eprintln!("[crc] after corruption: ck_errors={ck} state={state} error_seen={err} counted_at={ck_at:?}");

    assert!(
        ck >= 1,
        "checksum error was never counted after a corrupt frame (g_diag_ck_errors={ck}, ran {MAX_AFTER} steps)"
    );
    assert_eq!(
        state, 3,
        "master dropped out of OPERATE after a single corrupt frame (state={state}); expected retry-in-place"
    );
    assert_eq!(
        err, 0,
        "master fell through to ERROR on a single corrupt frame (g_error_seen={err}); expected survival"
    );
}

// Fault test 2 — sustained CRC corruption on the device->master direction must
// exhaust the master's rx-retry budget (3 consecutive bad frames), drive the
// port to ERROR, and fire the firmware's ERROR handler, which counts the event
// and restarts the port. Because the ERROR handler restarts immediately we key
// off the sticky g_error_seen / g_restart_count rather than catching state-4 live.
#[test]
fn sustained_crc_corruption_drives_master_to_error_and_restart() {
    let Some((mut world, mb)) = build_station_or_skip() else {
        return;
    };
    let a_state = sym(&mb, "g_master_state");
    let a_ck = sym(&mb, "g_diag_ck_errors");
    let a_err = sym(&mb, "g_error_seen");
    let a_restart = sym(&mb, "g_restart_count");

    let op_at = step_to_operate(&mut world, a_state, 10_000_000);
    eprintln!("[error] reached OPERATE at iteration {op_at}");

    // Corrupt a long run of device->master bytes: spans several response frames,
    // so the retry budget (rx_retry_count < 2, then ERROR on the 3rd bad frame)
    // is exhausted and the port is driven to ERROR.
    crosslink(&mut world).set_corrupt_b_to_a(32);

    const MAX_AFTER: u64 = 2_000_000;
    let mut saw_ck = false;
    let mut restart_at = None;
    for i in 0..MAX_AFTER {
        world.step_all();
        if master_u8(&world, a_ck) >= 1 {
            saw_ck = true; // checksum errors were counted before the restart reset diagnostics
        }
        if master_u8(&world, a_restart) >= 1 {
            restart_at = Some(i);
            break;
        }
    }

    let restart_at = restart_at
        .expect("sustained corruption never drove the master to ERROR/restart within 2M steps");
    let err = master_u8(&world, a_err);
    let restarts = master_u8(&world, a_restart);
    eprintln!(
        "[error] ERROR/restart at iteration {restart_at}; error_seen={err} restart_count={restarts} saw_ck={saw_ck}"
    );

    assert!(
        saw_ck,
        "the master reached ERROR without ever counting a checksum error; the corruption was not the cause"
    );
    assert_eq!(
        err, 1,
        "sustained corruption did not drive the master to ERROR (g_error_seen={err})"
    );
    assert!(
        restarts >= 1,
        "the ERROR handler did not fire a restart (g_restart_count={restarts})"
    );
}

// Fault test 3 — a muted (absent/dead) device must be detected. With the device
// frozen, no response frames arrive; the master's response-timeout scheduling
// (master-fw-svc issues a RESPONSE_TIMEOUT tick once response_deadline passes)
// counts response timeouts, exhausts the retry budget, and drives the port to
// ERROR + restart. We step ONLY the master and tick the wires (the device never
// advances) to model a silent partner.
//
// This is the canonical "dead device" fault and it is only reachable because the
// firmware schedules RESPONSE_TIMEOUT ticks — a coarse per-cycle clock, or the
// CYCLE_DUE-only loop it replaced, steps straight over the sub-cycle timeout
// window and a muted device would (wrongly) look healthy forever.
//
// KNOWN GAP, deliberately NOT asserted: the port does NOT recover to OPERATE
// after the device is un-muted. The ERROR handler restarts immediately and
// re-wakes, but the device stack only re-syncs to a fresh wake-up after ~1000ms
// of link *silence* (dll.c SDCI->SIO inactivity fallback), which the
// continuously-restarting master never provides, so it stays in STARTUP. That is
// a genuine master/device resync limitation worth a follow-up, not something to
// paper over here.
#[test]
fn master_detects_muted_device_via_response_timeout() {
    let Some((mut world, mb)) = build_station_or_skip() else {
        return;
    };
    let a_state = sym(&mb, "g_master_state");
    let a_timeouts = sym(&mb, "g_diag_timeouts");
    let a_err = sym(&mb, "g_error_seen");
    let a_restart = sym(&mb, "g_restart_count");

    let op_at = step_to_operate(&mut world, a_state, 10_000_000);
    eprintln!("[mute] reached OPERATE at iteration {op_at}");

    // Mute the device: advance ONLY the master and tick the wires. The device
    // CPU never steps, so it emits no response frames.
    const MAX_MUTE: u64 = 5_000_000;
    let mut saw_timeout = false;
    let mut err_at = None;
    for i in 0..MAX_MUTE {
        world.machines.get_mut("master").unwrap().step().unwrap();
        for ic in world.interconnects.iter_mut() {
            ic.tick().unwrap();
        }
        if master_u8(&world, a_timeouts) >= 1 {
            saw_timeout = true; // a response timeout was counted before the restart reset it
        }
        if master_u8(&world, a_err) == 1 {
            err_at = Some(i);
            break;
        }
    }

    let err_at = err_at.expect("a muted device never drove the master to ERROR within the bound");
    let timeouts = master_u8(&world, a_timeouts);
    eprintln!(
        "[mute] ERROR at iteration {err_at}; saw_timeout={saw_timeout} timeouts_now={timeouts} \
         error_seen={} state={}",
        master_u8(&world, a_err),
        master_u8(&world, a_state)
    );

    assert!(
        saw_timeout,
        "the master reached ERROR without ever counting a response timeout; the mute was not the cause"
    );
    assert_eq!(
        master_u8(&world, a_err),
        1,
        "a muted device did not drive the master to ERROR (g_error_seen != 1)"
    );

    // Keep muting briefly so the ERROR handler's restart is observable.
    for _ in 0..2_000_000u64 {
        world.machines.get_mut("master").unwrap().step().unwrap();
        for ic in world.interconnects.iter_mut() {
            ic.tick().unwrap();
        }
        if master_u8(&world, a_restart) >= 1 {
            break;
        }
    }
    assert!(
        master_u8(&world, a_restart) >= 1,
        "the ERROR handler did not fire a restart after the mute (g_restart_count=0)"
    );
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

    let env =
        EnvironmentManifest::from_file(root.join("env-svc.yaml")).expect("parse env-svc.yaml");
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
    // Scope the fidelity accounting to this run: any undecoded instruction or
    // unmapped MMIO the real firmware hits must be zero (the UADD8/SEL strlen
    // bug is exactly what this guards against).
    labwired_core::fidelity::take();

    const MAX_STEPS: u64 = 30_000_000;
    let mut done = false;
    let mut done_at = 0u64;
    for i in 0..MAX_STEPS {
        world.step_all();
        if world
            .machines
            .get("master")
            .unwrap()
            .read_u8(a_done)
            .unwrap()
            == 1
        {
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

    // The whole real-firmware run must not have hit a single simulator-coverage
    // gap. A non-empty report means the model silently skipped an instruction or
    // swallowed an unmapped access — the class of bug that made string ISDU
    // reads return garbage until UADD8/SEL were implemented.
    let fidelity = labwired_core::fidelity::report();
    eprintln!("{fidelity}");
    assert!(
        fidelity.is_empty(),
        "simulator hit coverage gaps during a real firmware run:\n{fidelity}"
    );
}
