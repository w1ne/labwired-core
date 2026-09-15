// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Resolve a chip by name against the config catalog on disk, the way the CLI
//! and browser both do it: `configs/chips/<chip>.yaml`, plus a matching
//! `configs/systems/<chip>.yaml` when one exists.

use std::path::PathBuf;

/// Search roots for chip/system configs, in priority order.
pub struct Catalog {
    roots: Vec<PathBuf>,
}

impl Catalog {
    /// Roots, in order: `$LABWIRED_CONFIG_DIR`, then `<workspace>/configs`
    /// (via `CARGO_MANIFEST_DIR`).
    pub fn discover() -> Catalog {
        let mut roots = Vec::new();
        if let Ok(dir) = std::env::var("LABWIRED_CONFIG_DIR") {
            roots.push(PathBuf::from(dir));
        }
        roots.push(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../configs")
                .canonicalize()
                .unwrap_or_else(|_| {
                    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs")
                }),
        );
        Catalog { roots }
    }

    /// Stems of `chips/*.yaml` across every root, non-recursive. `onboarding/`
    /// is excluded (it holds worked examples, not chip descriptors).
    pub fn chip_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for root in &self.roots {
            let dir = root.join("chips");
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
                    continue;
                }
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    if !names.iter().any(|n: &String| n == stem) {
                        names.push(stem.to_string());
                    }
                }
            }
        }
        names.sort();
        names
    }

    /// Resolve `chip` to a `(ChipDescriptor, SystemManifest)` pair. The chip
    /// file is the first root with `chips/<chip>.yaml`; the system is
    /// `systems/<chip>.yaml` in the same root if present, else a synthesized
    /// single-chip manifest. `manifest.chip` is always rewritten to the
    /// absolute chip path, as `build_system_bus` does.
    pub fn resolve(
        &self,
        chip: &str,
    ) -> anyhow::Result<(
        labwired_config::ChipDescriptor,
        labwired_config::SystemManifest,
    )> {
        let root = self
            .roots
            .iter()
            .find(|r| r.join("chips").join(format!("{chip}.yaml")).is_file())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no chip named '{chip}' in any catalog root ({:?})",
                    self.roots
                )
            })?;
        let chip_path = root.join("chips").join(format!("{chip}.yaml"));
        let descriptor = labwired_config::ChipDescriptor::from_file(&chip_path)?;

        let system_path = root.join("systems").join(format!("{chip}.yaml"));
        let mut manifest = if system_path.is_file() {
            labwired_config::SystemManifest::from_file(&system_path)?
        } else {
            let yaml = format!(
                "name: {chip}\nchip: {}\nexternal_devices: []\n",
                chip_path.to_string_lossy()
            );
            serde_yaml::from_str(&yaml)?
        };
        manifest.chip = chip_path.to_string_lossy().into_owned();
        Ok((descriptor, manifest))
    }
}

impl Default for Catalog {
    fn default() -> Self {
        Self::discover()
    }
}
