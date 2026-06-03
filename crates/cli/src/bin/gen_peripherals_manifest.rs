// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Serialise the [`labwired_core::peripherals::kit::registry::KITS`] slice
//! into a JSON manifest the playground / docs / generated UI consume.
//!
//! Run with:
//!
//! ```sh
//! cargo run -p labwired-cli --bin gen-peripherals-manifest -- \
//!     --out path/to/peripherals-manifest.json
//! ```
//!
//! With no `--out` the JSON is written to stdout. The gate test re-runs
//! this generator and diffs against the committed file — so the manifest
//! cannot drift from the registry without CI catching it.

use std::path::PathBuf;

use anyhow::{Context, Result};
use labwired_core::peripherals::kit::registry;
use serde::Serialize;

#[derive(Serialize)]
struct Manifest {
    /// Schema version. Bumped when the JSON shape changes (the TS reader
    /// pins to a major; consumers must update together).
    schema_version: u32,
    /// All peripherals registered through `PeripheralKit`. One entry per
    /// kit; legacy hand-written peripherals are not represented here.
    peripherals: Vec<&'static labwired_core::peripherals::kit::KitMetadata>,
}

fn main() -> Result<()> {
    let mut out_path: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" | "-o" => {
                let p = args.next().context("--out requires a path argument")?;
                out_path = Some(PathBuf::from(p));
            }
            "--help" | "-h" => {
                println!("Usage: gen-peripherals-manifest [--out <path>]");
                return Ok(());
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }

    let manifest = Manifest {
        schema_version: 1,
        peripherals: registry::kits().iter().map(|k| k.metadata()).collect(),
    };
    let json = serde_json::to_string_pretty(&manifest)?;

    match out_path {
        Some(path) => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating parent directory {}", parent.display()))?;
            }
            // Force trailing newline so editors / git don't complain.
            let mut bytes = json.into_bytes();
            if !bytes.ends_with(b"\n") {
                bytes.push(b'\n');
            }
            std::fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))?;
            eprintln!("wrote {}", path.display());
        }
        None => {
            println!("{json}");
        }
    }
    Ok(())
}
