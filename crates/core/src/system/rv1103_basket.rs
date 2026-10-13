//! Closed reviewed Linux basket fixture. This is a synthetic execution/power
//! proxy, not a physical RV1103 power-threshold model. The base loader is separate.
use super::rv1103_linux::*;
use crate::{bus::SystemBus, SimResult, SimulationError};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};

pub const FUNCTIONAL_CPU_HZ: u64 = 24_000_000;
pub const PROFILE: &str = "rv1103-linux-basket-v1";
pub const TRUSTED_MANIFEST_SHA256: &str =
    "73688dfd91cf2786f8b2a2e4b10fde8c86d197c8c9d74e18fcf71d8c10e66426";
const TRUSTED_MANIFEST: &str =
    include_str!("../../../../examples/luckfox-pico-mini-a-linux/basket/trusted-prepared.json");
const TRUSTED_NEWC: &str =
    include_str!("../../../../examples/luckfox-pico-mini-a-linux/basket/trusted-newc.json");
pub const CANONICAL_MANIFEST: &str = include_str!(
    "../../../../examples/luckfox-pico-mini-a-linux/basket-native-runtime/system.json"
);
pub const CANONICAL_EXPORT: &str = include_str!(
    "../../../../examples/luckfox-pico-mini-a-linux/basket-native-runtime/receipt.json"
);

fn invalid(message: impl Into<String>) -> SimulationError {
    SimulationError::Other(format!("RV1103 basket: {}", message.into()))
}
fn regular_read(path: &Path, limit: u64) -> SimResult<Vec<u8>> {
    use std::io::Read;
    let meta = std::fs::symlink_metadata(path).map_err(|e| invalid(e.to_string()))?;
    if !meta.is_file() || meta.len() > limit {
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
fn observation_log(name: &str) -> bool {
    matches!(
        name,
        "build-a.log" | "build-b.log" | "build-a-dtc.log" | "build-b-dtc.log"
    )
}
fn same_shape(actual: &Value, expected: &Value) -> bool {
    match (actual, expected) {
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && b.iter()
                    .all(|(k, v)| a.get(k).is_some_and(|x| same_shape(x, v)))
        }
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same_shape(x, y))
        }
        (Value::String(s), Value::String(_)) => s.len() <= 8192,
        (Value::Number(_), Value::Number(_))
        | (Value::Bool(_), Value::Bool(_))
        | (Value::Null, Value::Null) => true,
        _ => false,
    }
}
fn validate_manifest(actual: &Value, trusted: &Value) -> SimResult<()> {
    if sha256(TRUSTED_MANIFEST.as_bytes()) != TRUSTED_MANIFEST_SHA256
        || !same_shape(actual, trusted)
    {
        return Err(invalid(
            "manifest differs from trusted prepared profile keys/bounds",
        ));
    }
    let stable = |v: &Value| {
        let mut v = v.clone();
        v.as_object_mut().unwrap().remove("builds");
        v["artifacts"]
            .as_object_mut()
            .unwrap()
            .retain(|k, _| !observation_log(k));
        v
    };
    if stable(actual) != stable(trusted) {
        return Err(invalid("manifest differs from trusted prepared profile"));
    }
    for (a, t) in actual["builds"]
        .as_array()
        .unwrap()
        .iter()
        .zip(trusted["builds"].as_array().unwrap())
    {
        for key in ["source", "generated_header", "output", "dtb", "initramfs"] {
            if a[key] != t[key] {
                return Err(invalid(
                    "build output differs from trusted prepared profile",
                ));
            }
        }
    }
    for (name, pin) in actual["artifacts"].as_object().unwrap() {
        if observation_log(name) {
            let digest = pin["sha256"].as_str().ok_or_else(|| invalid("log hash"))?;
            if pin["bytes"].as_u64().is_none_or(|n| n > 65536)
                || !pin["address"].is_null()
                || digest.len() != 64
                || !digest
                    .bytes()
                    .all(|n| n.is_ascii_digit() || (b'a'..=b'f').contains(&n))
            {
                return Err(invalid("observed build log bounds"));
            }
        }
    }
    Ok(())
}
pub struct BasketArtifacts {
    pub manifest: Value,
    pub artifacts: BTreeMap<String, Vec<u8>>,
}
impl BasketArtifacts {
    /// Closed stable profile and immutable artifacts authorize loading. Bounded
    /// preparation paths/tool captures/logs are observations, not authorization;
    /// independently prepared byte-identical runtime artifacts remain portable.
    pub fn read(directory: &Path) -> SimResult<Self> {
        let raw = regular_read(&directory.join("artifact-manifest.json"), 65536)?;
        let manifest: Value = serde_json::from_slice(&raw).map_err(|e| invalid(e.to_string()))?;
        let trusted: Value =
            serde_json::from_str(TRUSTED_MANIFEST).expect("reviewed manifest JSON");
        validate_manifest(&manifest, &trusted)?;
        let mut artifacts = BTreeMap::new();
        for (name, pin) in trusted["artifacts"].as_object().expect("trusted artifacts") {
            let pin = if observation_log(name) {
                &manifest["artifacts"][name]
            } else {
                pin
            };
            let expected: Artifact =
                serde_json::from_value(pin.clone()).map_err(|e| invalid(e.to_string()))?;
            let bytes = regular_read(&directory.join(name), expected.bytes)?;
            if bytes.len() as u64 != expected.bytes || sha256(&bytes) != expected.sha256 {
                return Err(invalid(format!(
                    "{name} differs from trusted artifact identity"
                )));
            }
            artifacts.insert(name.clone(), bytes);
        }
        let staged = Self {
            manifest,
            artifacts,
        };
        staged.validate_content()?;
        Ok(staged)
    }
    fn validate_content(&self) -> SimResult<()> {
        let trusted: Value = serde_json::from_str(TRUSTED_MANIFEST).expect("trusted manifest");
        validate_manifest(&self.manifest, &trusted)?;
        if self.artifacts.len() != trusted["artifacts"].as_object().unwrap().len() {
            return Err(invalid("manifest differs from trusted prepared profile"));
        }
        for (name, pin) in trusted["artifacts"].as_object().unwrap() {
            let pin = if observation_log(name) {
                &self.manifest["artifacts"][name]
            } else {
                pin
            };
            let bytes = self
                .artifacts
                .get(name)
                .ok_or_else(|| invalid(format!("missing {name}")))?;
            if Some(bytes.len() as u64) != pin["bytes"].as_u64()
                || Some(sha256(bytes).as_str()) != pin["sha256"].as_str()
            {
                return Err(invalid(format!(
                    "{name} differs from trusted artifact identity"
                )));
            }
        }
        validate_zimage(&self.artifacts["zImage"])?;
        validate_config(&self.artifacts["kernel.config"])?;
        validate_archive(&self.artifacts["initramfs.cpio"])?;
        let tree = super::rv1103_linux::tree(&self.artifacts["rv1103-basket.dtb"])?;
        let paths = [
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
            "/serial@ff4e0000",
            "/i2c@ff460000",
        ];
        if tree.len() != paths.len() || tree.keys().any(|p| !paths.contains(&p.as_str())) {
            return Err(invalid(
                "basket DT nodes differ from closed twelve-node profile",
            ));
        }
        let end = checked_range(
            INITRD_ADDRESS,
            self.artifacts["initramfs.cpio"].len() as u64,
            0,
            RAM_BYTES,
        )?;
        let chosen = tree
            .get("/chosen")
            .ok_or_else(|| invalid("missing chosen"))?;
        if chosen.get("linux,initrd-end").map(Vec::as_slice)
            != Some((end as u32).to_be_bytes().as_slice())
        {
            return Err(invalid("basket final initrd-end"));
        }
        Ok(())
    }
    pub fn validate(&self, bus: &SystemBus) -> SimResult<()> {
        self.validate_content()?;
        if bus.ram.base_addr != 0
            || bus.ram.data.len() as u64 != RAM_BYTES
            || bus.cpu_hz != FUNCTIONAL_CPU_HZ
        {
            return Err(invalid(
                "basket requires exactly64MiB DDR and functional24MHz",
            ));
        }
        // Foundation configure does not itself reject all existing mappings.
        // A closed fresh bus preflight avoids ambiguous controller ownership.
        if !bus.flash.data.is_empty()
            || bus.current_cycle != 0
            || bus.irq_fabric.armv7.is_some()
            || !bus.peripherals.is_empty()
            || !bus.extra_mem.is_empty()
        {
            return Err(invalid("basket requires a fresh empty controller bus"));
        }
        let mut ranges = vec![(DECOMPRESSION_START, DECOMPRESSION_END)];
        for (address, name) in [
            (KERNEL_ADDRESS, "zImage"),
            (DTB_ADDRESS, "rv1103-basket.dtb"),
            (INITRD_ADDRESS, "initramfs.cpio"),
        ] {
            let end = checked_range(
                address,
                self.artifacts[name].len() as u64,
                bus.ram.base_addr,
                bus.ram.data.len() as u64,
            )?;
            if ranges.iter().any(|&(a, b)| address < b && a < end) {
                return Err(invalid("overlapping boot ranges"));
            }
            ranges.push((address, end));
        }
        Ok(())
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArchivePin {
    name: String,
    fields: [u32; 13],
    body_sha256: String,
}
/// Independent newc metadata/body validation, against reviewed trust, including
/// the single trailer. 25 semantic entries; no overlay, duplicate or tail.
pub fn validate_archive(bytes: &[u8]) -> SimResult<()> {
    let pins: Vec<ArchivePin> = serde_json::from_str(TRUSTED_NEWC).expect("trusted newc metadata");
    let mut cursor = 0usize;
    for pin in pins {
        let header = bytes
            .get(cursor..cursor.saturating_add(110))
            .ok_or_else(|| invalid("newc header range"))?;
        if &header[..6] != b"070701" {
            return Err(invalid("newc header magic"));
        }
        let mut fields = [0; 13];
        for (i, value) in fields.iter_mut().enumerate() {
            let raw = &header[6 + i * 8..14 + i * 8];
            if !raw
                .iter()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
            {
                return Err(invalid("newc canonical hex"));
            }
            *value = u32::from_str_radix(
                std::str::from_utf8(raw).map_err(|_| invalid("newc encoding"))?,
                16,
            )
            .map_err(|_| invalid("newc field"))?;
        }
        if fields != pin.fields {
            return Err(invalid(format!("newc {} deterministic metadata", pin.name)));
        }
        cursor += 110;
        let end = cursor
            .checked_add(fields[11] as usize)
            .ok_or_else(|| invalid("newc name overflow"))?;
        let name = bytes
            .get(cursor..end)
            .ok_or_else(|| invalid("newc name range"))?;
        if name != [pin.name.as_bytes(), &[0]].concat() {
            return Err(invalid("newc exact sorted whitelist"));
        }
        cursor = padded(bytes, end)?;
        let end = cursor
            .checked_add(fields[6] as usize)
            .ok_or_else(|| invalid("newc body overflow"))?;
        let body = bytes
            .get(cursor..end)
            .ok_or_else(|| invalid("newc body range"))?;
        if sha256(body) != pin.body_sha256 {
            return Err(invalid(format!("newc {} body identity", pin.name)));
        }
        cursor = padded(bytes, end)?;
    }
    if cursor != bytes.len() {
        return Err(invalid("newc trailing archive/bytes"));
    }
    Ok(())
}
fn padded(bytes: &[u8], end: usize) -> SimResult<usize> {
    let aligned = end
        .checked_add(3)
        .ok_or_else(|| invalid("newc alignment overflow"))?
        & !3;
    if bytes
        .get(end..aligned)
        .is_none_or(|pad| pad.iter().any(|b| *b != 0))
    {
        return Err(invalid("newc nonzero/missing padding"));
    }
    Ok(aligned)
}

/// RAM remains unchanged on every fallible validation/native/controller/device
/// operation. Normal kit attachment can partially populate the bus on error;
/// callers must discard that bus (never retry it). RAM commit follows all of it.
/// Returned native CPU remains unstarted at PC0; supply release precedes entry.
pub fn load_basket(bus: &mut SystemBus, artifacts: BasketArtifacts) -> SimResult<LoadedLinux> {
    artifacts.validate(bus)?;
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
        bus.configure_armv7_rv1103()?;
        bus.configure_armv7_rv1103_i2c3()?;
        bus.configure_armv7_rv1103_uart4()?;
        bus.register_supply_domain("external3v3", 2.7)
            .map_err(|e| invalid(e.to_string()))?;
        let manifest: labwired_config::SystemManifest =
            serde_json::from_str(CANONICAL_MANIFEST).map_err(|e| invalid(e.to_string()))?;
        bus.attach_manifest_external_devices(&manifest)
            .map_err(|e| invalid(e.to_string()))?;
        bus.supply = crate::power::SupplySupervisor::new(Some(labwired_config::SupplyMonitor {
            por_rising_v: 3.0,
            pdr_falling_v: 2.7,
            bor_levels: BTreeMap::new(),
            bor_level: None,
        }));
        bus.supply.attach();
        for (address, name) in [
            (KERNEL_ADDRESS, "zImage"),
            (DTB_ADDRESS, "rv1103-basket.dtb"),
            (INITRD_ADDRESS, "initramfs.cpio"),
        ] {
            let start = address as usize;
            let data = &artifacts.artifacts[name];
            bus.ram.data[start..start + data.len()].copy_from_slice(data);
        }
        // The existing return metadata type names the base DT, so do not use it
        // to authorize basket loading. Metadata here reports the actual profile.
        let manifest = ArtifactManifest {
            schema: 1,
            board: BOARD.into(),
            ram_base: 0,
            ram_bytes: RAM_BYTES,
            timer_hz: TIMER_HZ,
            decompression_start: DECOMPRESSION_START,
            decompression_end: DECOMPRESSION_END,
            bootargs: BOOTARGS.into(),
            artifacts: serde_json::from_value(artifacts.manifest["artifacts"].clone())
                .expect("validated artifacts"),
        };
        Ok(LoadedLinux { cpu, manifest })
    }
}
