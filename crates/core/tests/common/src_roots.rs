// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The one place that knows where the engine's Rust source lives.
//!
//! Source-scanning gates ("no cross-chip references", "every `impl Cpu for`
//! is traced", "this trait stays narrow") must not spell `src/...` themselves:
//! when a module moves, every spelling is a separate failure that has nothing
//! to do with behaviour. They ask this file instead, so a move is a one-line
//! edit to [`CORE_SRC_ROOTS`].
//!
//! Paths handed around here are *module-relative*: `cpu/avr.rs`,
//! `peripherals/components/i80_panel.rs`, `bus/mod.rs` - the path the file has
//! below the engine's `src/` directory today. A root may mount a directory at a
//! module prefix, so a module that later moves into its own crate keeps the same
//! module-relative name and no test that names it has to change.
//!
//! Included two ways, because the same helper serves both kinds of test:
//! `#[path = "common/src_roots.rs"] mod src_roots;` from an integration test,
//! and `#[path = "../../tests/common/src_roots.rs"]` from `src/tests/mod.rs`.
//! It depends on `std` only.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// `(directory relative to the workspace root, module prefix it is mounted at)`.
///
/// Today there is one root and no prefix. When a module tree moves to another
/// crate, add `("crates/core/<new>/src", "<module>")` (or move the first entry).
pub const CORE_SRC_ROOTS: &[(&str, &str)] = &[("crates/core/src", "")];

/// A Rust source file found under the engine's source roots.
#[derive(Debug, Clone)]
pub struct SrcFile {
    /// Module-relative path with `/` separators, e.g. `cpu/avr.rs`.
    pub rel: String,
    pub path: PathBuf,
}

pub fn workspace_root() -> PathBuf {
    // crates/core -> workspace root, normalised so `strip_prefix` against a
    // root computed the same way works.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

/// Directory of the first (primary) source root. Prefer [`core_src`] or
/// [`core_src_files_under`] when the thing wanted is a file or a subtree.
pub fn core_src_dir() -> PathBuf {
    workspace_root().join(CORE_SRC_ROOTS[0].0)
}

/// Directory for a module-relative path, if some root provides it.
fn resolve(rel: &str) -> Option<PathBuf> {
    let rel = rel.trim_matches('/');
    CORE_SRC_ROOTS.iter().find_map(|(dir, mount)| {
        let inner = if mount.is_empty() {
            rel
        } else if rel == *mount {
            ""
        } else {
            rel.strip_prefix(&format!("{mount}/"))?
        };
        let p = workspace_root().join(dir).join(inner);
        p.exists().then_some(p)
    })
}

/// Path of the module-relative file or directory `rel` (`"cpu/avr.rs"`,
/// `"bus"`). When nothing provides it, the path under the primary root is
/// returned so the caller's read fails with a message that names a real place.
pub fn core_src(rel: &str) -> PathBuf {
    resolve(rel).unwrap_or_else(|| core_src_dir().join(rel.trim_matches('/')))
}

pub fn read_core_src(rel: &str) -> String {
    let path = core_src(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

/// Every `.rs` file below the module-relative directory `sub` (`""` for the
/// whole engine), across all roots, sorted by module-relative path.
pub fn core_src_files_under(sub: &str) -> Vec<SrcFile> {
    let sub = sub.trim_matches('/');
    let mut out = Vec::new();
    for (dir, mount) in CORE_SRC_ROOTS {
        let root = workspace_root().join(dir);
        // The part of `sub` that lies inside this root.
        let inner = if mount.is_empty() || sub.is_empty() {
            Some(sub)
        } else if sub == *mount {
            Some("")
        } else {
            sub.strip_prefix(&format!("{mount}/"))
        };
        let Some(inner) = inner else { continue };
        let mut files = Vec::new();
        walk(&root.join(inner), &mut files);
        for path in files {
            let under_root = path.strip_prefix(&root).expect("under root");
            let under_root = under_root.to_string_lossy().replace('\\', "/");
            let rel = if mount.is_empty() {
                under_root
            } else {
                format!("{mount}/{under_root}")
            };
            out.push(SrcFile { rel, path });
        }
    }
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    out
}

/// The historic spelling of the module-relative namespace: a file that is
/// `cpu/avr.rs` here has always been `crates/core/src/cpu/avr.rs` in a
/// workspace-relative list. Allowlists that name files in that form keep
/// working through [`workspace_path`] when the file moves.
const LEGACY_SRC_PREFIX: &str = "crates/core/src/";

/// A workspace-relative path. Paths in the legacy `crates/core/src/<module>`
/// spelling are resolved through [`CORE_SRC_ROOTS`]; every other path (other
/// crates, `configs/`, `.github/`) is simply joined onto the workspace root.
pub fn workspace_path(rel: &str) -> PathBuf {
    match rel.strip_prefix(LEGACY_SRC_PREFIX) {
        Some(module_rel) => core_src(module_rel),
        None => workspace_root().join(rel),
    }
}

/// Module-relative path (`peripherals/uart.rs`) of a file found under a root.
pub fn module_rel(path: &Path) -> String {
    for (dir, mount) in CORE_SRC_ROOTS {
        if let Ok(under) = path.strip_prefix(workspace_root().join(dir)) {
            let under = under.to_string_lossy().replace('\\', "/");
            return if mount.is_empty() {
                under
            } else {
                format!("{mount}/{under}")
            };
        }
    }
    path.to_string_lossy().replace('\\', "/")
}
