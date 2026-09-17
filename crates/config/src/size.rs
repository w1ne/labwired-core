// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

#![allow(dead_code)]
use crate::*;

pub(crate) fn deserialize_u64_lax<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum IntOrString {
        Int(u64),
        String(String),
    }

    match IntOrString::deserialize(deserializer)? {
        IntOrString::Int(v) => Ok(v),
        IntOrString::String(s) => {
            let s = s.trim();
            if let Some(stripped) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                u64::from_str_radix(&stripped.replace('_', ""), 16)
                    .map_err(serde::de::Error::custom)
            } else {
                s.replace('_', "")
                    .parse::<u64>()
                    .map_err(serde::de::Error::custom)
            }
        }
    }
}

/// [`deserialize_u64_lax`] for an optional field: absent ⇒ `None`, present ⇒
/// the same int-or-underscored-string parse. YAML 1.2 does not accept `_` in a
/// number, so `cpu_hz: 160_000_000` arrives as a *string* — the corpus is
/// written that way throughout and this is what makes it a clock.
pub(crate) fn deserialize_opt_u64_lax<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let raw = Option::<serde_yaml::Value>::deserialize(deserializer)?;
    match raw {
        None | Some(serde_yaml::Value::Null) => Ok(None),
        Some(v) => deserialize_u64_lax(v).map(Some).map_err(|e| {
            serde::de::Error::custom(format!("cpu_hz is not a whole number of hertz: {e}"))
        }),
    }
}

/// Default schema version for YAML configs
pub(crate) fn default_schema_version() -> String {
    "1.0".to_string()
}

/// Deserialize a memory size, in bytes, from the human form the chip YAMLs use.
///
/// The wire format is unchanged — `128KB`, `1.5 MiB`, `0x20000` and a bare
/// `131072` all still load. What changed is that the parse happens HERE, once,
/// at the boundary, instead of at each of the 39 places that used to call
/// `parse_size(&chip.ram.size)` on a `String` field.
///
/// That mattered: 18 of those call sites ended `.unwrap_or(0)`. A size that
/// failed to parse did not fail the run — it silently became **zero bytes of
/// RAM**, and the eleven ESP32-C3 suites computing `sp_top = ram.base + size`
/// got a stack pointer at the very bottom of RAM. Wrong, and green. Making the
/// field a `u64` deletes the fallible read, so that state cannot be
/// constructed: a bad size is now a load error naming the field.
pub(crate) fn deserialize_size<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum IntOrString {
        Int(u64),
        String(String),
    }

    match IntOrString::deserialize(deserializer)? {
        IntOrString::Int(v) => Ok(v),
        IntOrString::String(s) => parse_size(&s).map_err(serde::de::Error::custom),
    }
}

/// Serialize a size back as a bare byte count.
///
/// Deliberately NOT re-rendered in a unit, because the units here do not mean
/// what they look like. Measured against the real parser:
///
/// ```text
///   1KB  -> 1024          1KiB -> 1024
///   1MB  -> 1_000_000     1MiB -> 1_048_576
/// ```
///
/// `KB` is BINARY and `MB` is DECIMAL — inconsistent with each other, inside
/// one parser. So re-rendering `1048576` as `1MB` would read back as
/// `1_000_000` and quietly shrink a chip's flash by 4.9% on every round trip.
/// A bare byte count says exactly one thing and `parse_size` reads it back
/// unchanged.
///
/// (That asymmetry was also a live fidelity bug: nine committed chips spelled
/// flash in `MB`, so e.g. esp32s3 modelled 16_000_000 bytes where the part has
/// 16 MiB = 16_777_216. All nine have since been rewritten in `KB`, and
/// `labwired_core::tests::chip_memory_sizes` fails the build if a new chip
/// reintroduces the spelling.)
pub(crate) fn serialize_size<S>(value: &u64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_u64(*value)
}

pub fn parse_size(size_str: &str) -> Result<u64> {
    use human_size::{Byte, Size, SpecificSize};
    let trimmed = size_str.trim();
    // A bare integer is a raw byte count. `human_size` rejects unit-less values
    // with "no multiple", but many chip configs give sizes as plain bytes
    // (e.g. `1048576`), so accept those directly before falling back to the
    // unit-aware parser ("512KB", "1.5 MiB", …).
    if let Ok(bytes) = trimmed.parse::<u64>() {
        return Ok(bytes);
    }
    let s: Size = trimmed
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid size format: {}", e))?;
    let bytes: SpecificSize<Byte> = s.into();
    Ok(bytes.value() as u64)
}

#[cfg(test)]
#[path = "lib_parse_size_tests.rs"]
mod parse_size_tests;

#[cfg(test)]
#[path = "lib_memory_size_tests.rs"]
mod memory_size_tests;
