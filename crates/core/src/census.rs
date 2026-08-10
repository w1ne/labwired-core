// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Silent-path census — a **measurement-only** instrument, compiled out unless
//! the `silent-census` Cargo feature is enabled.
//!
//! # What this counts
//!
//! Three paths on which the simulator can be wrong while still reporting
//! success. None of them is fixed here; this module only counts how often each
//! is taken, so the follow-up repair work can be scoped from data instead of
//! guessed at.
//!
//! * **(a) `ArmDrop`** — the Cortex-M interpreter creates a
//!   [`crate::SimulationError::MemoryViolation`] on the bus and then throws it
//!   away (`let _ = bus.write_*`, or an `if let Ok(..) = bus.read_*` whose
//!   `Err` arm is the implicit empty else). Recorded with the PC of the
//!   instruction that did it, the faulting address, and read-vs-write.
//!   (For contrast, `cpu/riscv.rs` propagates these with `?`.)
//!
//! * **(b1) `StubFactoryFallthrough`** — the nine-stage peripheral factory
//!   chain in `bus/from_config.rs` fell through to `StubPeripheral`, announced
//!   only at `tracing::debug!`. Recorded with the manifest `type:` string that
//!   fell through. This is a statistic about **the factory**: how often the
//!   final `_other =>` arm was taken. It is *not* a statement about what the
//!   machine runs — see (b2).
//!
//! * **(b2) `StubLivePostConstruction`** — the peripherals that are *still* a
//!   `StubPeripheral` on a fully assembled machine, recorded with entry name
//!   and base address. **This is the actionable number.**
//!
//!   (b1) alone is misleading, and was measured wrongly once because of it.
//!   `from_config` is not the end of construction: `system::cortex_m::configure_cortex_m`
//!   runs afterwards on every ARM path and *replaces* the entry matching
//!   `name == "nvic" || base == 0xE000_E100` with a real [`crate::peripherals::nvic::Nvic`],
//!   and likewise `"scb"` / `0xE000_ED00` with a real [`crate::peripherals::scb::Scb`]
//!   (and DWT at `0xE000_1000`). A manifest carrying `type: nvic` therefore
//!   trips (b1) at the factory and then has a real model installed over it
//!   before a single instruction executes. The first published census read
//!   those (b1) rows as live stubs and called them "not intentional"; they were
//!   neither live nor stubs. The sweep runs at [`crate::Machine::new`] — the one
//!   choke every runner passes through with a finished bus — and identifies a
//!   stub by **`TypeId`**, via `Peripheral::as_any` + `dyn Any::is::<StubPeripheral>()`.
//!   Never by name: the replacement pass rewrites `name`, which is exactly how
//!   the first measurement went wrong.
//!
//! * **(c) `UndecodedReg`** — an MMIO offset a peripheral *does* claim but does
//!   not decode: a `_ => {}` write arm discards the value, a `_ => 0` read arm
//!   fabricates a zero. Recorded with peripheral name, offset, and
//!   read-vs-write. This is distinct from [`crate::fidelity`]'s
//!   `unmapped_mmio`, which fires only when *no* peripheral claims the address
//!   at all — the case counted here is invisible to that log, and is the one
//!   with the shipped production precedent (the STM32WB55 RCC layout bug: every
//!   clock-enable write landed on an undecoded offset and left TIM/I2C/SPI/ADC
//!   permanently gated off while blinky and UART validation stayed green).
//!
//! # Behavioural neutrality
//!
//! Every recording site is written so the feature-off expansion is *token-wise
//! identical* to the original code:
//!
//! * [`census_bus!`] expands to bare `$e` — the wrapped expression, untouched.
//! * [`census_reg!`] expands to `()` — a unit statement, so `_ => { census!(); }`
//!   is `_ => {}` and `_ => { census!(); 0 }` is `_ => 0`.
//! * [`record_stub`], [`record_live_stubs`] and [`dump_if_requested`] become
//!   empty `#[inline]` fns.
//! * `StubPeripheral`'s `as_any` override — the only production-type change the
//!   census needs — is itself `#[cfg(feature = "silent-census")]`, so with the
//!   feature off the type is byte-for-byte what it was. With the feature on it
//!   returns `Some(self)` where it used to return `None`; every existing
//!   consumer of `Peripheral::as_any` immediately `downcast_ref`s to a concrete
//!   type that a stub is not, so `Some(stub)` and `None` are indistinguishable
//!   to all of them.
//!
//! With the feature off the macro arguments are *not evaluated at all*, so a
//! recording site cannot introduce a side effect, a panic, or a borrow. With
//! the feature on the recorded values are read-only copies of values the
//! surrounding code already computed; no arm's control flow changes.
//!
//! # Gating
//!
//! `silent-census` is not in any crate's `default` feature set and is not
//! implied by any other feature. It must be named explicitly:
//!
//! ```text
//! cargo build -p labwired-cli --features silent-census
//! ```
//!
//! Enabling the feature alone still writes nothing: [`dump_if_requested`] only
//! emits a report when `LABWIRED_CENSUS_OUT` names a path. Both the compile-time
//! feature and the runtime env var must be set, so this cannot be switched on by
//! accident.

/// Record a dropped Cortex-M bus error around a bus access expression.
///
/// `census_bus!(self, "write", bus.write_u32(addr, val))` evaluates to exactly
/// what `bus.write_u32(addr, val)` evaluates to. With the feature on it also
/// records `(pc, addr, kind)` when — and only when — the result is
/// `Err(MemoryViolation(addr))`. The faulting address is taken out of the error
/// itself rather than re-derived at the call site, so the recorded address is
/// by construction the one the bus actually rejected.
#[cfg(feature = "silent-census")]
#[macro_export]
macro_rules! census_bus {
    ($cpu:expr, $kind:expr, $e:expr) => {{
        let __census_result = $e;
        if let Err($crate::SimulationError::MemoryViolation(__census_addr)) = &__census_result {
            $crate::census::record_arm_drop($cpu.pc, *__census_addr, $kind);
        }
        __census_result
    }};
}

/// Feature-off expansion: the wrapped expression, unchanged.
#[cfg(not(feature = "silent-census"))]
#[macro_export]
macro_rules! census_bus {
    ($cpu:expr, $kind:expr, $e:expr) => {
        $e
    };
}

/// Record an undecoded register offset from inside a catch-all match arm.
///
/// `census_reg!("rcc:F1", offset, "write")` evaluates to `()`.
#[cfg(feature = "silent-census")]
#[macro_export]
macro_rules! census_reg {
    ($periph:expr, $off:expr, $kind:expr) => {
        $crate::census::record_undecoded_reg($periph, $off as u64, $kind)
    };
}

/// Feature-off expansion: a unit expression that evaluates nothing.
#[cfg(not(feature = "silent-census"))]
#[macro_export]
macro_rules! census_reg {
    ($periph:expr, $off:expr, $kind:expr) => {
        ()
    };
}

#[cfg(feature = "silent-census")]
mod enabled {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    /// One dropped Cortex-M memory error, keyed by where and what.
    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
    pub struct ArmDropKey {
        /// PC of the instruction whose bus access was rejected.
        pub pc: u32,
        /// Address the bus refused.
        pub addr: u64,
        /// `"read"` or `"write"`.
        pub kind: &'static str,
    }

    /// One undecoded register access, keyed by peripheral, offset and direction.
    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
    pub struct RegKey {
        /// Static peripheral/model name, e.g. `"rcc:F1"`.
        pub periph: &'static str,
        /// Register offset within the peripheral.
        pub offset: u64,
        /// `"read"` or `"write"`.
        pub kind: &'static str,
    }

    /// Process-wide census tallies.
    ///
    /// Process-wide rather than thread-local (which is what [`crate::fidelity`]
    /// uses) because a census must aggregate the whole run regardless of which
    /// thread the sim loop landed on. The mutex is irrelevant to production
    /// cost: none of this exists unless the feature is compiled in.
    #[derive(Default, Debug)]
    pub struct Census {
        /// (a) Cortex-M memory errors created and then discarded.
        pub arm_drops: BTreeMap<ArmDropKey, u64>,
        /// (b1) Manifest `type:` strings that fell through to `StubPeripheral`
        /// at the factory. A count of how often the `_other =>` arm was taken —
        /// NOT a count of stubs the machine runs. See `live_stubs`.
        pub stub_types: BTreeMap<String, u64>,
        /// (b2) Peripherals that are still a `StubPeripheral` on a fully
        /// constructed machine, keyed by `(entry name, base address)`. This is
        /// the number that means something: it is measured after every
        /// post-factory replacement pass has run.
        pub live_stubs: BTreeMap<(String, u64), u64>,
        /// How many machines the (b2) sweep visited. Published so a reader can
        /// tell "one machine with three stubs" from "three machines with one
        /// each", and so a runner that builds a machine twice cannot inflate
        /// the table without the inflation being visible.
        pub machines_swept: u64,
        /// (c) Register offsets a peripheral claimed but did not decode, from
        /// the hand-written models whose decode is a `match` with a
        /// `_ => {}` / `_ => 0` arm. Keyed by `&'static str` so the hot arms
        /// allocate nothing.
        pub undecoded_regs: BTreeMap<RegKey, u64>,
        /// (c, declarative half) Same silent path, different code shape: a
        /// `GenericPeripheral` whose `reg_index_at(offset)` missed, so the read
        /// returned a fabricated `Ok(0)` or the write returned `Ok(())` having
        /// stored nothing. The peripheral name is data (the descriptor's
        /// `peripheral:` field), not a literal, so it needs an owned key.
        ///
        /// Tracked separately because a grep for `_ =>` arms — which is how the
        /// original audit sized counter (c) — cannot see this path at all.
        pub undecoded_regs_dyn: BTreeMap<(String, u64, &'static str), u64>,
    }

    static CENSUS: Mutex<Option<Census>> = Mutex::new(None);

    fn with<R>(f: impl FnOnce(&mut Census) -> R) -> R {
        let mut guard = CENSUS.lock().unwrap_or_else(|e| e.into_inner());
        f(guard.get_or_insert_with(Census::default))
    }

    pub fn record_arm_drop(pc: u32, addr: u64, kind: &'static str) {
        with(|c| {
            *c.arm_drops
                .entry(ArmDropKey { pc, addr, kind })
                .or_insert(0) += 1
        });
    }

    pub fn record_undecoded_reg(periph: &'static str, offset: u64, kind: &'static str) {
        with(|c| {
            *c.undecoded_regs
                .entry(RegKey {
                    periph,
                    offset,
                    kind,
                })
                .or_insert(0) += 1
        });
    }

    /// Record an undecoded offset on a declarative (`GenericPeripheral`) model,
    /// whose name is runtime data rather than a literal.
    pub fn record_undecoded_reg_named(periph: &str, offset: u64, kind: &'static str) {
        with(|c| {
            if let Some(n) = c
                .undecoded_regs_dyn
                .get_mut(&(periph.to_string(), offset, kind))
            {
                *n += 1;
            } else {
                c.undecoded_regs_dyn
                    .insert((periph.to_string(), offset, kind), 1);
            }
        });
    }

    pub fn record_stub(type_str: &str) {
        with(|c| *c.stub_types.entry(type_str.to_string()).or_insert(0) += 1);
    }

    /// (b2) Sweep a fully constructed machine's bus and record every entry
    /// whose device is *still* a [`crate::peripherals::stub::StubPeripheral`].
    ///
    /// Identity is decided by `TypeId` — `Peripheral::as_any()` followed by
    /// `dyn Any::is::<StubPeripheral>()`. That is exact: it cannot false-positive
    /// on some other model, and it cannot false-negative on a real stub, because
    /// `as_any` on the concrete type returns `Some(self)` and `Any::is` compares
    /// the concrete `TypeId`. Deliberately **not** a name test: the post-factory
    /// replacement passes rewrite `PeripheralEntry::name` (see the module docs),
    /// so a name is evidence of what the manifest asked for, never of what the
    /// machine ended up holding.
    ///
    /// The name and base are still *reported*, because they are how a human
    /// finds the entry again — they are output, not the predicate.
    pub fn record_live_stubs(bus: &crate::bus::SystemBus) {
        // Collect before taking the census lock: no lock is held across the
        // peripheral walk, and the walk touches nothing mutable.
        let found: Vec<(String, u64)> = bus
            .peripherals
            .iter()
            .filter(|p| {
                p.dev
                    .as_any()
                    .is_some_and(|a| a.is::<crate::peripherals::stub::StubPeripheral>())
            })
            .map(|p| (p.name.clone(), p.base))
            .collect();
        with(|c| {
            c.machines_swept += 1;
            for key in found {
                *c.live_stubs.entry(key).or_insert(0) += 1;
            }
        });
    }

    /// Serialise the census as JSON. Sorted throughout so two runs of the same
    /// firmware produce byte-identical reports.
    pub fn to_json() -> serde_json::Value {
        with(|c| {
            let arm: Vec<serde_json::Value> = c
                .arm_drops
                .iter()
                .map(|(k, n)| {
                    serde_json::json!({
                        "pc": format!("{:#010x}", k.pc),
                        "addr": format!("{:#010x}", k.addr),
                        "kind": k.kind,
                        "count": n,
                    })
                })
                .collect();
            let stubs: Vec<serde_json::Value> = c
                .stub_types
                .iter()
                .map(|(t, n)| serde_json::json!({ "type": t, "count": n }))
                .collect();
            let live_stubs: Vec<serde_json::Value> = c
                .live_stubs
                .iter()
                .map(|((name, base), n)| {
                    serde_json::json!({
                        "name": name,
                        "base": format!("{base:#010x}"),
                        "count": n,
                    })
                })
                .collect();
            let mut regs: Vec<serde_json::Value> = c
                .undecoded_regs
                .iter()
                .map(|(k, n)| {
                    serde_json::json!({
                        "peripheral": k.periph,
                        "offset": format!("{:#06x}", k.offset),
                        "kind": k.kind,
                        "count": n,
                        "shape": "match_arm",
                    })
                })
                .collect();
            regs.extend(c.undecoded_regs_dyn.iter().map(|((p, off, kind), n)| {
                serde_json::json!({
                    "peripheral": format!("declarative:{p}"),
                    "offset": format!("{off:#06x}"),
                    "kind": kind,
                    "count": n,
                    "shape": "declarative_miss",
                })
            }));
            let sum = |v: &Vec<serde_json::Value>| -> u64 {
                v.iter()
                    .map(|e| e["count"].as_u64().unwrap_or(0))
                    .sum::<u64>()
            };
            serde_json::json!({
                // (a) Cortex-M memory errors discarded instead of propagated.
                //
                // `instrumented_sites: 0` is load-bearing and must be read
                // before `total`. This arm was written against 64 discard sites
                // in `cpu/cortex_m.rs`; #897 propagated 62 of them with `?`, and
                // the last two — the reset-vector SP/PC reads — are named in
                // ALLOWED_DISCARDS, a shrink-only list keyed on the literal
                // source line, so wrapping them would break the guard that
                // protects them. Nothing is instrumented, so `total: 0` means
                // NOT MEASURED, not MEASURED CLEAN. A consumer that reads a
                // zero here as evidence of correctness is drawing a false
                // verdict from an absent instrument.
                //
                // The category stays rather than being deleted: `census_bus!`
                // and its value-transparency tests are what a future silent
                // discard would be caught with, and re-arming it is one wrap.
                "arm_dropped_memory_errors": {
                    "instrumented_sites": 0,
                    "distinct": arm.len(),
                    "total": sum(&arm),
                    "entries": arm,
                },
                // (b1) FACTORY-TIME: how often `from_config`'s `_other =>` arm
                // was taken. Construction statistic only — a `type:` here may
                // still have had a real model installed over it afterwards.
                "stub_factory_fallthrough": {
                    "distinct": stubs.len(),
                    "total": sum(&stubs),
                    "entries": stubs,
                },
                // (b2) POST-CONSTRUCTION: what is still a StubPeripheral on the
                // assembled machine. This is the actionable number.
                "stub_live_post_construction": {
                    "machines_swept": c.machines_swept,
                    "distinct": live_stubs.len(),
                    "total": sum(&live_stubs),
                    "entries": live_stubs,
                },
                "undecoded_register_access": {
                    "distinct": regs.len(),
                    "total": sum(&regs),
                    "entries": regs,
                },
            })
        })
    }

    /// Write the census to the path named by `LABWIRED_CENSUS_OUT`, if set.
    ///
    /// Failure to write is reported on stderr and otherwise ignored: a census
    /// is an observer, and must never change a run's exit status.
    pub fn dump_if_requested() {
        let Some(path) = std::env::var_os("LABWIRED_CENSUS_OUT") else {
            return;
        };
        let json = to_json();
        let text = serde_json::to_string_pretty(&json).unwrap_or_else(|_| "{}".to_string());
        if let Err(e) = std::fs::write(&path, text) {
            eprintln!("labwired census: cannot write {path:?}: {e}");
        }
    }

    /// Clear the tallies. Used by the census self-tests.
    pub fn reset() {
        with(|c| *c = Census::default());
    }
}

#[cfg(feature = "silent-census")]
pub use enabled::{
    dump_if_requested, record_arm_drop, record_live_stubs, record_stub, record_undecoded_reg,
    record_undecoded_reg_named, reset, to_json, Census,
};

#[cfg(not(feature = "silent-census"))]
mod disabled {
    /// No-op: the census is not compiled in.
    #[inline(always)]
    pub fn record_stub(_type_str: &str) {}

    /// No-op: the census is not compiled in.
    #[inline(always)]
    pub fn record_undecoded_reg_named(_periph: &str, _offset: u64, _kind: &'static str) {}

    /// No-op: the census is not compiled in. Takes the bus by shared reference
    /// and never touches it, so the single call site in `Machine::new` needs no
    /// `cfg` attribute and cannot perturb construction.
    #[inline(always)]
    pub fn record_live_stubs(_bus: &crate::bus::SystemBus) {}

    /// No-op: the census is not compiled in. Present unconditionally so the
    /// single call site on the CLI's run path needs no `cfg` attribute.
    #[inline(always)]
    pub fn dump_if_requested() {}
}

#[cfg(not(feature = "silent-census"))]
pub use disabled::{dump_if_requested, record_live_stubs, record_stub, record_undecoded_reg_named};

#[cfg(all(test, feature = "silent-census"))]
mod tests {
    use super::*;

    /// The census is deliberately process-global (a run must be aggregated
    /// whatever thread the sim loop landed on), so these tests — which call
    /// `reset()` and then assert absolute totals — must not run concurrently
    /// with each other under `cargo test`'s threaded harness.
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn serialized() -> std::sync::MutexGuard<'static, ()> {
        let g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        g
    }

    /// The macros must not perturb the value of the expression they wrap.
    #[test]
    fn census_bus_is_value_transparent() {
        let _guard = serialized();
        struct Cpu {
            pc: u32,
        }
        let cpu = Cpu { pc: 0x0800_1234 };

        let ok: crate::SimResult<()> = census_bus!(cpu, "write", Ok(()));
        assert!(ok.is_ok(), "Ok must pass through untouched");

        let err: crate::SimResult<u32> = census_bus!(
            cpu,
            "read",
            Err(crate::SimulationError::MemoryViolation(0x4002_0000))
        );
        assert!(matches!(
            err,
            Err(crate::SimulationError::MemoryViolation(0x4002_0000))
        ));

        let j = to_json();
        assert_eq!(j["arm_dropped_memory_errors"]["total"], 1);
        assert_eq!(
            j["arm_dropped_memory_errors"]["entries"][0]["pc"],
            "0x08001234"
        );
        assert_eq!(
            j["arm_dropped_memory_errors"]["entries"][0]["addr"],
            "0x40020000"
        );
        assert_eq!(j["arm_dropped_memory_errors"]["entries"][0]["kind"], "read");
    }

    /// A non-`MemoryViolation` error must not be counted as a dropped access.
    #[test]
    fn census_bus_ignores_other_errors() {
        let _guard = serialized();
        struct Cpu {
            pc: u32,
        }
        let cpu = Cpu { pc: 0x1000 };
        let e: crate::SimResult<u32> = census_bus!(
            cpu,
            "read",
            Err(crate::SimulationError::DecodeError(0xDEAD))
        );
        assert!(e.is_err());
        assert_eq!(to_json()["arm_dropped_memory_errors"]["total"], 0);
    }

    /// `census_reg!` must be a unit expression so `_ => { census_reg!(..); 0 }`
    /// keeps the arm's original type and value.
    #[test]
    fn census_reg_is_a_unit_expression_and_counts() {
        let _guard = serialized();
        let offset: u64 = 0x4C;
        let v = match offset {
            0x00 => 1u32,
            _ => {
                census_reg!("rcc:test", offset, "read");
                0
            }
        };
        assert_eq!(v, 0, "the arm must still produce its original value");

        let j = to_json();
        assert_eq!(j["undecoded_register_access"]["total"], 1);
        assert_eq!(
            j["undecoded_register_access"]["entries"][0]["peripheral"],
            "rcc:test"
        );
        assert_eq!(
            j["undecoded_register_access"]["entries"][0]["offset"],
            "0x004c"
        );
    }

    #[test]
    fn stub_types_are_histogrammed_by_string() {
        let _guard = serialized();
        record_stub("ssd1306");
        record_stub("ssd1306");
        record_stub("mystery-part");
        let j = to_json();
        assert_eq!(j["stub_factory_fallthrough"]["total"], 3);
        assert_eq!(j["stub_factory_fallthrough"]["distinct"], 2);
    }

    /// (b2) must be decided by concrete type, not by the entry's name, and must
    /// not be confused with (b1). A hand-built bus with one real model and one
    /// stub — the stub deliberately *named* like a real peripheral and the real
    /// model deliberately named like a stub — pins the discrimination.
    #[test]
    fn live_stub_sweep_keys_on_type_not_name() {
        let _guard = serialized();
        let mut bus = crate::bus::SystemBus::new();
        // A real model wearing a stub-ish name: must NOT be counted.
        bus.add_peripheral(
            "totally_a_stub",
            0x4000_0000,
            0x400,
            None,
            Box::new(crate::peripherals::rcc::Rcc::new()),
        );
        // A stub wearing a real peripheral's name: must be counted.
        bus.add_peripheral(
            "nvic",
            0xE000_E100,
            0x400,
            None,
            Box::new(crate::peripherals::stub::StubPeripheral::new(0)),
        );
        record_live_stubs(&bus);

        let j = to_json();
        assert_eq!(
            j["stub_live_post_construction"]["total"], 1,
            "exactly one entry on this bus is a StubPeripheral"
        );
        assert_eq!(j["stub_live_post_construction"]["machines_swept"], 1);
        let e = &j["stub_live_post_construction"]["entries"][0];
        assert_eq!(e["name"], "nvic");
        assert_eq!(e["base"], "0xe000e100");
        // …and the factory counter is untouched: (b1) and (b2) are separate
        // measurements and must never be read off one another.
        assert_eq!(j["stub_factory_fallthrough"]["total"], 0);
    }
}
