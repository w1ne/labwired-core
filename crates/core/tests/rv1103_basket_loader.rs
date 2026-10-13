use labwired_core::system::rv1103_basket::BasketArtifacts;
use labwired_core::system::rv1103_basket::{load_basket, validate_archive, FUNCTIONAL_CPU_HZ};
use labwired_core::system::rv1103_linux::*;
use labwired_core::{bus::SystemBus, memory::LinearMemory};

fn prepared() -> BasketArtifacts {
    BasketArtifacts::read(std::path::Path::new(
        &std::env::var("LABWIRED_RV1103_BASKET_ARTIFACTS").unwrap(),
    ))
    .unwrap()
}
fn bus() -> SystemBus {
    let mut bus = SystemBus::empty();
    bus.ram = LinearMemory::new(RAM_BYTES as usize, 0);
    bus.ram.data.fill(0xa5);
    bus.cpu_hz = FUNCTIONAL_CPU_HZ;
    bus
}

#[test]
fn normal_sensor_inspection_exposes_same_instance_registers_and_actual_component_inputs() {
    use labwired_core::{cpu::cortex_m::CortexM, Machine};
    let mut b = SystemBus::empty();
    b.cpu_hz = FUNCTIONAL_CPU_HZ;
    b.configure_armv7_rv1103().unwrap();
    b.configure_armv7_rv1103_i2c3().unwrap();
    b.configure_armv7_rv1103_uart4().unwrap();
    b.register_supply_domain("external3v3", 2.7).unwrap();
    b.attach_manifest_external_devices(
        &serde_json::from_str(labwired_core::system::rv1103_basket::CANONICAL_MANIFEST).unwrap(),
    )
    .unwrap();
    b.update_supply_domain("external3v3", 3.3).unwrap();
    let mut machine = Machine::new(CortexM::new(), b);
    machine.set_input_on("accelerometer", "x", 1.).unwrap();
    let inspected = machine.bus.inspect_devices(None, &Default::default());
    let sensor = inspected
        .iter()
        .find(|d| d.id == "accelerometer")
        .expect("canonical sensor identity");
    let artifact = sensor
        .artifacts
        .iter()
        .find(|a| a.kind == "i2c_registers")
        .expect("live sensor must expose its actual registers");
    assert_eq!(artifact.meta["registers"]["DEVID"], 229);
    assert_eq!(artifact.meta["registers"]["POWER_CTL"], 0);
    assert_eq!(artifact.meta["inputs"]["x"], 1.);
    assert_eq!(artifact.meta["powered"], true);
}

#[test]
#[ignore = "requires reviewed pinned artifacts in LABWIRED_RV1103_BASKET_ARTIFACTS"]
fn reviewed_prepared_profile_is_accepted() {
    BasketArtifacts::read(std::path::Path::new(
        &std::env::var("LABWIRED_RV1103_BASKET_ARTIFACTS").unwrap(),
    ))
    .expect("reviewed exact basket profile must load");
}

#[test]
#[ignore = "requires reviewed pinned artifacts in LABWIRED_RV1103_BASKET_ARTIFACTS"]
fn same_reviewed_profile_accepts_independent_build_location_without_self_authorizing_artifacts() {
    let source =
        std::path::PathBuf::from(std::env::var("LABWIRED_RV1103_BASKET_ARTIFACTS").unwrap());
    let dir = std::env::temp_dir().join(format!("basket-equivalent-build-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(source.join("artifact-manifest.json")).unwrap())
            .unwrap();
    for (name, pin) in manifest["artifacts"].as_object_mut().unwrap() {
        if name.ends_with(".log") {
            let bytes = b"independent preparation tool log\\n";
            std::fs::write(dir.join(name), bytes).unwrap();
            pin["bytes"] = (bytes.len() as u64).into();
            pin["sha256"] = sha256(bytes).into();
        } else {
            std::fs::hard_link(source.join(name), dir.join(name)).unwrap();
        }
    }
    for build in manifest["builds"].as_array_mut().unwrap() {
        build["directory"] = "/independent/reviewed/build".into();
        build["environment"]["PATH"] = "/independent/toolchain/bin".into();
        build["compiler"]["path"] = "/independent/toolchain/arm-gcc".into();
        build["dtc"]["version"] = "Independent pinned-compatible DTC build".into();
    }
    std::fs::write(
        dir.join("artifact-manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let result = BasketArtifacts::read(&dir);
    std::fs::remove_dir_all(dir).unwrap();
    assert!(
        result.is_ok(),
        "identical reviewed runtime bytes must remain portable: {:?}",
        result.err()
    );
}

#[test]
#[ignore = "requires reviewed pinned artifacts in LABWIRED_RV1103_BASKET_ARTIFACTS"]
fn exact_archive_rejects_mode_body_duplicate_and_concatenated_overlay() {
    let bytes = prepared().artifacts.remove("initramfs.cpio").unwrap();
    validate_archive(&bytes).unwrap();
    for index in [6, 14, 38, 78, 94, 110] {
        let mut changed = bytes.clone();
        changed[index] ^= 1;
        assert!(
            validate_archive(&changed).is_err(),
            "tampered newc byte {index}"
        );
    }
    // Parse offsets from the independently trusted metadata, not guessed
    // header bytes. Exercise actual file payload, complete-entry duplication,
    // reordering and canonical alignment separately from header mutations.
    let pins: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../examples/luckfox-pico-mini-a-linux/basket/trusted-newc.json"
    ))
    .unwrap();
    let mut cursor = 0usize;
    let mut spans = Vec::new();
    let mut first_padding = None;
    for pin in &pins {
        let start = cursor;
        let name_end = start + 110 + pin["fields"][11].as_u64().unwrap() as usize;
        let body_start = (name_end + 3) & !3;
        let body_end = body_start + pin["fields"][6].as_u64().unwrap() as usize;
        let end = (body_end + 3) & !3;
        if first_padding.is_none() {
            first_padding = (name_end < body_start)
                .then_some(name_end)
                .or_else(|| (body_end < end).then_some(body_end));
        }
        if matches!(
            pin["name"].as_str(),
            Some("bin/basket-helper" | "bin/busybox")
        ) {
            assert!(body_end > body_start, "actual regular-file payload");
            let mut changed = bytes.clone();
            changed[body_start + (body_end - body_start) / 2] ^= 1;
            assert!(
                validate_archive(&changed).is_err(),
                "actual file body identity"
            );
        }
        spans.push(start..end);
        cursor = end;
    }
    assert_eq!(cursor, bytes.len());
    let helper_index = pins
        .iter()
        .position(|p| p["name"] == "bin/basket-helper")
        .unwrap();
    let helper = &spans[helper_index];
    let mut duplicate = bytes.clone();
    duplicate.splice(
        helper.end..helper.end,
        bytes[helper.clone()].iter().copied(),
    );
    assert!(
        validate_archive(&duplicate).is_err(),
        "complete valid repeated entry"
    );
    let mut reordered = bytes[..helper.start].to_vec();
    let following = &spans[helper_index + 1];
    reordered.extend_from_slice(&bytes[following.clone()]);
    reordered.extend_from_slice(&bytes[helper.clone()]);
    reordered.extend_from_slice(&bytes[following.end..]);
    assert!(
        validate_archive(&reordered).is_err(),
        "valid entries in wrong order"
    );
    let mut padding = bytes.clone();
    padding[first_padding.expect("actual canonical newc has alignment bytes")] = 1;
    assert!(
        validate_archive(&padding).is_err(),
        "nonzero alignment byte"
    );
    let mut extra = bytes.clone();
    extra.extend(&bytes);
    assert!(validate_archive(&extra).is_err());
    for cut in [0, 109, bytes.len() - 1] {
        assert!(validate_archive(&bytes[..cut]).is_err());
    }
}

#[test]
#[ignore = "requires reviewed pinned artifacts in LABWIRED_RV1103_BASKET_ARTIFACTS"]
fn artifact_and_clock_preflight_fail_without_mutating_any_ram() {
    for corrupted in [false, true] {
        let mut artifacts = prepared();
        let mut b = bus();
        if corrupted {
            artifacts.artifacts.get_mut("initramfs.cpio").unwrap()[120] ^= 1;
        } else {
            b.cpu_hz = 1_200_000_000;
        }
        let before = b.ram.data.clone();
        assert!(load_basket(&mut b, artifacts).is_err());
        assert_eq!(b.ram.data, before);
        assert!(b.irq_fabric.armv7.is_none());
    }
}

#[test]
#[ignore = "requires reviewed pinned artifacts in LABWIRED_RV1103_BASKET_ARTIFACTS"]
fn native_startup_holds_releases_then_initializes_entry_without_rewind_or_ram_erase() {
    let artifacts = prepared();
    let mut b = bus();
    #[cfg(not(feature = "qemu-arm-system"))]
    {
        let before = b.ram.data.clone();
        let error = load_basket(&mut b, artifacts).err().unwrap().to_string();
        assert!(error.contains("native backend unavailable"), "{error}");
        assert_eq!(b.ram.data, before);
        assert!(b.irq_fabric.armv7.is_none());
    }
    #[cfg(all(feature = "qemu-arm-system", feature = "event-scheduler"))]
    {
        use labwired_core::{AdvanceRequest, Cpu, Machine};
        let loaded = load_basket(&mut b, artifacts).unwrap();
        let mut machine = Machine::new(loaded.cpu, b);
        machine.config.idle_fast_forward_enabled = true;
        assert_eq!(machine.cpu.get_pc(), 0);
        assert!(machine.supply_status().routed);
        assert!(machine.supply_status().held_in_reset);
        let ram = sha256(&machine.bus.ram.data);
        let held = machine
            .advance(AdvanceRequest::run(Some(2_400_000)).with_cycle_limit(2_400_000))
            .unwrap();
        assert_eq!(held.primary_steps, 0);
        assert_eq!(held.elapsed_cycles, 2_400_000);
        assert_eq!(machine.total_cycles, 2_400_000);
        machine.set_supply_volts(3.3);
        let released = machine.advance(AdvanceRequest::run(Some(0))).unwrap();
        assert_eq!(released.primary_steps, 0);
        assert_eq!(released.elapsed_cycles, 0);
        assert_eq!(machine.cpu.get_pc(), 0);
        assert_eq!(machine.total_cycles, 2_400_000);
        assert_eq!(machine.supply_status().power_on_resets, 1);
        assert_eq!(machine.supply_status().release_volts, Some(3.0));
        assert_eq!(machine.supply_status().reset_volts, Some(2.7));
        assert_eq!(sha256(&machine.bus.ram.data), ram);
        machine
            .cpu
            .initialize_linux(KERNEL_ADDRESS as u32, DTB_ADDRESS as u32)
            .unwrap();
        assert_eq!(machine.cpu.get_pc(), KERNEL_ADDRESS as u32);
        assert_eq!(machine.cpu.get_register(2), DTB_ADDRESS as u32);
        assert_eq!(machine.total_cycles, 2_400_000);
    }
}

#[test]
fn caller_manifest_cannot_authorize_a_rehashed_archive_or_extra_key() {
    let dir = std::env::temp_dir().join(format!("basket-loader-tamper-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let mut manifest: serde_json::Value = serde_json::from_str(include_str!(
        "../../../examples/luckfox-pico-mini-a-linux/basket/trusted-prepared.json"
    ))
    .unwrap();
    for extra in [false, true] {
        if extra {
            manifest["allow_extra_devices"] = true.into();
        } else {
            manifest["artifacts"]["initramfs.cpio"]["sha256"] = "0".repeat(64).into();
        }
        std::fs::write(
            dir.join("artifact-manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let error = BasketArtifacts::read(&dir)
            .err()
            .expect("tampered profile must fail")
            .to_string();
        assert!(error.contains("trusted prepared profile"), "{error}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "requires reviewed pinned artifacts in LABWIRED_RV1103_BASKET_ARTIFACTS"]
fn closed_fresh_bus_refuses_shadow_flash_or_preexisting_cycle_before_ram_commit() {
    for shadow_flash in [false, true] {
        let mut b = bus();
        if shadow_flash {
            b.flash = LinearMemory::new(4, 0xff840000);
        } else {
            b.set_current_cycle(1);
        }
        let before = sha256(&b.ram.data);
        assert!(
            prepared().validate(&b).is_err(),
            "fresh closed basket bus cannot inherit shadow flash or clock"
        );
        assert_eq!(sha256(&b.ram.data), before);
        assert!(b.irq_fabric.armv7.is_none());
    }
}
