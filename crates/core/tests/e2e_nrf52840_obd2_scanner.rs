use labwired_config::EnvironmentManifest;
use labwired_core::{network::CanBus, world::World};
use std::path::{Path, PathBuf};
use std::process::Command;

fn core_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Build the two firmwares this gate measures.
///
/// Both are real builds, not fixtures: the scanner is a cargo crate and the ECU
/// is C built with arm-none-eabi-gcc, and the whole point of the test is that
/// the two REAL binaries talk to each other over a modelled CAN bus.
///
/// This used to be a documented prerequisite in the example's README, with the
/// test simply reading the paths. That made `cargo test -p labwired-core` red
/// in any clean checkout -- and red with `Os { code: 2, kind: NotFound }` from
/// a bare `fs::read`, which names neither the missing artifact nor the command
/// that produces it. No workflow ran the prerequisite either, so the gate was
/// red wherever it ran rather than measuring anything. A gate that cannot build
/// its own inputs is not a gate.
///
/// `strict_onboarding` already builds its firmware the same way; this follows
/// that precedent rather than inventing one.
fn build_firmwares(root: &Path) {
    let cargo = Command::new("cargo")
        .current_dir(root)
        .args([
            "build",
            "-q",
            "-p",
            "firmware-nrf52840-obd2-scanner",
            "--release",
            "--target",
            "thumbv7em-none-eabi",
        ])
        .status()
        .expect("run cargo to build the OBD2 scanner firmware");
    assert!(
        cargo.success(),
        "building firmware-nrf52840-obd2-scanner failed; the thumbv7em-none-eabi \
         target must be installed (rustup target add thumbv7em-none-eabi)"
    );

    let make = Command::new("make")
        .current_dir(root.join("examples/nrf52840-obd2-scanner/ecu/firmware"))
        .status()
        .expect("run make to build the OBD2 ECU firmware");
    assert!(
        make.success(),
        "building the OBD2 ECU firmware failed; it needs arm-none-eabi-gcc on PATH"
    );
}

fn symbol(elf: &[u8], name: &str) -> u64 {
    labwired_loader::resolve_symbol_in_elf(elf, name)
        .unwrap_or_else(|| panic!("missing firmware export {name}"))
        .into()
}

fn read_bytes(world: &World, node: &str, address: u64, len: usize) -> Vec<u8> {
    (0..len)
        .map(|offset| {
            world.machines[node]
                .read_u8(address + offset as u64)
                .unwrap()
        })
        .collect()
}

fn read_u32(world: &World, node: &str, address: u64) -> u32 {
    u32::from_le_bytes(read_bytes(world, node, address, 4).try_into().unwrap())
}

#[test]
fn real_scanner_and_ecu_firmware_complete_the_obd2_workflow() {
    let root = core_root();
    build_firmwares(&root);
    let env_path = root.join("examples/nrf52840-obd2-scanner/env.yaml");
    let env: EnvironmentManifest = serde_yaml::from_slice(
        &std::fs::read(&env_path).expect("build the example before running its e2e gate"),
    )
    .expect("parse OBD2 environment");
    let scanner_elf_path = root.join(
        &env.nodes
            .iter()
            .find(|n| n.id == "scanner")
            .unwrap()
            .firmware,
    );
    let ecu_elf_path = root.join(&env.nodes.iter().find(|n| n.id == "ecu").unwrap().firmware);
    let scanner_elf = std::fs::read(scanner_elf_path).expect("build the real scanner ELF");
    let ecu_elf = std::fs::read(ecu_elf_path).expect("build the real ECU ELF");

    let rpm = symbol(&scanner_elf, "SCANNER_RPM");
    let speed = symbol(&scanner_elf, "SCANNER_SPEED_KPH");
    let coolant = symbol(&scanner_elf, "SCANNER_COOLANT_C");
    let dtcs = symbol(&scanner_elf, "SCANNER_DTC_COUNT");
    let vin = symbol(&scanner_elf, "VIN_BYTES");
    let vin_valid = symbol(&scanner_elf, "VIN_VALID");
    let ble = symbol(&scanner_elf, "BLE_PAYLOAD");
    let cycles = symbol(&scanner_elf, "CYCLE_COUNT");
    let ble_tx = symbol(&scanner_elf, "TX_DONE_COUNT");
    let snapshot_seq = symbol(&scanner_elf, "SCANNER_SNAPSHOT_SEQ");
    let clear_request = symbol(&scanner_elf, "CLEAR_DTC_REQUEST");
    let clear_result = symbol(&scanner_elf, "CLEAR_DTC_RESULT");
    let ecu_dtcs = symbol(&ecu_elf, "DTC_COUNT");

    let mut world = World::from_manifest(env, &root).expect("assemble two-node OBD2 lab");
    let mut saw_seeded_dtcs = false;
    for step in 0..8_000_000 {
        let results = world.step_all();
        for (node, result) in results {
            if let Err(error) = result {
                panic!("node {node} failed at world step {step}: {error:?}");
            }
        }
        if world.machines["scanner"].read_u8(dtcs).unwrap() == 2 {
            saw_seeded_dtcs = true;
        }
        if saw_seeded_dtcs
            && world.machines["scanner"].read_u8(vin_valid).unwrap() == 1
            && read_u32(&world, "scanner", cycles) > 0
            && read_u32(&world, "scanner", snapshot_seq) & 1 == 0
        {
            break;
        }
    }
    assert!(
        saw_seeded_dtcs,
        "scanner must observe P0133 and U0123 before clearing"
    );
    assert_eq!(read_bytes(&world, "scanner", rpm, 2), 3000u16.to_le_bytes());
    assert_eq!(world.machines["scanner"].read_u8(speed).unwrap(), 88);
    assert_eq!(
        read_bytes(&world, "scanner", coolant, 2),
        90i16.to_le_bytes()
    );
    assert_eq!(read_bytes(&world, "scanner", vin, 17), b"LWOBD2SIM00000001");
    let pre_clear_has_mode03 = world.interconnects[0]
        .as_any_mut()
        .unwrap()
        .downcast_mut::<CanBus>()
        .unwrap()
        .trace_snapshot()
        .iter()
        .any(|frame| frame.id == 0x7DF && frame.data.get(1) == Some(&0x03));

    let cycles_before_clear = read_u32(&world, "scanner", cycles);
    world
        .machines
        .get_mut("scanner")
        .unwrap()
        .write_u8(clear_request, 1)
        .unwrap();
    for step in 0..4_000_000 {
        for (node, result) in world.step_all() {
            if let Err(error) = result {
                panic!("node {node} failed at post-clear world step {step}: {error:?}");
            }
        }
        if world.machines["scanner"].read_u8(clear_result).unwrap() == 2
            && world.machines["scanner"].read_u8(dtcs).unwrap() == 0
            && read_u32(&world, "ecu", ecu_dtcs) == 0
            && read_u32(&world, "scanner", cycles) > cycles_before_clear
            && read_u32(&world, "scanner", snapshot_seq) & 1 == 0
            && read_bytes(&world, "scanner", ble, 9)[..7] == [1, 0x01, 0xB8, 0x0B, 88, 90, 0]
        {
            break;
        }
    }

    assert_eq!(world.machines["scanner"].read_u8(clear_result).unwrap(), 2);
    assert!(
        read_u32(&world, "scanner", cycles) > cycles_before_clear,
        "post-clear telemetry cycle must complete"
    );
    assert_eq!(world.machines["scanner"].read_u8(dtcs).unwrap(), 0);
    assert_eq!(read_u32(&world, "ecu", ecu_dtcs), 0);
    assert_eq!(
        &read_bytes(&world, "scanner", ble, 9)[..7],
        &[1, 0x01, 0xB8, 0x0B, 88, 90, 0]
    );
    assert!(read_u32(&world, "scanner", ble_tx) > 0);

    let can_bus = world.interconnects[0]
        .as_any_mut()
        .unwrap()
        .downcast_mut::<CanBus>()
        .unwrap();
    assert_eq!(
        can_bus.trace_dropped(),
        0,
        "CAN evidence must not be truncated"
    );
    let frames = can_bus.trace_snapshot();
    assert!(frames.iter().any(|frame| frame.id == 0x7DF));
    assert!(frames.iter().any(|frame| frame.id == 0x7E8));
    let services: Vec<u8> = frames
        .iter()
        .filter(|frame| frame.id == 0x7DF && frame.data.len() > 1)
        .map(|frame| frame.data[1])
        .collect();
    let clear = services
        .iter()
        .position(|service| *service == 0x04)
        .expect("Mode 04 request");
    assert!(pre_clear_has_mode03, "Mode 03 must expose seeded DTCs");
    assert!(
        services[clear + 1..].contains(&0x03),
        "Mode 03 must confirm the clear"
    );

    println!(
        "RPM 3000, speed 88, coolant 90, VIN LWOBD2SIM00000001, DTC clear confirmed, CAN frames {}, BLE transmissions {}",
        frames.len(), read_u32(&world, "scanner", ble_tx)
    );
}
