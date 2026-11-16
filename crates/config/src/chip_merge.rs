// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Chip YAML `include:` expansion: a descriptor names one or more base
//! descriptors and overlays only what differs (a package variant of one die,
//! a board variant of one chip). Includes load first and local keys win;
//! `peripherals` merge by `id`, `memory_regions` by `name`, `pins` and
//! `analog_pins` by label; every other key is replaced whole.
//!
//! This file is shared, via `#[path]`, by the crate and by `build.rs`, which
//! flattens included built-in chips before embedding them so a chip resolved
//! by bare name is the same descriptor as the file loaded by path. It may only
//! depend on `std`, `anyhow` and `serde_yaml`.

#![allow(dead_code)]

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub(crate) fn yaml_str_key(key: &str) -> serde_yaml::Value {
    serde_yaml::Value::String(key.to_string())
}

/// `from_str` accepts `size: 1024` as a string field; `from_value` does not.
/// Walk mappings and stringify numeric `size` so include-merge can deserialize
/// without a YAML text round-trip.
pub(crate) fn coerce_yaml_size_numbers(value: &mut serde_yaml::Value) {
    match value {
        serde_yaml::Value::Mapping(map) => {
            let size_key = yaml_str_key("size");
            if let Some(serde_yaml::Value::Number(n)) = map.get(&size_key) {
                let s = n.to_string();
                map.insert(size_key, serde_yaml::Value::String(s));
            }
            for (_, v) in map.iter_mut() {
                coerce_yaml_size_numbers(v);
            }
        }
        serde_yaml::Value::Sequence(seq) => {
            for v in seq {
                coerce_yaml_size_numbers(v);
            }
        }
        _ => {}
    }
}

pub(crate) fn mapping_field<'a>(value: &'a serde_yaml::Value, key: &str) -> Option<&'a str> {
    value.as_mapping()?.get(yaml_str_key(key))?.as_str()
}

pub(crate) fn take_includes(doc: &mut serde_yaml::Value) -> Result<Vec<String>> {
    let Some(map) = doc.as_mapping_mut() else {
        return Ok(Vec::new());
    };
    let Some(value) = map.remove(yaml_str_key("include")) else {
        return Ok(Vec::new());
    };
    if matches!(value, serde_yaml::Value::Null) {
        return Ok(Vec::new());
    }
    match value {
        serde_yaml::Value::String(path) => Ok(vec![path]),
        serde_yaml::Value::Sequence(items) => items
            .into_iter()
            .map(|item| match item {
                serde_yaml::Value::String(path) => Ok(path),
                _ => anyhow::bail!("include must be a string or a list of strings"),
            })
            .collect(),
        _ => anyhow::bail!("include must be a string or a list of strings"),
    }
}

pub(crate) fn merge_seq_by(
    base: serde_yaml::Value,
    overlay: serde_yaml::Value,
    field: &str,
) -> Result<serde_yaml::Value> {
    let mut items = match base {
        serde_yaml::Value::Null => Vec::new(),
        serde_yaml::Value::Sequence(seq) => seq,
        other => anyhow::bail!("expected a sequence while merging {field}, got {other:?}"),
    };
    let overlay = match overlay {
        serde_yaml::Value::Null => return Ok(serde_yaml::Value::Sequence(items)),
        serde_yaml::Value::Sequence(seq) => seq,
        other => anyhow::bail!("expected a sequence while merging {field}, got {other:?}"),
    };
    for item in overlay {
        if let Some(id) = mapping_field(&item, field).map(str::to_string) {
            if let Some(existing) = items
                .iter_mut()
                .find(|entry| mapping_field(entry, field) == Some(id.as_str()))
            {
                *existing = item;
                continue;
            }
        }
        items.push(item);
    }
    Ok(serde_yaml::Value::Sequence(items))
}

pub(crate) fn merge_yaml_maps(
    base: serde_yaml::Value,
    overlay: serde_yaml::Value,
) -> serde_yaml::Value {
    let mut map = match base {
        serde_yaml::Value::Mapping(map) => map,
        _ => serde_yaml::Mapping::new(),
    };
    if let serde_yaml::Value::Mapping(overlay) = overlay {
        for (key, value) in overlay {
            map.insert(key, value);
        }
    }
    serde_yaml::Value::Mapping(map)
}

pub(crate) fn merge_chip_yaml(
    base: serde_yaml::Value,
    overlay: serde_yaml::Value,
) -> Result<serde_yaml::Value> {
    let mut base_map = match base {
        serde_yaml::Value::Mapping(map) => map,
        serde_yaml::Value::Null => serde_yaml::Mapping::new(),
        other => anyhow::bail!("chip YAML must be a mapping, got {other:?}"),
    };
    let overlay_map = match overlay {
        serde_yaml::Value::Mapping(map) => map,
        serde_yaml::Value::Null => return Ok(serde_yaml::Value::Mapping(base_map)),
        other => anyhow::bail!("chip YAML must be a mapping, got {other:?}"),
    };
    for (key, value) in overlay_map {
        let key_name = key.as_str().unwrap_or("");
        let merged = match key_name {
            "include" => continue,
            "peripherals" => merge_seq_by(
                base_map.remove(&key).unwrap_or(serde_yaml::Value::Null),
                value,
                "id",
            )?,
            "memory_regions" => merge_seq_by(
                base_map.remove(&key).unwrap_or(serde_yaml::Value::Null),
                value,
                "name",
            )?,
            "pins" | "analog_pins" => merge_yaml_maps(
                base_map.remove(&key).unwrap_or(serde_yaml::Value::Null),
                value,
            ),
            _ => value,
        };
        base_map.insert(key, merged);
    }
    Ok(serde_yaml::Value::Mapping(base_map))
}

pub(crate) fn expand_chip_includes(
    path: &Path,
    content: &str,
    stack: &mut Vec<PathBuf>,
) -> Result<serde_yaml::Value> {
    let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if stack.contains(&canon) {
        anyhow::bail!("cycle detected in chip YAML include of {}", path.display());
    }
    stack.push(canon);
    let result = (|| {
        let mut doc: serde_yaml::Value =
            serde_yaml::from_str(content).context("Failed to parse Chip Descriptor YAML")?;
        let includes = take_includes(&mut doc)?;
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        let mut merged = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
        for rel in includes {
            let inc_path = dir.join(&rel);
            if !inc_path.is_file() {
                anyhow::bail!("chip YAML include not found: {}", inc_path.display());
            }
            let inc_content = std::fs::read_to_string(&inc_path).with_context(|| {
                format!("Failed to read chip YAML include {}", inc_path.display())
            })?;
            let included = expand_chip_includes(&inc_path, &inc_content, stack)?;
            merged = merge_chip_yaml(merged, included)?;
        }
        merge_chip_yaml(merged, doc)
    })();
    stack.pop();
    result
}
