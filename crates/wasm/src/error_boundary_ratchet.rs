//! The wasm boundary must return errors, not `null`.
//!
//! A wasm-bindgen function that returns a bare `JsValue` has exactly one way to
//! say "I could not answer": `JsValue::NULL`. In the JS consumer `null` coerces
//! to `0`, so a failed register read is indistinguishable from a register that
//! genuinely reads zero, and a failed memory read is indistinguishable from a
//! page of real zeros. LabWired's whole claim is to be a trustworthy hardware
//! oracle; a silent wrong number is the worst failure it can have. This has
//! already produced false verdicts in this product.
//!
//! A function that returns `Result<T, JsValue>` cannot make that mistake:
//! wasm-bindgen surfaces `Err` as a thrown JS exception, which no consumer can
//! coerce to `0`. It is also the only *safe* way to fail here — a `panic!`
//! unwinds straight out of the wasm frame as a JS exception, and JS exceptions
//! do NOT run Rust destructors, so the wasm-bindgen borrow guard never drops
//! and every later call fails with "recursive use of an object". `Err` is an
//! ordinary Rust return: the guard drops first, the glue throws after.
//!
//! Converting all of it at once would be worse than not converting it, because
//! a half-converted boundary with two conventions is worse than a consistent
//! one and there is no way to review 80 signature changes honestly. So this is
//! a ratchet: the numbers below may only ever go DOWN, and every function that
//! is still failure-blind is NAMED in `BARE_JSVALUE_ACCESSORS` rather than left
//! silently pending.
//!
//! `WasmWorld` (`world.rs`) is the reference for the target convention — it has
//! returned `Result` for `get_pc`, `get_register`, `get_register_names`,
//! `read_memory` and `node_snapshot` since it was written.

/// The wasm-crate sources this ratchet scans, as `(file name, contents)`.
///
/// `include_str!` rather than a runtime directory walk: the count is then a
/// compile-time fact about the source that shipped, and the test cannot pass
/// vacuously because it was run from the wrong working directory.
const SOURCES: &[(&str, &str)] = &[
    ("inputs.rs", include_str!("inputs.rs")),
    ("inspect.rs", include_str!("inspect.rs")),
    ("install.rs", include_str!("install.rs")),
    ("jit_browser.rs", include_str!("jit_browser.rs")),
    ("lib.rs", include_str!("lib.rs")),
    ("playground_repro.rs", include_str!("playground_repro.rs")),
    ("traces.rs", include_str!("traces.rs")),
    ("world.rs", include_str!("world.rs")),
];

/// Every `pub fn` in the crate, as `(file, fn name, full signature)`.
///
/// Signatures are joined across lines up to the opening brace, so a multi-line
/// `-> Result<..>` (there are several) is not miscounted as failure-blind.
fn public_signatures() -> Vec<(&'static str, String, String)> {
    let mut out = Vec::new();
    for (file, src) in SOURCES {
        let lines: Vec<&str> = src.lines().collect();
        let mut i = 0;
        while i < lines.len() {
            let trimmed = lines[i].trim();
            if trimmed.starts_with("pub fn ") {
                let mut sig = trimmed.to_owned();
                let mut j = i;
                while !sig.contains('{') && j + 1 < lines.len() {
                    j += 1;
                    sig.push(' ');
                    sig.push_str(lines[j].trim());
                }
                let name = sig
                    .trim_start_matches("pub fn ")
                    .split('(')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                out.push((*file, name, sig));
                i = j;
            }
            i += 1;
        }
    }
    out
}

/// `pub fn`s whose return type cannot express failure — anything not returning
/// `Result`. These are the functions that must lie (a zero, an empty buffer, a
/// `null`) when they cannot answer.
fn failure_blind() -> Vec<(&'static str, String)> {
    public_signatures()
        .into_iter()
        .filter(|(_, _, sig)| !sig.contains("-> Result<"))
        .map(|(file, name, _)| (file, name))
        .collect()
}

/// `pub fn`s returning a bare `JsValue`, i.e. whose only failure channel is the
/// `null` that JS coerces to `0`.
fn bare_jsvalue_accessors() -> Vec<(&'static str, String)> {
    public_signatures()
        .into_iter()
        .filter(|(_, _, sig)| {
            let after = sig.rsplit("->").next().unwrap_or_default();
            after.trim().trim_end_matches('{').trim() == "JsValue"
        })
        .map(|(file, name, _)| (file, name))
        .collect()
}

/// Sites that hand `null` back to JS as the answer.
///
/// Only *returns* count: a tail `JsValue::NULL`, an explicit `return`, or the
/// `unwrap_or(JsValue::NULL)` that swallows a serialization failure. Passing
/// `JsValue::NULL` as an argument (the tests do, for absent config) is not a
/// null answer and is deliberately not counted.
fn null_return_sites() -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    for (file, src) in SOURCES {
        for line in src.lines() {
            let t = line.trim();
            if t == "JsValue::NULL"
                || t.contains("return JsValue::NULL")
                || t.contains("unwrap_or(JsValue::NULL)")
            {
                out.push((*file, t.to_owned()));
            }
        }
    }
    out
}

/// Accessors that still return a bare `JsValue`, each with what it feeds.
///
/// This is an exact set, not a floor: converting one and forgetting to strike
/// it here fails the test too, so the list cannot rot into a list of things
/// that were fixed years ago. Adding a new bare-`JsValue` accessor fails with
/// "not declared".
///
/// Ordered as the ratchet should be spent. The trace snapshots and the device
/// state lists are the cheapest (all six trace fns are a single
/// `unwrap_or(JsValue::NULL)` on serialization). `inspect` is the most
/// valuable and the most delicate: it is the register/memory display payload,
/// and `packages/ui/src/wasm/simulator-bridge.ts` currently turns its `null`
/// into an empty peripheral list, so a serialization failure renders as "this
/// board has no peripherals".
const BARE_JSVALUE_ACCESSORS: &[(&str, &str)] = &[
    ("inputs.rs", "get_i2c_sensor_states"),
    ("inspect.rs", "get_board_io_config"),
    ("inspect.rs", "get_board_io_states"),
    ("inspect.rs", "sample_logic_signals"),
    ("inspect.rs", "logic_wire_surface"),
    ("inspect.rs", "watch_logic_signals"),
    ("inspect.rs", "read_logic_edges"),
    ("inspect.rs", "pin_routing"),
    ("inspect.rs", "get_peripheral_snapshot"),
    ("inspect.rs", "get_adc_device_states"),
    ("inspect.rs", "get_board_io_analog_states"),
    ("inspect.rs", "get_actuator_states"),
    ("inspect.rs", "get_display"),
    ("inspect.rs", "get_spi_device_states"),
    ("inspect.rs", "get_uart_device_states"),
    ("inspect.rs", "get_peripheral_list"),
    ("inspect.rs", "inspect"),
    ("inspect.rs", "get_iolink_master_state"),
    ("traces.rs", "air_trace_snapshot"),
    ("traces.rs", "uart_trace_snapshot"),
    ("traces.rs", "wifi_trace_snapshot"),
    ("traces.rs", "fdcan_trace_snapshot"),
    ("traces.rs", "bus_trace_snapshot"),
    ("traces.rs", "iolink_trace_snapshot"),
    ("world.rs", "node_ids"),
    ("world.rs", "air_trace_snapshot"),
];

/// How many `pub fn`s at this boundary cannot report a failure at all.
///
/// May only ever go DOWN. If you are here because the test failed after adding
/// a function: return `Result<T, JsValue>`. There is no case at this boundary
/// where a caller is better served by a fabricated value than by a thrown
/// error, because the caller cannot tell the fabrication from data.
///
///  * 81 → 76 when the CPU inspector path (`get_pc`, `get_register`,
///    `get_register_names`, `read_memory`, `peek`) started returning `Result`.
///    `read_memory` was the worst of them: `bus.read_u8(..).unwrap_or(0)` made
///    a refused bus read byte-identical to real zeros, and that buffer is the
///    stack window the CPU inspector displays.
const FAILURE_BLIND_CEILING: usize = 76;

/// How many sites answer a JS caller with `null`.
///
/// May only ever go DOWN. Unchanged by the CPU-inspector conversion above,
/// which is the point of counting it separately: that work removed panicking
/// `.unwrap()`s and fabricated zeros, and left every `null` answer standing.
/// The 55 live behind the 26 accessors named in `BARE_JSVALUE_ACCESSORS` and
/// fall with them.
const NULL_RETURN_CEILING: usize = 55;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// One `file::fn` per line, so a failure names the work rather than a delta.
    fn render(items: &[(&'static str, String)]) -> String {
        items
            .iter()
            .map(|(file, name)| format!("  {file}::{name}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn failure_blind_boundary_only_shrinks() {
        let blind = failure_blind();
        let n = blind.len();
        assert!(
            n <= FAILURE_BLIND_CEILING,
            "the wasm boundary grew to {n} functions that cannot report failure \
             (ceiling {FAILURE_BLIND_CEILING}). A bare return type has to invent a value \
             when it fails, and JS cannot tell the invented value from data. Return \
             `Result<T, JsValue>` — see `world.rs` for the convention, and the doc comment \
             on FAILURE_BLIND_CEILING before changing it.\nStill failure-blind:\n{}",
            render(&blind)
        );
    }

    #[test]
    fn null_answers_only_shrink() {
        let sites = null_return_sites();
        let n = sites.len();
        assert!(
            n <= NULL_RETURN_CEILING,
            "the wasm boundary grew to {n} sites that answer JS with `null` \
             (ceiling {NULL_RETURN_CEILING}). `null` coerces to `0` in the consumer, so a \
             failed read becomes a register that reads zero. Return `Err(JsValue)` instead."
        );
    }

    /// The unconverted accessors are declared by name, so what is left is a
    /// list a reader can act on rather than a number.
    #[test]
    fn every_bare_jsvalue_accessor_is_declared() {
        let found: BTreeSet<(&str, String)> = bare_jsvalue_accessors().into_iter().collect();
        let declared: BTreeSet<(&str, String)> = BARE_JSVALUE_ACCESSORS
            .iter()
            .map(|(f, n)| (*f, (*n).to_owned()))
            .collect();

        let undeclared: Vec<_> = found.difference(&declared).collect();
        assert!(
            undeclared.is_empty(),
            "new bare-`JsValue` accessor(s) at the wasm boundary, not declared in \
             BARE_JSVALUE_ACCESSORS: {undeclared:?}. These can only report failure as \
             `null`, which JS reads as `0`. Return `Result<JsValue, JsValue>` instead of \
             adding to the list."
        );

        let stale: Vec<_> = declared.difference(&found).collect();
        assert!(
            stale.is_empty(),
            "BARE_JSVALUE_ACCESSORS names {stale:?}, which no longer return a bare \
             `JsValue`. Strike them from the list so the remaining debt stays honest — \
             this ratchet is an exact set, not a floor."
        );
    }

    /// A duplicate entry would let the declared set match while hiding a real
    /// accessor, the same way a symbol in two thunk lists hides one of them.
    #[test]
    fn no_accessor_is_declared_twice() {
        let unique: BTreeSet<_> = BARE_JSVALUE_ACCESSORS.iter().collect();
        assert_eq!(
            BARE_JSVALUE_ACCESSORS.len(),
            unique.len(),
            "BARE_JSVALUE_ACCESSORS lists the same (file, fn) twice"
        );
    }

    /// Guards the scanner itself: if `include_str!` or the signature join ever
    /// stopped matching, every count above would collapse to zero and all
    /// three ratchets would pass vacuously.
    #[test]
    fn the_scanner_actually_sees_the_boundary() {
        let sigs = public_signatures();
        assert!(
            sigs.len() > 100,
            "scanner found only {} public fns in the wasm crate — it has stopped matching, \
             so every ceiling above is vacuous",
            sigs.len()
        );
        // A function known to have been converted, and one known to be bare.
        let converted = sigs
            .iter()
            .find(|(f, n, _)| *f == "lib.rs" && n == "read_memory")
            .expect("lib.rs read_memory not found by the scanner");
        assert!(
            converted.2.contains("-> Result<"),
            "lib.rs read_memory is no longer Result-returning: {}",
            converted.2
        );
        assert!(
            bare_jsvalue_accessors()
                .iter()
                .any(|(f, n)| *f == "inspect.rs" && n == "inspect"),
            "scanner no longer recognises a known bare-JsValue accessor"
        );
    }
}
