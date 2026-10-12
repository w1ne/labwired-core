use labwired_core::memory::{LinearMemory, Segment};

#[test]
fn segment_address_overflow_preserves_memory() {
    let mut ram = LinearMemory::new(16, 0);
    ram.data.fill(0xa5);
    let before = ram.data.clone();
    assert!(!ram.load_from_segment(&Segment {
        start_addr: u64::MAX - 1,
        data: vec![1, 2, 3]
    }));
    assert_eq!(ram.data, before);
}
#[test]
fn memory_end_overflow_preserves_memory() {
    let mut ram = LinearMemory::new(16, u64::MAX - 7);
    ram.data.fill(0x37);
    let before = ram.data.clone();
    assert!(!ram.load_from_segment(&Segment {
        start_addr: u64::MAX - 7,
        data: vec![1]
    }));
    assert_eq!(ram.data, before);
}

use labwired_core::bus::SystemBus;
use labwired_core::system::rv1103_linux::*;

fn bus() -> SystemBus {
    let mut b = SystemBus::empty();
    b.ram = LinearMemory::new(RAM_BYTES as usize, 0);
    b.ram.data.fill(0xa7);
    b.cpu_hz = CPU_HZ;
    b
}
fn fdt() -> Vec<u8> {
    // Compile a source fixture with the same public preparation toolchain.
    // dtc is an explicit prerequisite for this platform's integration tests.
    use std::process::Command;
    let dir = std::env::temp_dir().join(format!(
        "rv1103-loader-dtc-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir(&dir).unwrap();
    let source = include_str!("../../../examples/luckfox-pico-mini-a-linux/rv1103-direct.dts")
        .replace("@INITRD_END@", "0x03100010");
    std::fs::write(dir.join("test.dts"), source).unwrap();
    let status = Command::new("dtc")
        .args(["-q", "-I", "dts", "-O", "dtb", "-o"])
        .arg(dir.join("test.dtb"))
        .arg(dir.join("test.dts"))
        .status()
        .expect("dtc prerequisite");
    assert!(status.success());
    let data = std::fs::read(dir.join("test.dtb")).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
    data
}
fn fixture() -> LinuxArtifacts {
    let mut kernel = vec![0; 0x100];
    kernel[0x24..0x28].copy_from_slice(&0x016f2818u32.to_le_bytes());
    kernel[0x2c..0x30].copy_from_slice(&0x100u32.to_le_bytes());
    let config = b"CONFIG_MMU=y\nCONFIG_ARM_GIC=y\nCONFIG_ARM_ARCH_TIMER=y\nCONFIG_SERIAL_8250=y\nCONFIG_SERIAL_8250_CONSOLE=y\nCONFIG_SERIAL_8250_DW=y\nCONFIG_BLK_DEV_INITRD=y\n# CONFIG_ARM_LPAE is not set\n".to_vec();
    let dtb = fdt();
    let initramfs = b"0707010000000000".to_vec();
    let manifest = ArtifactManifest {
        schema: 1,
        board: BOARD.into(),
        ram_base: 0,
        ram_bytes: RAM_BYTES,
        timer_hz: TIMER_HZ,
        decompression_start: DECOMPRESSION_START,
        decompression_end: DECOMPRESSION_END,
        bootargs: BOOTARGS.into(),
        artifacts: [
            ("zImage", &kernel, Some(KERNEL_ADDRESS)),
            ("rv1103-direct.dtb", &dtb, Some(DTB_ADDRESS)),
            ("initramfs.cpio", &initramfs, Some(INITRD_ADDRESS)),
            ("kernel.config", &config, None),
        ]
        .into_iter()
        .map(|(name, data, address)| {
            (
                name.into(),
                Artifact {
                    bytes: data.len() as u64,
                    sha256: sha256(data),
                    address,
                },
            )
        })
        .collect(),
    };
    LinuxArtifacts {
        manifest,
        kernel,
        dtb,
        initramfs,
        config,
    }
}
fn rehash(a: &mut LinuxArtifacts, name: &str) {
    let data = match name {
        "zImage" => &a.kernel,
        "rv1103-direct.dtb" => &a.dtb,
        "initramfs.cpio" => &a.initramfs,
        "kernel.config" => &a.config,
        _ => unreachable!(),
    };
    let expected = a.manifest.artifacts.get_mut(name).unwrap();
    expected.bytes = data.len() as u64;
    expected.sha256 = sha256(data);
}
fn rejected(a: LinuxArtifacts, expected: &str) {
    let mut b = bus();
    let before = b.ram.data.clone();
    let e = load_linux(&mut b, a)
        .err()
        .expect("input must be rejected")
        .to_string();
    assert!(e.contains(expected), "expected {expected:?}, got {e:?}");
    assert_eq!(
        b.ram.data, before,
        "rejection must preserve every nonzero RAM byte"
    );
    assert!(b.irq_fabric.armv7.is_none());
}
#[test]
fn complete_staging_validates_compiled_geometry_before_pinned_identity() {
    let a = fixture();
    a.validate(&bus()).unwrap();
    rejected(a, "pinned Alpine");
}
#[test]
fn wrong_board_is_atomic() {
    let mut a = fixture();
    a.manifest.board = "another-board".into();
    rejected(a, "wrong board");
}
#[test]
fn malformed_zimage_is_atomic() {
    let mut a = fixture();
    a.kernel[0x24] ^= 1;
    rehash(&mut a, "zImage");
    rejected(a, "malformed zImage");
}
#[test]
fn artifact_hash_mismatch_is_atomic() {
    let mut a = fixture();
    a.initramfs[9] ^= 1;
    rejected(a, "size/hash mismatch");
}
#[test]
fn wrong_compiled_board_is_atomic() {
    let mut a = fixture();
    let p = a.dtb.windows(6).position(|v| v == b"rv1103").unwrap();
    a.dtb[p + 5] = b'6';
    rehash(&mut a, "rv1103-direct.dtb");
    rejected(a, "wrong DTB /:compatible");
}
#[test]
fn malformed_dtb_is_atomic() {
    let mut a = fixture();
    a.dtb[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
    rehash(&mut a, "rv1103-direct.dtb");
    rejected(a, "DTB structure range");
}
#[test]
fn overlapping_kernel_is_atomic() {
    let mut a = fixture();
    a.kernel.resize(0x01000001, 0);
    let n = a.kernel.len() as u32;
    a.kernel[0x2c..0x30].copy_from_slice(&n.to_le_bytes());
    rehash(&mut a, "zImage");
    rejected(a, "overlapping");
}
#[test]
fn oversized_initramfs_is_atomic() {
    let mut a = fixture();
    a.initramfs
        .resize((RAM_BYTES - INITRD_ADDRESS + 1) as usize, 0);
    rehash(&mut a, "initramfs.cpio");
    rejected(a, "outside RAM");
}
#[test]
fn wrong_config_is_atomic() {
    let mut a = fixture();
    a.config = b"CONFIG_ARM_LPAE=y".to_vec();
    rehash(&mut a, "kernel.config");
    rejected(a, "missing builtin");
}
#[test]
fn checked_reservations_reject_address_overflow() {
    assert!(checked_range(u64::MAX, 1, 0, u64::MAX).is_err());
    assert!(checked_range(8, 1, u64::MAX, 8).is_err());
}
#[test]
fn dtb_requires_final_exact_initrd_end() {
    let a = fixture();
    assert!(validate_dtb(&a.dtb, INITRD_ADDRESS + 17)
        .unwrap_err()
        .to_string()
        .contains("initrd-end"));
}
#[test]
#[ignore = "requires independently prepared pinned Alpine artifacts in LABWIRED_RV1103_ARTIFACTS"]
fn pinned_artifacts_load_or_backend_refusal_is_atomic() {
    let dir =
        std::env::var("LABWIRED_RV1103_ARTIFACTS").expect("artifact preparation prerequisite");
    let artifacts = LinuxArtifacts::read(std::path::Path::new(&dir)).unwrap();
    let mut b = bus();
    let before = b.ram.data.clone();
    #[cfg(not(feature = "qemu-arm-system"))]
    {
        let e = load_linux(&mut b, artifacts).err().unwrap().to_string();
        assert!(e.contains("native backend unavailable"), "{e}");
        assert_eq!(b.ram.data, before);
    }
    #[cfg(feature = "qemu-arm-system")]
    {
        use labwired_core::Cpu;
        let mut expected = before;
        for (address, bytes) in [
            (KERNEL_ADDRESS, &artifacts.kernel),
            (DTB_ADDRESS, &artifacts.dtb),
            (INITRD_ADDRESS, &artifacts.initramfs),
        ] {
            let start = address as usize;
            expected[start..start + bytes.len()].copy_from_slice(bytes);
        }
        let loaded = load_linux(&mut b, artifacts).unwrap();
        assert_eq!(loaded.cpu.get_register(0), 0);
        assert_eq!(loaded.cpu.get_register(1), u32::MAX);
        assert_eq!(loaded.cpu.get_register(2), DTB_ADDRESS as u32);
        assert_eq!(loaded.cpu.get_register(16), 0xd3);
        assert_eq!(loaded.cpu.get_pc(), KERNEL_ADDRESS as u32);
        assert_eq!(
            b.ram.data, expected,
            "only the exact preflighted artifact ranges may change"
        );
        assert!(b.irq_fabric.armv7.is_some());
    }
}

#[test]
fn truncated_dtb_structure_never_panics_or_changes_ram() {
    // Root node token/name, then partial property length/name-offset/data.
    for size in [0u32, 1, 2, 3, 4, 5, 7, 8, 9, 10, 11, 12, 13, 15, 16, 20] {
        let mut a = fixture();
        a.dtb[36..40].copy_from_slice(&size.to_be_bytes());
        rehash(&mut a, "rv1103-direct.dtb");
        let mut b = bus();
        let before = b.ram.data.clone();
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| load_linux(&mut b, a)));
        assert!(
            result.is_ok(),
            "malformed structure size {size} must return an error, never panic"
        );
        assert!(
            result.unwrap().is_err(),
            "truncated structure size {size} must be rejected"
        );
        assert_eq!(
            b.ram.data, before,
            "truncated structure size {size} preserves all RAM"
        );
    }
}

#[test]
fn reserved_phandles_with_matching_references_are_atomic() {
    for (node, reference, property, values) in [
        (
            "/clock24m",
            "/serial@ff4c0000",
            "clocks",
            vec!["ffffffff", "ffffffff"],
        ),
        (
            "/interrupt-controller@ff1f1000",
            "/",
            "interrupt-parent",
            vec!["ffffffff"],
        ),
    ] {
        let mut a = fixture();
        let path = std::env::temp_dir().join(format!(
            "rv1103-phandle-{}-{:?}.dtb",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::write(&path, &a.dtb).unwrap();
        for (target, prop, cells) in [
            (node, "phandle", vec!["ffffffff"]),
            (reference, property, values),
        ] {
            assert!(std::process::Command::new("fdtput")
                .args(["-t", "x"])
                .arg(&path)
                .args([target, prop])
                .args(cells)
                .status()
                .expect("fdtput prerequisite")
                .success());
        }
        a.dtb = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        rehash(&mut a, "rv1103-direct.dtb");
        assert!(
            validate_dtb(&a.dtb, INITRD_ADDRESS + 16).is_err(),
            "reserved phandle must be rejected even with matching {property}"
        );
        let mut b = bus();
        let before = b.ram.data.clone();
        let error = load_linux(&mut b, a).err().unwrap().to_string();
        assert!(error.contains("phandles"), "{error}");
        assert_eq!(b.ram.data, before);
    }
}
