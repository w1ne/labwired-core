//! Strict reduced direct-boot loader. All input and native initialization are
//! fallible before the first write to caller-owned RAM.
use crate::bus::SystemBus;
#[cfg(feature = "qemu-arm-system")]
use crate::memory::Segment;
use crate::{SimResult, SimulationError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

pub const BOARD: &str = "luckfox-pico-mini-a-rv1103";
pub const RAM_BYTES: u64 = 0x0400_0000;
pub const KERNEL_ADDRESS: u64 = 0x0200_0000;
pub const DTB_ADDRESS: u64 = 0x0300_0000;
pub const INITRD_ADDRESS: u64 = 0x0310_0000;
pub const DECOMPRESSION_START: u64 = 0x8000;
pub const DECOMPRESSION_END: u64 = 0x016e_5850;
pub const CPU_HZ: u64 = 1_200_000_000;
pub const TIMER_HZ: u64 = 24_000_000;
pub const BACKEND_SOURCE_REVISION: &str = "167decf9129758829762e582939128c0694e90d6";
pub const BACKEND_SOURCE_ARCHIVE_SHA256: &str =
    "8db4d1fe0a316eec5dcae83bd4d2757b5172332493d6fdecfd9d4b81cb7a77a3";
pub fn compiled_backend_sha256() -> Option<String> {
    #[cfg(feature = "qemu-arm-system")]
    {
        labwired_arm_system::compiled_backend_sha256()
    }
    #[cfg(not(feature = "qemu-arm-system"))]
    {
        None
    }
}
pub const KERNEL_SHA256: &str = "eac9582162e912bdb577bba6ea224d98a29c997e52ee891383ea836aa704680a";
pub const INITRAMFS_SHA256: &str =
    "b13962b76edb327b89cec464014cd2565694d88a7f0f86ccc361317cf879ccce";
pub const INITRAMFS_BYTES: u64 = 889_544;
pub const INIT_SHA256: &str = "5d1d1e1cd5d6b0e37ee352feeef4d998548d1ecd98ffee0d67a0b797c044663c";
pub const CONFIG_SHA256: &str = "4e64812402b287e13fd7513aa830432782a1ae94a5461bfe579999d1e18432fb";
pub const BOOTARGS: &str = "mem=64M cma=0 maxcpus=1 earlycon=uart8250,mmio32,0xff4c0000,115200n8 console=ttyS2,115200n8 rdinit=/init loglevel=8 panic=-1 clocksource.arm_arch_timer.evtstrm=0";
fn invalid(message: impl Into<String>) -> SimulationError {
    SimulationError::Other(format!("RV1103 Linux: {}", message.into()))
}
pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Artifact {
    pub bytes: u64,
    pub sha256: String,
    pub address: Option<u64>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ArtifactManifest {
    pub schema: u32,
    pub board: String,
    pub ram_base: u64,
    pub ram_bytes: u64,
    pub timer_hz: u64,
    pub decompression_start: u64,
    pub decompression_end: u64,
    pub bootargs: String,
    pub artifacts: BTreeMap<String, Artifact>,
}
impl ArtifactManifest {
    fn geometry(&self) -> SimResult<()> {
        if self.schema != 1 || self.board != BOARD {
            return Err(invalid("wrong board/schema"));
        }
        if self.ram_base != 0
            || self.ram_bytes != RAM_BYTES
            || self.timer_hz != TIMER_HZ
            || self.decompression_start != DECOMPRESSION_START
            || self.decompression_end != DECOMPRESSION_END
            || self.bootargs != BOOTARGS
        {
            return Err(invalid("wrong platform geometry or bootargs"));
        }
        for (name, address) in [
            ("zImage", Some(KERNEL_ADDRESS)),
            ("rv1103-direct.dtb", Some(DTB_ADDRESS)),
            ("initramfs.cpio", Some(INITRD_ADDRESS)),
            ("kernel.config", None),
        ] {
            let a = self
                .artifacts
                .get(name)
                .ok_or_else(|| invalid(format!("missing {name}")))?;
            if a.address != address {
                return Err(invalid(format!("wrong {name} address")));
            }
            if a.sha256.len() != 64
                || !a
                    .sha256
                    .bytes()
                    .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
            {
                return Err(invalid("malformed SHA256"));
            }
        }
        Ok(())
    }
    fn pins(&self) -> SimResult<()> {
        if self.artifacts["zImage"].bytes != 8_524_288
            || self.artifacts["zImage"].sha256 != KERNEL_SHA256
            || self.artifacts["kernel.config"].bytes != 269_493
            || self.artifacts["kernel.config"].sha256 != CONFIG_SHA256
        {
            return Err(invalid(
                "kernel/config differs from pinned Alpine artifacts",
            ));
        }
        if self.artifacts["initramfs.cpio"].bytes != INITRAMFS_BYTES
            || self.artifacts["initramfs.cpio"].sha256 != INITRAMFS_SHA256
        {
            return Err(invalid(
                "initramfs differs from deterministic pinned Busybox/init fixture",
            ));
        }
        Ok(())
    }
}
/// In-memory input supports validation without touching the bus. Public loaders
/// still require pinned kernel/config identities before native creation.
pub struct LinuxArtifacts {
    pub manifest: ArtifactManifest,
    pub kernel: Vec<u8>,
    pub dtb: Vec<u8>,
    pub initramfs: Vec<u8>,
    pub config: Vec<u8>,
}
impl LinuxArtifacts {
    pub fn read(directory: &Path) -> SimResult<Self> {
        fn read(path: &Path, limit: u64) -> SimResult<Vec<u8>> {
            use std::io::Read;
            let metadata = std::fs::symlink_metadata(path)
                .map_err(|e| invalid(format!("{}: {e}", path.display())))?;
            if !metadata.is_file() || metadata.len() > limit {
                return Err(invalid("nonregular or oversized artifact"));
            }
            let mut bytes = Vec::new();
            std::fs::File::open(path)
                .map_err(|e| invalid(e.to_string()))?
                .take(limit + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| invalid(e.to_string()))?;
            if bytes.len() as u64 > limit {
                return Err(invalid("artifact grew beyond bound"));
            }
            Ok(bytes)
        }
        let manifest: ArtifactManifest =
            serde_json::from_slice(&read(&directory.join("artifact-manifest.json"), 65536)?)
                .map_err(|e| invalid(e.to_string()))?;
        manifest.geometry()?;
        manifest.pins()?;
        for name in ["init", "rv1103-direct.dts"] {
            let expected = manifest
                .artifacts
                .get(name)
                .ok_or_else(|| invalid(format!("missing source {name}")))?;
            let content = read(&directory.join(name), 65536)?;
            if expected.address.is_some()
                || expected.bytes != content.len() as u64
                || expected.sha256 != sha256(&content)
            {
                return Err(invalid(format!("{name} source size/hash mismatch")));
            }
            if name == "init" && sha256(&content) != INIT_SHA256 {
                return Err(invalid("init differs from acceptance fixture"));
            }
        }
        Ok(Self {
            manifest,
            kernel: read(&directory.join("zImage"), 8_524_288)?,
            dtb: read(&directory.join("rv1103-direct.dtb"), 65536)?,
            initramfs: read(
                &directory.join("initramfs.cpio"),
                RAM_BYTES - INITRD_ADDRESS,
            )?,
            config: read(&directory.join("kernel.config"), 269_493)?,
        })
    }
    /// Validate every hash, header, DT property and reservation, with no writes.
    pub fn validate(&self, bus: &SystemBus) -> SimResult<()> {
        self.manifest.geometry()?;
        if bus.ram.base_addr != 0 || bus.ram.data.len() as u64 != RAM_BYTES || bus.cpu_hz == 0 {
            return Err(invalid(
                "RAM must be DDR base zero, exactly 64 MiB; CPU Hz must be nonzero",
            ));
        }
        if bus.irq_fabric.armv7.is_some() {
            return Err(invalid("ARM fabric already installed"));
        }
        for (name, bytes) in [
            ("zImage", &self.kernel),
            ("rv1103-direct.dtb", &self.dtb),
            ("initramfs.cpio", &self.initramfs),
            ("kernel.config", &self.config),
        ] {
            let expected = &self.manifest.artifacts[name];
            if expected.bytes != bytes.len() as u64 || sha256(bytes) != expected.sha256 {
                return Err(invalid(format!("{name} size/hash mismatch")));
            }
        }
        validate_zimage(&self.kernel)?;
        validate_config(&self.config)?;
        let decompression_length = DECOMPRESSION_END
            .checked_sub(DECOMPRESSION_START)
            .ok_or_else(|| invalid("decompression reservation underflow"))?;
        let decompression_end = checked_range(
            DECOMPRESSION_START,
            decompression_length,
            bus.ram.base_addr,
            bus.ram.data.len() as u64,
        )?;
        let mut ranges = vec![(DECOMPRESSION_START, decompression_end)];
        for (address, bytes) in [
            (KERNEL_ADDRESS, &self.kernel),
            (DTB_ADDRESS, &self.dtb),
            (INITRD_ADDRESS, &self.initramfs),
        ] {
            let end = checked_range(
                address,
                bytes.len() as u64,
                bus.ram.base_addr,
                bus.ram.data.len() as u64,
            )?;
            if ranges
                .iter()
                .any(|&(start, stop)| address < stop && start < end)
            {
                return Err(invalid("overlapping boot/decompression reservations"));
            }
            ranges.push((address, end));
        }
        if self.dtb.len() > 65536
            || self.initramfs.is_empty()
            || !self.initramfs.starts_with(b"070701")
        {
            return Err(invalid("invalid DTB/initramfs size or format"));
        }
        validate_dtb(&self.dtb, INITRD_ADDRESS + self.initramfs.len() as u64)?;
        Ok(())
    }
}
pub fn checked_range(start: u64, size: u64, base: u64, length: u64) -> SimResult<u64> {
    let end = start
        .checked_add(size)
        .ok_or_else(|| invalid("artifact address overflow"))?;
    let stop = base
        .checked_add(length)
        .ok_or_else(|| invalid("RAM address overflow"))?;
    if start < base || end > stop {
        return Err(invalid("artifact outside RAM"));
    }
    Ok(end)
}
pub fn validate_zimage(bytes: &[u8]) -> SimResult<()> {
    if bytes.len() < 0x30 {
        return Err(invalid("truncated zImage header"));
    }
    let word = |n| u32::from_le_bytes(bytes[n..n + 4].try_into().unwrap());
    if word(0x24) != 0x016f2818 || word(0x28) != 0 || u64::from(word(0x2c)) != bytes.len() as u64 {
        return Err(invalid("malformed zImage magic/start/end"));
    }
    Ok(())
}
pub fn validate_config(bytes: &[u8]) -> SimResult<()> {
    let text = std::str::from_utf8(bytes).map_err(|_| invalid("non UTF8 kernel config"))?;
    for name in [
        "MMU",
        "ARM_GIC",
        "ARM_ARCH_TIMER",
        "SERIAL_8250",
        "SERIAL_8250_CONSOLE",
        "SERIAL_8250_DW",
        "BLK_DEV_INITRD",
    ] {
        if !text.lines().any(|line| line == format!("CONFIG_{name}=y")) {
            return Err(invalid(format!("missing builtin CONFIG_{name}")));
        }
    }
    if !text
        .lines()
        .any(|line| line == "# CONFIG_ARM_LPAE is not set")
    {
        return Err(invalid("LPAE must be disabled"));
    }
    Ok(())
}

// Strict FDT parser: ranges, tokens, node/property uniqueness and required
// compiled values are checked; a magic-only check cannot qualify a board.
type Tree = BTreeMap<String, BTreeMap<String, Vec<u8>>>;
fn word(bytes: &[u8], offset: usize) -> SimResult<u32> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| invalid("DTB offset overflow"))?;
    Ok(u32::from_be_bytes(
        bytes
            .get(offset..end)
            .ok_or_else(|| invalid("truncated DTB"))?
            .try_into()
            .unwrap(),
    ))
}
fn terminated(bytes: &[u8]) -> SimResult<(&str, usize)> {
    let end = bytes
        .iter()
        .position(|v| *v == 0)
        .ok_or_else(|| invalid("unterminated DTB string"))?;
    Ok((
        std::str::from_utf8(&bytes[..end]).map_err(|_| invalid("non UTF8 DTB string"))?,
        end + 1,
    ))
}
pub(crate) fn tree(bytes: &[u8]) -> SimResult<Tree> {
    if bytes.len() < 40
        || word(bytes, 0)? != 0xd00dfeed
        || word(bytes, 4)? as usize != bytes.len()
        || word(bytes, 20)? != 17
        || word(bytes, 24)? > 17
        || word(bytes, 28)? != 0
    {
        return Err(invalid("malformed DTB header"));
    }
    let structure = word(bytes, 8)? as usize;
    let strings = word(bytes, 12)? as usize;
    let reserve = word(bytes, 16)? as usize;
    let slen = word(bytes, 36)? as usize;
    let nlen = word(bytes, 32)? as usize;
    let send = structure
        .checked_add(slen)
        .filter(|v| *v <= bytes.len())
        .ok_or_else(|| invalid("DTB structure range"))?;
    let nend = strings
        .checked_add(nlen)
        .filter(|v| *v <= bytes.len())
        .ok_or_else(|| invalid("DTB strings range"))?;
    if structure < 40
        || structure % 4 != 0
        || slen % 4 != 0
        || strings < send
        || reserve < 40
        || reserve % 8 != 0
        || reserve.checked_add(16).is_none_or(|v| v > structure)
        || nend != bytes.len()
    {
        return Err(invalid("DTB block overlap/alignment"));
    }
    // This reduced fixture contains no memory reservations; dynamic reservation
    // changes must be reflected in and qualified against the loader layout.
    if bytes
        .get(reserve..structure)
        .ok_or_else(|| invalid("DTB reservation range"))?
        .iter()
        .any(|v| *v != 0)
    {
        return Err(invalid("unsupported DTB reservations"));
    }
    let names = &bytes[strings..nend];
    // Token/header reads must never borrow bytes from the strings or padding
    // beyond the declared structure block, even when the full file is valid.
    let structure_bytes = &bytes[structure..send];
    let mut cursor = 0;
    let mut nodes = Tree::new();
    let mut stack: Vec<String> = Vec::new();
    loop {
        if cursor >= structure_bytes.len() {
            return Err(invalid("missing DTB end token"));
        }
        let token = word(structure_bytes, cursor)?;
        cursor += 4;
        match token {
            1 => {
                let (name, length) = terminated(&structure_bytes[cursor..])?;
                if name.contains('/')
                    || (stack.is_empty() && (!name.is_empty() || !nodes.is_empty()))
                {
                    return Err(invalid("invalid DTB node name"));
                }
                cursor = cursor
                    .checked_add(length)
                    .and_then(|v| v.checked_add(3))
                    .ok_or_else(|| invalid("DTB node alignment overflow"))?
                    & !3;
                if cursor > structure_bytes.len() {
                    return Err(invalid("DTB node padding range"));
                }
                let path = if stack.is_empty() {
                    "/".into()
                } else {
                    format!(
                        "{}{name}",
                        if stack.last().unwrap() == "/" {
                            "/".into()
                        } else {
                            format!("{}/", stack.last().unwrap())
                        }
                    )
                };
                if nodes.insert(path.clone(), BTreeMap::new()).is_some() {
                    return Err(invalid("duplicate DTB node"));
                }
                stack.push(path);
            }
            2 => {
                if stack.pop().is_none() {
                    return Err(invalid("unbalanced DTB nodes"));
                }
            }
            3 => {
                let length = word(structure_bytes, cursor)? as usize;
                let nameoff = word(
                    structure_bytes,
                    cursor
                        .checked_add(4)
                        .ok_or_else(|| invalid("DTB property offset overflow"))?,
                )? as usize;
                cursor += 8;
                let (name, _) = terminated(
                    names
                        .get(nameoff..)
                        .ok_or_else(|| invalid("DTB property name offset"))?,
                )?;
                let end = cursor
                    .checked_add(length)
                    .filter(|v| *v <= structure_bytes.len())
                    .ok_or_else(|| invalid("DTB property range"))?;
                let path = stack
                    .last()
                    .ok_or_else(|| invalid("DTB property outside node"))?;
                if nodes
                    .get_mut(path)
                    .unwrap()
                    .insert(name.into(), structure_bytes[cursor..end].to_vec())
                    .is_some()
                {
                    return Err(invalid("duplicate DTB property"));
                }
                cursor = end
                    .checked_add(3)
                    .ok_or_else(|| invalid("DTB property alignment overflow"))?
                    & !3;
                if cursor > structure_bytes.len() {
                    return Err(invalid("DTB property padding range"));
                }
            }
            4 => {}
            9 => {
                if !stack.is_empty() || cursor != structure_bytes.len() {
                    return Err(invalid("unbalanced/trailing DTB structure"));
                }
                return Ok(nodes);
            }
            _ => return Err(invalid("unknown DTB token")),
        }
    }
}
pub fn validate_dtb(bytes: &[u8], initrd_end: u64) -> SimResult<()> {
    let nodes = tree(bytes)?;
    fn prop<'a>(nodes: &'a Tree, path: &str, name: &str) -> SimResult<&'a [u8]> {
        nodes
            .get(path)
            .and_then(|n| n.get(name))
            .map(Vec::as_slice)
            .ok_or_else(|| invalid(format!("missing DTB {path}:{name}")))
    }
    fn equal(nodes: &Tree, path: &str, name: &str, expected: &[u8]) -> SimResult<()> {
        if prop(nodes, path, name)? != expected {
            return Err(invalid(format!("wrong DTB {path}:{name}")));
        }
        Ok(())
    }
    fn cells(nodes: &Tree, path: &str, name: &str, expected: &[u32]) -> SimResult<()> {
        let bytes: Vec<u8> = expected.iter().flat_map(|v| v.to_be_bytes()).collect();
        equal(nodes, path, name, &bytes)
    }
    let allowed = [
        "/",
        "/aliases",
        "/chosen",
        "/memory@0",
        "/cpus",
        "/cpus/cpu@0",
        "/interrupt-controller@ff1f1000",
        "/timer",
        "/clock24m",
        "/serial@ff4c0000",
    ];
    if nodes.len() != allowed.len() || nodes.keys().any(|p| !allowed.contains(&p.as_str())) {
        return Err(invalid("unsupported DTB devices"));
    }
    for (path, properties) in &nodes {
        let allowed: &[&str] = match path.as_str() {
            "/" => &[
                "model",
                "compatible",
                "#address-cells",
                "#size-cells",
                "interrupt-parent",
            ],
            "/aliases" => &["serial2"],
            "/chosen" => &[
                "stdout-path",
                "bootargs",
                "linux,initrd-start",
                "linux,initrd-end",
            ],
            "/memory@0" => &["device_type", "reg"],
            "/cpus" => &["#address-cells", "#size-cells"],
            "/cpus/cpu@0" => &["device_type", "compatible", "reg"],
            "/interrupt-controller@ff1f1000" => &[
                "compatible",
                "#interrupt-cells",
                "#address-cells",
                "interrupt-controller",
                "reg",
                "phandle",
            ],
            "/timer" => &["compatible", "interrupts", "clock-frequency"],
            "/clock24m" => &["compatible", "#clock-cells", "clock-frequency", "phandle"],
            "/serial@ff4c0000" => &[
                "compatible",
                "reg",
                "interrupts",
                "reg-shift",
                "reg-io-width",
                "clock-frequency",
                "clocks",
                "clock-names",
                "status",
            ],
            _ => unreachable!(),
        };
        if properties
            .keys()
            .any(|key| !allowed.contains(&key.as_str()))
        {
            return Err(invalid("unsupported DTB properties"));
        }
    }
    equal(&nodes, "/", "compatible", b"rockchip,rv1103\0")?;
    cells(&nodes, "/", "#address-cells", &[1])?;
    cells(&nodes, "/", "#size-cells", &[1])?;
    equal(&nodes, "/aliases", "serial2", b"/serial@ff4c0000\0")?;
    equal(&nodes, "/chosen", "stdout-path", b"serial2:115200n8\0")?;
    equal(
        &nodes,
        "/chosen",
        "bootargs",
        format!("{BOOTARGS}\0").as_bytes(),
    )?;
    cells(
        &nodes,
        "/chosen",
        "linux,initrd-start",
        &[INITRD_ADDRESS as u32],
    )?;
    cells(
        &nodes,
        "/chosen",
        "linux,initrd-end",
        &[u32::try_from(initrd_end).map_err(|_| invalid("initrd end overflow"))?],
    )?;
    equal(&nodes, "/memory@0", "device_type", b"memory\0")?;
    cells(&nodes, "/memory@0", "reg", &[0, RAM_BYTES as u32])?;
    cells(&nodes, "/cpus", "#address-cells", &[1])?;
    cells(&nodes, "/cpus", "#size-cells", &[0])?;
    equal(&nodes, "/cpus/cpu@0", "device_type", b"cpu\0")?;
    equal(&nodes, "/cpus/cpu@0", "compatible", b"arm,cortex-a7\0")?;
    cells(&nodes, "/cpus/cpu@0", "reg", &[0])?;
    let gic = "/interrupt-controller@ff1f1000";
    equal(&nodes, gic, "compatible", b"arm,gic-400\0")?;
    cells(&nodes, gic, "#interrupt-cells", &[3])?;
    equal(&nodes, gic, "interrupt-controller", b"")?;
    cells(
        &nodes,
        gic,
        "reg",
        &[0xff1f1000, 0x1000, 0xff1f2000, 0x2000],
    )?;
    equal(
        &nodes,
        "/",
        "interrupt-parent",
        prop(&nodes, gic, "phandle")?,
    )?;
    equal(&nodes, "/timer", "compatible", b"arm,armv7-timer\0")?;
    cells(
        &nodes,
        "/timer",
        "interrupts",
        &[1, 13, 0x104, 1, 14, 0x104],
    )?;
    cells(&nodes, "/timer", "clock-frequency", &[TIMER_HZ as u32])?;
    equal(&nodes, "/clock24m", "compatible", b"fixed-clock\0")?;
    cells(&nodes, "/clock24m", "#clock-cells", &[0])?;
    cells(&nodes, "/clock24m", "clock-frequency", &[TIMER_HZ as u32])?;
    let uart = "/serial@ff4c0000";
    equal(&nodes, uart, "compatible", b"snps,dw-apb-uart\0")?;
    cells(&nodes, uart, "reg", &[0xff4c0000, 0x100])?;
    cells(&nodes, uart, "interrupts", &[0, 27, 4])?;
    cells(&nodes, uart, "reg-shift", &[2])?;
    cells(&nodes, uart, "reg-io-width", &[4])?;
    cells(&nodes, uart, "clock-frequency", &[TIMER_HZ as u32])?;
    equal(&nodes, uart, "status", b"okay\0")?;
    let cp = prop(&nodes, "/clock24m", "phandle")?;
    let gp = prop(&nodes, gic, "phandle")?;
    if cp.len() != 4
        || gp.len() != 4
        || cp == gp
        || cp == [0; 4]
        || gp == [0; 4]
        || cp == [0xff; 4]
        || gp == [0xff; 4]
    {
        return Err(invalid("invalid DTB phandles"));
    }
    let clock = word(cp, 0)?;
    cells(&nodes, uart, "clocks", &[clock, clock])?;
    equal(&nodes, uart, "clock-names", b"baudclk\0apb_pclk\0")?;
    Ok(())
}
#[cfg(feature = "qemu-arm-system")]
pub struct LoadedLinux {
    pub cpu: crate::cpu::CortexA7System,
    pub manifest: ArtifactManifest,
}
#[cfg(not(feature = "qemu-arm-system"))]
pub struct LoadedLinux {
    pub manifest: ArtifactManifest,
}
/// Validation and native CPU creation precede the sole RAM commit. The
/// preflighted copies are infallible, and no incremental legacy loader runs.
pub fn load_linux(bus: &mut SystemBus, artifacts: LinuxArtifacts) -> SimResult<LoadedLinux> {
    artifacts.validate(bus)?;
    artifacts.manifest.pins()?;
    #[cfg(not(feature = "qemu-arm-system"))]
    {
        Err(invalid(
            "native backend unavailable; enable qemu-arm-system",
        ))
    }
    #[cfg(feature = "qemu-arm-system")]
    {
        let mut cpu = crate::cpu::CortexA7System::new()?;
        cpu.executable_range(0, RAM_BYTES)?;
        cpu.initialize_linux(KERNEL_ADDRESS as u32, DTB_ADDRESS as u32)?;
        // configure is preflighted above: no existing ARM fabric and Hz > 0.
        bus.configure_armv7_rv1103()?;
        for (address, bytes) in [
            (KERNEL_ADDRESS, artifacts.kernel),
            (DTB_ADDRESS, artifacts.dtb),
            (INITRD_ADDRESS, artifacts.initramfs),
        ] {
            let segment = Segment {
                start_addr: address,
                data: bytes,
            };
            assert!(
                bus.ram.load_from_segment(&segment),
                "preflighted boot range"
            );
        }
        Ok(LoadedLinux {
            cpu,
            manifest: artifacts.manifest,
        })
    }
}
