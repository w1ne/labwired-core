// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Holder for `bus_free` in `cpu/xtensa_lx7.rs`.
//!
//! Inside `step_batch` the Xtensa core remembers the bus's pending-IRQ word
//! across instructions and drops it after any instruction that could have
//! reached the bus. `bus_free` names the instructions that cannot. Listing one
//! that CAN is the unsafe direction: the core would keep a stale IRQ word and
//! take an interrupt late. So the list is re-derived here from the code it
//! describes:
//!
//! * the `execute` match maps each variant to the `exec_*` helper its arm
//!   calls, and
//! * a helper ignores the bus when its bus parameter is spelled `_bus` AND its
//!   body never names `_bus` (an underscore binding is still usable in Rust;
//!   the prefix only silences the lint).
//!
//! Every variant `bus_free` lists must map to a helper that ignores the bus.
//! Leaving a variant out is always allowed -- it only costs a re-poll.

use super::source_text::strip_comments_and_strings;
use std::collections::{BTreeMap, BTreeSet};

fn read(rel: &str) -> String {
    let path = super::src_roots::core_src(rel);
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    strip_comments_and_strings(&src)
}

fn exec_sources() -> Vec<String> {
    let dir = super::src_roots::core_src("cpu/xtensa_lx7/exec");
    let mut out = vec![read("cpu/xtensa_lx7.rs")];
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read_dir {dir:?}: {e}"))
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect();
    entries.sort();
    for p in entries {
        let src = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {p:?}: {e}"));
        out.push(strip_comments_and_strings(&src));
    }
    out
}

/// Index just past the brace block that opens at or after `from`.
fn block_end(s: &str, from: usize) -> usize {
    let b = s.as_bytes();
    let open = from + s[from..].find('{').expect("a brace block");
    let mut depth = 0i32;
    for (i, &c) in b.iter().enumerate().skip(open) {
        match c {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced braces from {from}");
}

fn is_ident_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Whole-identifier occurrence test.
fn names(hay: &str, ident: &str) -> bool {
    let b = hay.as_bytes();
    let mut at = 0;
    while let Some(i) = hay[at..].find(ident) {
        let s = at + i;
        let e = s + ident.len();
        let before = s == 0 || !is_ident_byte(b[s - 1]);
        let after = e >= b.len() || !is_ident_byte(b[e]);
        if before && after {
            return true;
        }
        at = e;
    }
    false
}

/// Every `fn exec_*(&mut self, <param>: ...)` with whether it ignores the bus.
fn helper_ignores_bus(sources: &[String]) -> BTreeMap<String, bool> {
    let mut out = BTreeMap::new();
    for s in sources {
        let mut at = 0;
        while let Some(i) = s[at..].find("fn exec_") {
            let start = at + i + 3;
            let name_end = start + s[start..].find('(').expect("helper signature");
            let name = s[start..name_end].trim().to_string();
            let after_self = name_end + s[name_end..].find("self,").expect("&mut self") + 5;
            let colon = after_self + s[after_self..].find(':').expect("param type");
            let param = s[after_self..colon].trim();
            let end = block_end(s, colon);
            let body_open = colon + s[colon..].find('{').expect("body");
            let body = &s[body_open..end];
            let ignores = param == "_bus" && !names(body, "_bus");
            out.insert(name, ignores);
            at = end;
        }
    }
    out
}

/// Variant -> the `exec_*` helper its arm in `execute`'s `match ins` calls.
fn execute_arms(lx7: &str) -> BTreeMap<String, Option<String>> {
    let f = lx7.find("fn execute(").expect("fn execute");
    let m = f + lx7[f..].find("match ins {").expect("match ins");
    let open = m + "match ins ".len();
    let end = block_end(lx7, m);
    let body = &lx7[open + 1..end - 1];
    let b = body.as_bytes();
    let mut out = BTreeMap::new();
    let (mut depth, mut arm_start, mut i) = (0i32, 0usize, 0usize);
    let mut pattern: Option<&str> = None;
    while i < b.len() {
        match b[i] {
            b'{' | b'(' | b'[' => depth += 1,
            b'}' | b')' | b']' => {
                depth -= 1;
                // A block-bodied arm ends at its own closing brace.
                if depth == 0 && pattern.is_some() && b[i] == b'}' {
                    let rest = body[i + 1..].trim_start();
                    if !rest.starts_with(',') {
                        record(&mut out, pattern.take().unwrap(), &body[arm_start..=i]);
                        arm_start = i + 1;
                    }
                }
            }
            b'=' if depth == 0 && pattern.is_none() && b.get(i + 1) == Some(&b'>') => {
                pattern = Some(&body[arm_start..i]);
                arm_start = i + 2;
                i += 1;
            }
            b',' if depth == 0 => {
                if let Some(p) = pattern.take() {
                    record(&mut out, p, &body[arm_start..i]);
                }
                arm_start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if let Some(p) = pattern.take() {
        record(&mut out, p, &body[arm_start..]);
    }
    out
}

fn record(out: &mut BTreeMap<String, Option<String>>, pattern: &str, expr: &str) {
    let helper = expr.find("self.exec_").map(|i| {
        let s = &expr[i + 5..];
        s[..s.find('(').expect("call")].trim().to_string()
    });
    for v in variant_names(pattern) {
        out.entry(v).or_insert_with(|| helper.clone());
    }
}

/// Upper-case identifiers at brace depth 0 of a pattern (field names inside
/// `{ .. }` are lower-case and at depth 1).
fn variant_names(pattern: &str) -> Vec<String> {
    let b = pattern.as_bytes();
    let (mut out, mut depth, mut i) = (Vec::new(), 0i32, 0usize);
    while i < b.len() {
        match b[i] {
            b'{' | b'(' => depth += 1,
            b'}' | b')' => depth -= 1,
            c if depth == 0 && c.is_ascii_uppercase() && (i == 0 || !is_ident_byte(b[i - 1])) => {
                let s = i;
                while i < b.len() && is_ident_byte(b[i]) {
                    i += 1;
                }
                out.push(pattern[s..i].to_string());
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    out
}

fn bus_free_list(lx7: &str) -> BTreeSet<String> {
    let f = lx7.find("fn bus_free(").expect("fn bus_free");
    let m = f + lx7[f..].find("matches!(").expect("matches!");
    let ins = m + lx7[m..].find("ins,").expect("ins,") + 4;
    let close = ins + lx7[ins..].find("\n    )").expect("end of matches!");
    variant_names(&lx7[ins..close]).into_iter().collect()
}

#[test]
fn bus_free_matches_the_helpers_that_ignore_the_bus() {
    let sources = exec_sources();
    let lx7 = &sources[0];
    let helpers = helper_ignores_bus(&sources);
    let arms = execute_arms(lx7);
    let listed = bus_free_list(lx7);

    // Anti-vacuity: a parser that silently found nothing would pass the loop
    // below. These floors are far under today's counts (131+ helpers, 165
    // arms, 139 listed) and far over zero.
    assert!(
        helpers.len() >= 100,
        "found only {} exec_* helpers",
        helpers.len()
    );
    assert!(arms.len() >= 150, "found only {} execute arms", arms.len());
    assert!(
        listed.len() >= 100,
        "found only {} bus_free variants",
        listed.len()
    );
    // And the classifier must be able to say "uses the bus": a load does.
    assert_eq!(
        helpers.get("exec_l32i"),
        Some(&false),
        "exec_l32i must classify as using the bus"
    );
    assert_eq!(
        helpers.get("exec_add"),
        Some(&true),
        "exec_add must classify as bus-free"
    );

    let mut wrong = Vec::new();
    for v in &listed {
        match arms.get(v) {
            None => wrong.push(format!("{v}: no arm in execute's match")),
            Some(None) => wrong.push(format!("{v}: its arm calls no exec_* helper")),
            Some(Some(h)) => match helpers.get(h) {
                None => wrong.push(format!("{v}: helper {h} not found")),
                Some(false) => wrong.push(format!("{v}: helper {h} can reach the bus")),
                Some(true) => {}
            },
        }
    }
    assert!(
        wrong.is_empty(),
        "bus_free lists instructions that can reach the bus -- the IRQ memo \
         would survive them and an interrupt could be taken late:\n  {}",
        wrong.join("\n  ")
    );
}

#[test]
fn the_helper_classifier_sees_an_underscore_binding_that_is_used() {
    // An underscore-prefixed parameter is still a usable binding.
    let src = strip_comments_and_strings(
        "impl X {\n    fn exec_a(&mut self, _bus: &mut dyn Bus) -> R { Ok(()) }\n    \
         fn exec_b(&mut self, _bus: &mut dyn Bus) -> R { _bus.read_u8(0)?; Ok(()) }\n    \
         fn exec_c(&mut self, bus: &mut dyn Bus) -> R { bus.read_u8(0)?; Ok(()) }\n}\n",
    );
    let h = helper_ignores_bus(&[src]);
    assert_eq!(h.get("exec_a"), Some(&true));
    assert_eq!(
        h.get("exec_b"),
        Some(&false),
        "a used `_bus` must count as using the bus"
    );
    assert_eq!(h.get("exec_c"), Some(&false));
}
