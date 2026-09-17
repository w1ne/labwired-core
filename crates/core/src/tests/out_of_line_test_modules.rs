//! Source-scanner support: which `.rs` files under `src/` are test scaffolding
//! pulled in as out-of-line `#[cfg(test)]` modules.
//!
//! Several contract tests in this tree read production sources as text and
//! blank out inline `#[cfg(test)] mod x { .. }` bodies so mock peripherals and
//! fake buses never count as models. The same scaffolding can also live in a
//! sibling file declared as
//!
//! ```text
//! #[cfg(test)]
//! #[path = "spi_tests.rs"]
//! mod tests;
//! ```
//!
//! or as a plain `#[cfg(test)] mod tests;` resolved by the normal module rules.
//! Such a file has no `#[cfg(test)]` wrapper of its own, so a scanner that only
//! blanks inline bodies would read it as production code. This helper finds
//! every file reached that way so the scanners can skip it, keeping the rule
//! identical whether a test module is inline or out of line.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Return every file in `files` that some other file in `files` declares as an
/// out-of-line module under a `#[cfg(test)]` or `#[cfg(all(test, ..))]`
/// attribute. Paths are returned exactly as they appear in `files`.
pub(crate) fn out_of_line_cfg_test_files(files: &[PathBuf]) -> BTreeSet<PathBuf> {
    let known: BTreeSet<PathBuf> = files.iter().cloned().collect();
    let mut out = BTreeSet::new();
    for file in files {
        let Ok(src) = std::fs::read_to_string(file) else {
            continue;
        };
        for decl in cfg_test_module_decls(&src) {
            for candidate in resolve_module_file(file, &decl) {
                if known.contains(&candidate) {
                    out.insert(candidate);
                }
            }
        }
    }
    out
}

/// One `mod name;` declaration found under a test-only `cfg`.
struct ModDecl {
    name: String,
    /// The `#[path = ".."]` value, if any.
    path: Option<String>,
}

/// Scan `src` for attribute runs that contain a test-only `cfg` and end in an
/// out-of-line `mod name;`. Inline `mod name { .. }` bodies are ignored; the
/// scanners already blank those themselves.
fn cfg_test_module_decls(src: &str) -> Vec<ModDecl> {
    let mut decls = Vec::new();
    let mut i = 0usize;
    while let Some(rel) = src[i..].find("#[cfg(") {
        let at = i + rel;
        let mut cursor = at;
        let mut is_test = false;
        let mut path = None;
        // Walk a run of consecutive outer attributes.
        loop {
            let rest = &src[cursor..];
            let trimmed = rest.trim_start();
            if !trimmed.starts_with("#[") {
                break;
            }
            let attr_start = cursor + (rest.len() - trimmed.len());
            let Some(close_rel) = src[attr_start..].find(']') else {
                break;
            };
            let attr = &src[attr_start + 2..attr_start + close_rel];
            if is_test_cfg(attr) {
                is_test = true;
            }
            if let Some(p) = path_attr_value(attr) {
                path = Some(p);
            }
            cursor = attr_start + close_rel + 1;
        }
        if is_test {
            let rest = src[cursor..].trim_start();
            let rest = rest
                .strip_prefix("pub ")
                .map(str::trim_start)
                .unwrap_or(rest);
            if let Some(after_mod) = rest.strip_prefix("mod ") {
                let name: String = after_mod
                    .trim_start()
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                let tail = after_mod.trim_start()[name.len()..].trim_start();
                if !name.is_empty() && tail.starts_with(';') {
                    decls.push(ModDecl { name, path });
                }
            }
        }
        i = at + "#[cfg(".len();
    }
    decls
}

/// `cfg(test)` or `cfg(all(test, ..))` / `cfg(all(.., test, ..))`.
fn is_test_cfg(attr: &str) -> bool {
    let Some(inner) = attr.trim().strip_prefix("cfg(") else {
        return false;
    };
    let inner = inner.trim_end_matches(')').trim();
    if inner == "test" {
        return true;
    }
    if let Some(all) = inner.strip_prefix("all(") {
        return all
            .split(',')
            .any(|part| part.trim().trim_end_matches(')') == "test");
    }
    false
}

/// The string inside `path = "..."`.
fn path_attr_value(attr: &str) -> Option<String> {
    let rest = attr
        .trim()
        .strip_prefix("path")?
        .trim_start()
        .strip_prefix('=')?;
    let rest = rest.trim_start().strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Where Rust would look for the declared module's file.
///
/// With `#[path]`, the value is relative to the directory of the declaring
/// file (this crate never declares out-of-line test modules from inside an
/// inline `mod` block). Without it, `mod.rs`/`lib.rs` look in their own
/// directory; any other file `foo.rs` looks in `foo/`.
fn resolve_module_file(declaring: &Path, decl: &ModDecl) -> Vec<PathBuf> {
    let dir = declaring.parent().unwrap_or_else(|| Path::new(""));
    if let Some(p) = &decl.path {
        return vec![dir.join(p)];
    }
    let stem = declaring
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let base = if stem == "mod" || stem == "lib" || stem == "main" {
        dir.to_path_buf()
    } else {
        dir.join(&stem)
    };
    vec![
        base.join(format!("{}.rs", decl.name)),
        base.join(&decl.name).join("mod.rs"),
    ]
}

#[cfg(test)]
mod self_check {
    use super::*;

    #[test]
    fn finds_path_and_plain_declarations() {
        let src = "\
#[cfg(test)]
#[path = \"spi_tests.rs\"]
mod tests;

#[cfg(all(test, feature = \"event-scheduler\"))]
mod sched_tests;

#[cfg(test)]
mod inline { fn x() {} }

#[cfg(feature = \"jit\")]
mod not_test;
";
        let decls = cfg_test_module_decls(src);
        let names: Vec<(&str, Option<&str>)> = decls
            .iter()
            .map(|d| (d.name.as_str(), d.path.as_deref()))
            .collect();
        assert_eq!(
            names,
            vec![("tests", Some("spi_tests.rs")), ("sched_tests", None)]
        );
    }

    #[test]
    fn resolves_relative_to_declaring_file() {
        let decl = ModDecl {
            name: "tests".into(),
            path: Some("spi_tests.rs".into()),
        };
        assert_eq!(
            resolve_module_file(Path::new("src/peripherals/spi.rs"), &decl),
            vec![PathBuf::from("src/peripherals/spi_tests.rs")]
        );
        let plain = ModDecl {
            name: "pin_map_tests".into(),
            path: None,
        };
        assert_eq!(
            resolve_module_file(Path::new("src/bus/mod.rs"), &plain)[0],
            PathBuf::from("src/bus/pin_map_tests.rs")
        );
        assert_eq!(
            resolve_module_file(Path::new("src/bus/tick.rs"), &plain)[0],
            PathBuf::from("src/bus/tick/pin_map_tests.rs")
        );
    }
}
