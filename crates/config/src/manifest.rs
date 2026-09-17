// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

#![allow(dead_code)]
use crate::*;

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BoardIoKind {
    Led,
    Button,
    AdcInput,
    PwmOutput,
    I2cDevice,
    SpiDevice,
    UartDevice,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum BoardIoSignal {
    #[default]
    Output,
    Input,
}

pub(crate) fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BoardIoBinding {
    pub id: String,
    pub kind: BoardIoKind,
    pub peripheral: String,
    pub pin: u8,
    #[serde(default)]
    pub signal: BoardIoSignal,
    #[serde(default = "default_true")]
    pub active_high: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub i2c_address: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_type: Option<String>,
    /// Stimulus channel this contact exposes, when it is something other than a
    /// plain `pressed` button.
    ///
    /// A PIR, IR-obstacle, hall or vibration sensor is electrically the same
    /// thing — a digital output asserting a level on one pin — but "pressed" is
    /// the wrong word for motion or a magnetic field. The canvas catalog already
    /// names each one (`obstacle`, `field`, `vibration`), so the compiler stamps
    /// that name here rather than the engine keeping a second copy of the
    /// vocabulary. Absent means `pressed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
}

pub(crate) fn default_wifi_ap_ssid() -> String {
    "labwired-ap".to_string()
}

pub(crate) fn default_wifi_ap_ip() -> String {
    "192.168.4.1".to_string()
}

pub(crate) fn default_wifi_ap_serves() -> String {
    "labwired-stats".to_string()
}

/// Manifest opt-in for the per-lab virtual WiFi Access Point. Emitted when a
/// diagram contains a `wifi-ap` component. Absent ⇒ no AP (WiFi MACs stay
/// unassociated — honest "no AP present"). Mirrors `debug_uart`'s optional
/// pattern.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct WifiApManifest {
    /// Broadcast SSID of the AP.
    #[serde(default = "default_wifi_ap_ssid")]
    pub ssid: String,
    /// AP's IPv4 address (dotted-quad); its /24 is the DHCP pool.
    #[serde(default = "default_wifi_ap_ip")]
    pub ip: String,
    /// What the AP's HTTP origin serves: "labwired-stats" (default) or "none".
    #[serde(default = "default_wifi_ap_serves")]
    pub serves: String,
    /// Optional network password (PSK). Empty / absent = open AP.
    /// Stored so labs can record credentials that match firmware
    /// `WiFi.begin(ssid, password)`; the frame-level virtual AP does not
    /// model a WPA handshake yet — association still behaves as open.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SystemManifest {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    pub name: String,
    pub chip: String, // Reference to chip name or file path
    /// Override for [`ChipDescriptor::cpu_hz`] — the clock THIS board runs the
    /// part at, in Hz. Absent ⇒ the chip's declared default.
    ///
    /// The key has been in the corpus for a long time; until now nothing in the
    /// engine read it. Ten system YAMLs declared a `cpu_hz:` that serde threw
    /// away, and `nucleo-l476rg.yaml` documented the discard in a comment —
    /// the firmware there never configures the PLL, so the core really runs at
    /// the 4 MHz MSI reset rate and every self-timed device on that board was
    /// nevertheless being told 80 MHz. Reading the key is the fix.
    #[serde(default, deserialize_with = "deserialize_opt_u64_lax")]
    pub cpu_hz: Option<u64>,
    #[serde(default)]
    pub memory_overrides: HashMap<String, String>,
    #[serde(default)]
    pub external_devices: Vec<ExternalDevice>,
    /// Part packs this system carries — the parts that are NOT built into the
    /// engine. An `external_devices` entry whose `type:` names a pack here is
    /// modelled from that pack, so a private/vendor/customer catalog connects
    /// with no code in this repository and nothing published. See
    /// `docs/part-packs.md` for the `labwired.part/v1` contract.
    ///
    /// Packs ride in the manifest so every transport that already carries a
    /// manifest carries them too: the CLI, the browser wasm build, and the
    /// hosted builder's `/run`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<PartPack>,
    #[serde(default)]
    pub cosim_models: Vec<CosimModelConfig>,
    #[serde(default)]
    pub motor_models: Vec<MotorModelConfig>,
    #[serde(default)]
    pub board_io: Vec<BoardIoBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug_uart: Option<String>,
    /// Per-lab virtual WiFi AP config (present ⇒ a `wifi-ap` component is on the
    /// diagram). Absent ⇒ no AP. See [`WifiApManifest`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wifi_ap: Option<WifiApManifest>,
    #[serde(default)]
    pub peripherals: Vec<PeripheralConfig>,
    /// Per-cycle peripheral-walk deletion (only consulted in `event-scheduler`
    /// builds; no-op otherwise). Three states:
    ///
    /// - **absent (`None`)** — the core auto-derives walk-deletability at
    ///   `from_config` finalize time: the walk is deleted iff EVERY peripheral
    ///   on the bus is provably walk-independent for all firmware states
    ///   (scheduler-driven, or its `tick()` is a structural no-op — see the
    ///   core's `Peripheral::needs_legacy_walk` contract). Conservative: any
    ///   peripheral that could ever do walk work keeps the walk on.
    /// - **`Some(true)`** — force the walk deleted (hand opt-in / escape hatch
    ///   for configs the author verified byte-identical walk-free but that the
    ///   conservative auto-derivation cannot prove — e.g. a firmware that never
    ///   arms the timers/ADC/DMA the chip descriptor instantiates).
    /// - **`Some(false)`** — pin the walk ON, overriding any auto-derivation.
    ///
    /// Deserializes from the YAML `walk_deleted:` key; omit it for auto-derive.
    #[serde(default)]
    pub walk_deleted: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct NodeConfig {
    pub id: String,
    pub system: String,   // Path to SystemManifest
    pub firmware: String, // Path to ELF
    #[serde(default)]
    pub config_overrides: HashMap<String, serde_yaml::Value>,
}

/// Optional shared RF medium for a multi-node world (path loss / RSSI floor).
/// Positions are planar metres; co-located (default 0,0) keeps lossless links
/// until nodes are placed. Radios that honor [`labwired_core::peripherals::rf_medium`]
/// attach separately; this block is the env-manifest source of truth for seed + layout.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentRfConfig {
    /// Run seed for seeded PER / medium draws (0 is valid).
    #[serde(default)]
    pub seed: u64,
    /// Node id → planar position in metres.
    #[serde(default)]
    pub nodes: HashMap<String, EnvironmentRfNode>,
    /// Minimum RSSI (dBm) for decode; below → drop. Omit for medium default.
    #[serde(default)]
    pub rssi_floor_dbm: Option<f64>,
    /// Path-loss exponent (free space ≈ 2). Omit for medium default.
    #[serde(default)]
    pub path_loss_exponent: Option<f64>,
    /// Reference loss at 1 m (dB). Omit for medium default.
    #[serde(default)]
    pub ref_loss_db: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentRfNode {
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentManifest {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    pub name: String,
    pub nodes: Vec<NodeConfig>,
    #[serde(default)]
    pub interconnects: Vec<InterconnectConfig>,
    /// Shared RF medium parameters (optional).
    #[serde(default)]
    pub rf: Option<EnvironmentRfConfig>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct InterconnectConfig {
    pub r#type: String,     // "uart_cross_link", "virtual_switch", etc.
    pub nodes: Vec<String>, // List of node IDs
    #[serde(default)]
    pub config: HashMap<String, serde_yaml::Value>,
}

impl EnvironmentManifest {
    /// Test/helper constructor with no RF block.
    pub fn bare(
        name: impl Into<String>,
        nodes: Vec<NodeConfig>,
        interconnects: Vec<InterconnectConfig>,
    ) -> Self {
        Self {
            schema_version: "1.0".into(),
            name: name.into(),
            nodes,
            interconnects,
            rf: None,
        }
    }

    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let source = std::fs::read_to_string(path)?;
        // `NodeConfig` deliberately keeps a plain HashMap for source
        // compatibility with callers that build worlds in Rust. A YAML key with
        // an empty mapping would otherwise deserialize identically to an absent
        // key, so inspect the wire shape before that normalization happens.
        let wire: serde_yaml::Value =
            serde_yaml::from_str(&source).context("Failed to parse Environment Manifest")?;
        reject_explicit_node_config_overrides(&wire)?;
        let manifest: Self =
            serde_yaml::from_str(&source).context("Failed to parse Environment Manifest")?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Validate the structural contract shared by all environment runners.
    ///
    /// Topology-specific checks stay with `World::from_manifest`, where the
    /// named peripherals and machines are available. This layer rejects input
    /// that cannot describe an unambiguous world at all.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != "1.0" {
            anyhow::bail!(
                "Unsupported environment schema_version '{}'. Supported version: '1.0'",
                self.schema_version
            );
        }
        if self.name.trim().is_empty() {
            anyhow::bail!("Environment manifest requires a non-empty name");
        }
        if self.nodes.is_empty() {
            anyhow::bail!("Environment manifest requires at least one node");
        }

        let mut node_ids = HashSet::with_capacity(self.nodes.len());
        for (index, node) in self.nodes.iter().enumerate() {
            if node.id.trim().is_empty() {
                anyhow::bail!("Environment manifest nodes[{index}].id must be non-empty");
            }
            if !node_ids.insert(&node.id) {
                anyhow::bail!("Environment manifest has duplicate node id '{}'", node.id);
            }
            if node.system.trim().is_empty() {
                anyhow::bail!("Environment manifest nodes[{index}].system must be non-empty");
            }
            if node.firmware.trim().is_empty() {
                anyhow::bail!("Environment manifest nodes[{index}].firmware must be non-empty");
            }
            if !node.config_overrides.is_empty() {
                anyhow::bail!(
                    "Environment manifest nodes[{index}].config_overrides is unsupported in environment schema 1.0"
                );
            }
        }

        for (index, interconnect) in self.interconnects.iter().enumerate() {
            validate_environment_interconnect_config(index, interconnect)?;
        }

        if let Some(rf) = &self.rf {
            for node_id in rf.nodes.keys() {
                if !node_ids.contains(node_id) {
                    anyhow::bail!(
                        "Environment manifest rf.nodes contains unknown node id '{node_id}'"
                    );
                }
            }
            if let Some(exp) = rf.path_loss_exponent {
                if !exp.is_finite() || exp <= 0.0 {
                    anyhow::bail!(
                        "Environment manifest rf.path_loss_exponent must be a positive finite number"
                    );
                }
            }
            if let Some(r) = rf.ref_loss_db {
                if !r.is_finite() {
                    anyhow::bail!("Environment manifest rf.ref_loss_db must be finite");
                }
            }
            if let Some(floor) = rf.rssi_floor_dbm {
                if !floor.is_finite() {
                    anyhow::bail!("Environment manifest rf.rssi_floor_dbm must be finite");
                }
            }
        }

        Ok(())
    }
}

impl SystemManifest {
    /// Parse a System Manifest from a YAML string. Unlike [`Self::from_file`]
    /// this does no filesystem-relative `can-player` `path:` inlining (there is
    /// no base directory), so it is wasm-safe and handy for tests/embedding.
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        serde_yaml::from_str(yaml).context("Failed to parse System Manifest")
    }

    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let f = std::fs::File::open(path)?;
        let mut manifest: SystemManifest =
            serde_yaml::from_reader(f).context("Failed to parse System Manifest")?;
        // can-player accepts `path:` as a CLI convenience; core itself only
        // ever sees `data:` (keeps std::fs out of the sim core → wasm-safe).
        let base = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        for ext in &mut manifest.external_devices {
            if ext.r#type == "can-player" {
                if ext.config.contains_key("path") && ext.config.contains_key("data") {
                    return Err(anyhow::anyhow!(
                        "can-player '{}': both 'path' and 'data' are set in config; set only one \
                         ('path' is a CLI convenience that inlines the log into 'data')",
                        ext.id
                    ));
                }
                if let Some(p) = ext.config.remove("path") {
                    let p = p
                        .as_str()
                        .ok_or_else(|| {
                            anyhow::anyhow!("can-player '{}': path must be a string", ext.id)
                        })?
                        .to_string();
                    let full = base.join(&p);
                    let text = std::fs::read_to_string(&full).map_err(|e| {
                        anyhow::anyhow!("can-player '{}': cannot read log {:?}: {e}", ext.id, full)
                    })?;
                    ext.config
                        .insert("data".into(), serde_yaml::Value::String(text));
                }
            }
        }
        // `parts: [- path: ./acme.yaml]` is the same CLI convenience: read it
        // here so the core only ever sees an inline pack (wasm has no fs).
        for entry in &mut manifest.parts {
            let rel = match entry {
                PartPack::Path(p) => p.path.clone(),
                PartPack::Inline(_) => continue,
            };
            let full = base.join(&rel);
            let text = std::fs::read_to_string(&full)
                .map_err(|e| anyhow::anyhow!("part pack {:?}: cannot read {e}", full))?;
            let origin = full.display().to_string();
            *entry = PartPack::Inline(Box::new(parse_part_pack(&text, &origin)?));
        }
        manifest.validate_parts()?;
        Ok(manifest)
    }

    /// The ONE lookup for a manifest-carried part. Returns the pack whose
    /// `type:` matches, or `None` when this system carries no such part (the
    /// caller then falls through to the built-in registries).
    ///
    /// Call [`Self::validate_parts`] before relying on this — it is what
    /// guarantees the match is unambiguous.
    pub fn resolve_part(&self, device_type: &str) -> Option<&DeviceDescriptor> {
        self.parts
            .iter()
            .filter_map(PartPack::descriptor)
            .find(|d| d.r#type == device_type)
    }

    /// Enforce the contract across the whole `parts:` list: every entry is a
    /// resolved, well-formed pack, and no two packs claim the same `type`.
    ///
    /// Two packs for one type is the failure this exists to prevent. Picking a
    /// winner would mean a customer's firmware silently runs against whichever
    /// catalog happened to load last — so it is an error, named, with both
    /// sources in the message.
    pub fn validate_parts(&self) -> Result<()> {
        let mut seen: HashMap<&str, Option<&str>> = HashMap::new();
        for entry in &self.parts {
            let pack = match entry {
                PartPack::Inline(d) => d.as_ref(),
                PartPack::Path(p) => anyhow::bail!(
                    "part pack '{}' was never loaded. `path:` is a CLI convenience that \
                     SystemManifest::from_file inlines; a manifest handed to the engine \
                     directly (browser, hosted runner) must carry the pack inline.",
                    p.path
                ),
            };
            let origin = pack
                .source
                .as_deref()
                .map(|s| format!("source: {s}"))
                .unwrap_or_else(|| "no declared source".to_string());
            validate_part_pack(pack, &origin)?;
            if let Some(prev) = seen.insert(pack.r#type.as_str(), pack.source.as_deref()) {
                anyhow::bail!(
                    "two part packs both define '{}' (sources: {} and {}). \
                     One part is one document — rename one, or namespace them \
                     `vendor:part`.",
                    pack.r#type,
                    prev.unwrap_or("undeclared"),
                    pack.source.as_deref().unwrap_or("undeclared"),
                );
            }
        }
        Ok(())
    }

    pub fn validate_cosim_models(&self) -> Vec<String> {
        let mut issues = Vec::new();

        for (index, model) in self.cosim_models.iter().enumerate() {
            let location = format!("cosim_models[{index}]");
            if model.id.trim().is_empty() {
                issues.push(format!("{location}.id must be a non-empty identifier"));
            }
            if model.step_ns == 0 {
                issues.push(format!("{location}.step_ns must be greater than zero"));
            }
            if matches!(
                model.adapter,
                CosimAdapter::ExternalProcess | CosimAdapter::Fmi
            ) && model
                .model
                .as_deref()
                .is_none_or(|path| path.trim().is_empty())
            {
                issues.push(format!(
                    "{location}.model is required for {:?} adapters",
                    model.adapter
                ));
            }
            if model.adapter == CosimAdapter::Analog {
                // The netlist itself is parsed by the core crate, which knows
                // the element subset; see
                // `labwired_core::cosim::validate_analog_models`. What is
                // checkable here is that the manifest declares a circuit and
                // says what to read out of it.
                let has_netlist = model
                    .config
                    .get("netlist")
                    .or_else(|| model.config.get("netlist_text"))
                    .is_some();
                if !has_netlist {
                    issues.push(format!(
                        "{location}.config requires `netlist` (a path) or `netlist_text` \
                         (inline) for the analog adapter"
                    ));
                }
                if !model.config.contains_key("probes") {
                    issues.push(format!(
                        "{location}.config.probes is required for the analog adapter: a \
                         model with no probe produces no outputs"
                    ));
                }
            }
        }

        issues
    }

    /// Resolves both explicitly typed plants and canonical external-device
    /// entries through one typed validation boundary.
    pub fn resolved_motor_models(&self) -> Result<Vec<MotorModelConfig>> {
        let mut models = Vec::new();
        let mut locations = HashMap::<String, String>::new();
        for (index, model) in self.motor_models.iter().enumerate() {
            let location = format!("motor_models[{index}].id");
            if let Some(previous) = locations.insert(model.id().to_owned(), location.clone()) {
                return Err(anyhow::anyhow!(
                    "{location} duplicates motor id declared at {previous}"
                ));
            }
            models.push(model.clone());
        }
        for (index, device) in self.external_devices.iter().enumerate() {
            if let Some(model) = MotorModelConfig::from_external_device(device)? {
                let location = format!("external_devices[{index}].id");
                if let Some(previous) = locations.insert(model.id().to_owned(), location.clone()) {
                    return Err(anyhow::anyhow!(
                        "{location} duplicates motor id declared at {previous}"
                    ));
                }
                models.push(model);
            }
        }
        let issues: Vec<_> = models.iter().flat_map(MotorModelConfig::validate).collect();
        if issues.is_empty() {
            Ok(models)
        } else {
            Err(anyhow::anyhow!(issues.join("; ")))
        }
    }
}

#[cfg(test)]
#[path = "lib_can_player_path_inline_tests.rs"]
mod can_player_path_inline_tests;
