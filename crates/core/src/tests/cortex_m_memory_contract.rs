// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Guard: the Cortex-M **load/store** path must honour the bus memory contract.
//!
//! `bus/accessors.rs` returns `Err(SimulationError::MemoryViolation(addr))` for
//! any address no memory region or peripheral window covers. The RISC-V core
//! propagates that with `?` at every access site. The Cortex-M core historically
//! did not: data loads used `if let Ok(val) = bus.read_*` (leaving the
//! destination register at its previous value) and data stores used
//! `let _ = bus.write_*` (vanishing entirely). Firmware that would fault on
//! silicon ran on with fabricated state and a green verdict.
//!
//! **How this differs from `examples/ci/dummy-memory-violation.yaml`.** That
//! fixture asserts `expected_stop_reason: memory_violation` and passes today —
//! but it gets there through the *instruction-fetch* path (`bus.read_u16(fetch_pc)?`
//! in `step_internal`), which has always propagated. It proves the stop-reason
//! plumbing works; it proves nothing about load/store. Every test below keeps
//! the PC inside mapped flash for the whole run and faults only on the *data*
//! address held in `Rn` — the half that was broken.
//!
//! Both directions are covered: an unmapped access must stop the run, and a
//! **valid** access must still succeed and leave the run alive. A change that
//! aborted on every access would pass a one-directional test.

#[cfg(test)]
mod no_discarded_bus_access {
    //! The enforceable half: a discarded bus access anywhere under
    //! `crates/core/src/cpu/**` fails this test.
    //!
    //! `#[deny(clippy::let_underscore_must_use)]` on the `cortex_m` and `riscv`
    //! modules (see `cpu/mod.rs`) already makes `let _ = bus.write_*` a compile
    //! error there under CI's `cargo clippy --all-targets -- -D warnings`. This
    //! scan covers the whole `cpu/` tree and also catches the two discard shapes
    //! clippy cannot see — `if bus.write_*(..).is_err() { log }` and
    //! `bus.read_*(..).ok()` — and it runs in the cheap `cargo test -p
    //! labwired-core --lib` lane, so both a clippy-less lane and a
    //! clippy-carrying lane catch a reintroduction.

    use std::path::{Path, PathBuf};

    /// The complete set of discarded bus accesses allowed to remain under
    /// `src/cpu/**`, keyed by file and matched on **exact line content** so a new
    /// discard can never hide behind a line-number shift or a budget count.
    ///
    /// SHRINK-ONLY. Every entry is named and reasoned:
    ///
    /// **`cortex_m.rs` — the 2 reset-vector reads in `Cpu::reset`.** Deliberately
    /// left tolerant, and the tolerance is load-bearing: making these propagate
    /// was tried and measured, and it flips `examples/ci/dummy-memory-violation.yaml`
    /// from `PASS / stop=memory_violation` to `FAIL / stop=halt`. That fixture's
    /// 1-byte flash means the vector table is unreadable, and a reset that returns
    /// `Err` is reported through a different path than a run-time violation. Boot
    /// is a separate contract from the load/store contract this module pins; it
    /// needs its own change and its own blast-radius work. Everything in
    /// `step_internal` — every data load and store, and exception stacking — now
    /// propagates.
    ///
    /// **`xtensa_lx7.rs` — 3 sites, all legitimate.** The list held 7 until the
    /// windowed register-overflow spill was fixed:
    ///   * ~~**4 were the same defect this change fixed in Cortex-M**: the
    ///     windowed register-overflow spill (~L778-781) wrote a0..a3 with
    ///     `let _ = bus.write_u32`, so the spill silently vanished if the spill
    ///     area was unmapped.~~ Fixed: `write4` now returns `SimResult<()>` and
    ///     `spill_call_preserve_to_stack` propagates to both of its callers
    ///     (`dispatch_irq`, `xthal_window_spill_thunk`), which already returned
    ///     `SimResult<()>`. Pinned by `tests/xtensa_memory_contract.rs`.
    ///   * **1 is legitimate**: `px_current_tcb` (~L1042) *probes* candidate BSS
    ///     addresses for a live FreeRTOS TCB pointer. An `Err` there means "not
    ///     this address", which is a real answer, not a fabricated one.
    ///   * **2 are legitimate**: the hot-BB JIT pre-reads (~L555/L559) *decline
    ///     the fast path* on `Err` (`return Ok(None)`) so the interpreter re-runs
    ///     the access and raises the genuine fault with full context. The error is
    ///     honoured by deferring, not dropped.
    const ALLOWED_DISCARDS: &[(&str, &[&str])] = &[
        (
            "cortex_m.rs",
            &[
                "if let Ok(sp) = bus.read_u32(vtor) {",
                "if let Ok(pc) = bus.read_u32(vtor + 4) {",
            ],
        ),
        (
            "xtensa_lx7.rs",
            &[
                "let b0 = match bus.read_u8(a3 as u64) {",
                "let b1 = match bus.read_u8((a3.wrapping_add(1)) as u64) {",
                "|base: u32| -> Option<u32> { bus.read_u32((base.wrapping_add(core * 4)) as u64).ok() };",
                // `raw_word_for_trace` is an OBSERVER: it re-reads the word at
                // the PC purely so a trace line can show it. It is on no
                // execution path and its value reaches no register. Turning a
                // lost trace word into a fault would mean switching tracing on
                // could change whether a run passes, which is the one thing an
                // observer must never do — so here the default IS the honest
                // answer, and it is why these two are allowed rather than fixed.
                "bus.read_u16(addr).map(u32::from).unwrap_or(0)",
                "bus.read_u32(addr).unwrap_or(0)",
            ],
        ),
    ];

    fn cpu_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/cpu")
    }

    fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        paths.sort();
        for p in paths {
            if p.is_dir() {
                rust_files(&p, out);
            } else if p.extension().is_some_and(|e| e == "rs") {
                out.push(p);
            }
        }
    }

    /// True if `line` discards the result of a bus access.
    ///
    /// Deliberately syntactic: it is a guard against a shape being reintroduced
    /// by a copy/paste, not a borrow checker.
    fn is_discarded_bus_access(line: &str) -> bool {
        let l = line.trim();
        if l.starts_with("//") || l.starts_with("///") || l.starts_with("*") {
            return false;
        }
        let touches_bus = l.contains("bus.read_u")
            || l.contains("bus.write_u")
            || l.contains("bus.read_")
            || l.contains("bus.write_");
        if !touches_bus {
            return false;
        }
        // `let _ = bus.write_u32(..)` / `let _ = bus.read_u8(..)`
        if l.starts_with("let _ =") || l.starts_with("let _:") {
            return true;
        }
        // `if bus.write_u32(..).is_err() { .. }` — the error is observed and
        // then dropped on the floor, which is the same fabricated fact.
        if l.contains(".is_err()") || l.contains(".is_ok()") {
            return true;
        }
        // `bus.read_u32(..).ok()` — Err collapsed to None and defaulted away.
        if l.contains(".ok()") {
            return true;
        }
        // `if let Ok(val) = bus.read_u32(..) { .. }` — the shape 41 of the
        // original 66 sites used. The Err arm either logs or does nothing, and
        // the destination register keeps whatever it held before. This is the
        // single most important shape to catch: it is invisible to clippy,
        // because nothing is being discarded as far as the type system can see.
        if l.contains("if let Ok(") || l.contains("match bus.read_") {
            return true;
        }
        // `bus.read_u32(..).unwrap_or(0)` — the Err is collapsed to a default
        // and the caller cannot tell a real 0 from a refused access, which is
        // precisely the fabricated fact every other arm here exists to catch.
        //
        // This arm was missing while the six above were present, so the one
        // spelling that reads most like ordinary Rust was the one spelling the
        // guard could not see. A regression reintroduced as `.unwrap_or(0)`
        // would have passed a green gate.
        if l.contains(".unwrap_or") {
            return true;
        }
        false
    }

    /// Everything a scan finds, as (file name, hit count, rendered hits).
    /// Hits are rendered as `path:line: <trimmed content>`.
    fn scan() -> Vec<(String, u32, Vec<String>)> {
        let mut files = Vec::new();
        rust_files(&cpu_root(), &mut files);
        assert!(
            !files.is_empty(),
            "found no .rs files under {} — the scanner is looking in the wrong \
             place and would pass vacuously",
            cpu_root().display()
        );
        let mut out = Vec::new();
        for f in files {
            let Ok(text) = std::fs::read_to_string(&f) else {
                continue;
            };
            let mut hits = Vec::new();
            let mut in_tests = false;
            for (i, line) in text.lines().enumerate() {
                // Test modules legitimately use `.unwrap()`/`let _`; the contract
                // is about the production execute paths.
                if line.trim_start().starts_with("#[cfg(test)]") {
                    in_tests = true;
                }
                if in_tests {
                    continue;
                }
                if is_discarded_bus_access(line) {
                    hits.push(format!("{}:{}: {}", f.display(), i + 1, line.trim()));
                }
            }
            if !hits.is_empty() {
                let name = f.file_name().unwrap().to_string_lossy().to_string();
                out.push((name, hits.len() as u32, hits));
            }
        }
        out
    }

    /// The trimmed source line out of a rendered `path:line: content` hit.
    fn content_of(hit: &str) -> &str {
        hit.splitn(3, ':').nth(2).unwrap_or(hit).trim()
    }

    fn allowed_for(file: &str) -> &'static [&'static str] {
        ALLOWED_DISCARDS
            .iter()
            .find(|(f, _)| *f == file)
            .map(|(_, lines)| *lines)
            .unwrap_or(&[])
    }

    /// The headline contract: nothing in the Cortex-M **execute path** discards a
    /// bus access. The only permitted hits in `cortex_m.rs` are the two
    /// reset-vector reads named in [`ALLOWED_DISCARDS`].
    #[test]
    fn cortex_m_execute_path_has_no_discarded_bus_accesses() {
        let found = scan();
        let hits: Vec<String> = found
            .iter()
            .filter(|(n, _, _)| n == "cortex_m.rs")
            .flat_map(|(_, _, h)| h.iter().cloned())
            .collect();
        let allowed = allowed_for("cortex_m.rs");
        let unexpected: Vec<&String> = hits
            .iter()
            .filter(|h| !allowed.contains(&content_of(h)))
            .collect();
        assert!(
            unexpected.is_empty(),
            "cortex_m.rs discards a bus access outside the two documented \
             reset-vector reads. `bus/accessors.rs` returns \
             Err(MemoryViolation) and a discard fabricates state — a failed load \
             leaves the destination register stale, a failed store vanishes, and \
             the run stays green. Use CortexM::load / CortexM::store, which \
             propagate with `?`. Found:\n{}",
            unexpected
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    /// The same contract for every other core under `src/cpu/**`, so the debt on
    /// `xtensa_lx7.rs` stays exactly where it is instead of growing quietly.
    #[test]
    fn cpu_module_discards_only_the_named_shrink_only_sites() {
        let found = scan();
        for (name, _, hits) in &found {
            let allowed = allowed_for(name);
            let unexpected: Vec<&String> = hits
                .iter()
                .filter(|h| !allowed.contains(&content_of(h)))
                .collect();
            assert!(
                unexpected.is_empty(),
                "{name} discards a bus access that is not in ALLOWED_DISCARDS. \
                 Either propagate the Err, or add the line with a written reason \
                 (the list is shrink-only). Found:\n{}",
                unexpected
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            );
        }
        // Shrink-only, enforced in the other direction: an allowlisted line that
        // no longer exists must be deleted from the list, so the list can never
        // quietly become a licence for a future discard.
        for (name, lines) in ALLOWED_DISCARDS {
            let hits: Vec<String> = found
                .iter()
                .filter(|(n, _, _)| n == name)
                .flat_map(|(_, _, h)| h.iter().cloned())
                .collect();
            for want in *lines {
                assert!(
                    hits.iter().any(|h| content_of(h) == *want),
                    "ALLOWED_DISCARDS still permits `{want}` in {name}, but the \
                     scan no longer finds it. Remove the entry — the list is \
                     shrink-only."
                );
            }
        }
    }

    /// ANTI-VACUITY. The scanner must actually recognise the shapes it bans —
    /// otherwise both tests above pass forever no matter what is committed.
    #[test]
    fn the_scanner_recognises_every_banned_shape() {
        for bad in [
            "let _ = bus.write_u32(addr as u64, val);",
            "                    let _ = bus.write_u8(addr, v);",
            "let _ = bus.read_u32(addr as u64);",
            "if bus.write_u32(addr as u64, val).is_err() { log(); }",
            "let v = bus.read_u32(addr).ok();",
            "if let Ok(val) = bus.read_u32(addr as u64) {",
            "match bus.read_u32(addr as u64) {",
        ] {
            assert!(
                is_discarded_bus_access(bad),
                "scanner failed to flag a banned shape: {bad}"
            );
        }
        for good in [
            "let val = self.load(bus, addr, AccessWidth::Word)?;",
            "self.store(bus, addr, AccessWidth::Word, val)?;",
            "let h1 = bus.read_u16(fetch_pc as u64)?;",
            "bus.write_u32(addr as u64, value)",
            "// let _ = bus.write_u32(addr, val); -- historical note",
        ] {
            assert!(
                !is_discarded_bus_access(good),
                "scanner wrongly flagged a propagating access: {good}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::cpu::CortexM;
    use crate::{Bus, Cpu, Machine, SimulationError};

    /// Address covered by no memory region and no peripheral window on a default
    /// `SystemBus` (flash 0x0000_0000..0x0010_0000, RAM 0x2000_0000..0x2010_0000,
    /// peripherals at 0x4000_C000 / 0x4001_0800 / 0x4002_1000 / 0xE000_E010).
    /// Also outside both bit-band alias windows.
    const UNMAPPED: u32 = 0x9000_0000;
    /// Mapped, writable RAM.
    const MAPPED: u32 = 0x2000_0100;
    /// Where the test instruction lives — always mapped, so instruction fetch
    /// never faults and only the data access can.
    const CODE: u32 = 0x1000;

    const SENTINEL: u32 = 0xDEAD_BEEF;

    fn machine_with_instr(instr: u16) -> Machine<CortexM> {
        let mut bus = crate::bus::SystemBus::new();
        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        bus.write_u16(CODE as u64, instr)
            .expect("code address must be mapped — otherwise this test would be a fetch test");
        let mut machine = Machine::new(cpu, bus);
        machine.cpu.set_pc(CODE);
        machine
    }

    /// `LDR r0, [r1, #0]` — T1 encoding, rn = r1, rt = r0.
    const LDR_R0_R1: u16 = 0x6808;
    /// `STR r0, [r1, #0]` — T1 encoding, rn = r1, rt = r0.
    const STR_R0_R1: u16 = 0x6008;

    #[test]
    fn cortex_m_load_from_unmapped_address_stops_the_run() {
        let mut machine = machine_with_instr(LDR_R0_R1);
        // The #880 abort contract is the opt-out behaviour now that fault
        // escalation defaults on (`LABWIRED_CORTEXM_FAULTS=0`); pin it
        // explicitly rather than depending on the process default.
        machine.cpu.set_faults_enabled(false);
        machine.cpu.set_register(1, UNMAPPED); // base address: unmapped
        machine.cpu.set_register(0, SENTINEL); // destination: pre-loaded sentinel

        let step = machine.step();

        assert!(
            matches!(step, Err(SimulationError::MemoryViolation(a)) if a == UNMAPPED as u64),
            "a load from unmapped 0x{UNMAPPED:08X} must surface the bus \
             MemoryViolation, not be discarded. got {step:?}; \
             r0 = 0x{:08X} (stale sentinel means the load was silently dropped), \
             pc = 0x{:08X}",
            machine.cpu.get_register(0),
            machine.cpu.get_pc(),
        );
    }

    #[test]
    fn cortex_m_store_to_unmapped_address_stops_the_run() {
        let mut machine = machine_with_instr(STR_R0_R1);
        // See the load twin above: the abort contract is pinned with fault
        // escalation explicitly off.
        machine.cpu.set_faults_enabled(false);
        machine.cpu.set_register(1, UNMAPPED); // base address: unmapped
        machine.cpu.set_register(0, SENTINEL);

        let step = machine.step();

        assert!(
            matches!(step, Err(SimulationError::MemoryViolation(a)) if a == UNMAPPED as u64),
            "a store to unmapped 0x{UNMAPPED:08X} must surface the bus \
             MemoryViolation, not vanish. got {step:?}; pc = 0x{:08X}",
            machine.cpu.get_pc(),
        );
    }

    #[test]
    fn cortex_m_load_from_mapped_address_still_succeeds() {
        let mut machine = machine_with_instr(LDR_R0_R1);
        machine
            .bus
            .write_u32(MAPPED as u64, 0xA5A5_1234)
            .expect("RAM write must succeed");
        machine.cpu.set_register(1, MAPPED);
        machine.cpu.set_register(0, SENTINEL);

        let step = machine.step();

        assert!(
            step.is_ok(),
            "a valid load must not abort the run: {step:?}"
        );
        assert_eq!(
            machine.cpu.get_register(0),
            0xA5A5_1234,
            "a valid load must actually deliver the value"
        );
        assert_eq!(
            machine.cpu.get_pc(),
            CODE + 2,
            "pc must advance past the LDR"
        );
    }

    #[test]
    fn cortex_m_store_to_mapped_address_still_succeeds() {
        let mut machine = machine_with_instr(STR_R0_R1);
        machine.cpu.set_register(1, MAPPED);
        machine.cpu.set_register(0, SENTINEL);

        let step = machine.step();

        assert!(
            step.is_ok(),
            "a valid store must not abort the run: {step:?}"
        );
        assert_eq!(
            machine.bus.read_u32(MAPPED as u64).unwrap(),
            SENTINEL,
            "a valid store must actually land in RAM"
        );
        assert_eq!(
            machine.cpu.get_pc(),
            CODE + 2,
            "pc must advance past the STR"
        );
    }

    /// The other direction, at run length: a loop of valid loads and stores must
    /// keep running for many steps. Catches an over-eager fix that aborts on
    /// every access, or one that faults on a legitimate peripheral access.
    #[test]
    fn cortex_m_repeated_valid_accesses_keep_the_run_alive() {
        let mut bus = crate::bus::SystemBus::new();
        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        // STR r0,[r1] ; LDR r2,[r1] ; B .-4   (branch back to CODE)
        bus.write_u16(CODE as u64, STR_R0_R1).unwrap();
        bus.write_u16((CODE + 2) as u64, 0x680A).unwrap(); // LDR r2,[r1,#0]
        bus.write_u16((CODE + 4) as u64, 0xE7FC).unwrap(); // B  -> CODE
        let mut machine = Machine::new(cpu, bus);
        machine.cpu.set_pc(CODE);
        machine.cpu.set_register(0, SENTINEL);
        machine.cpu.set_register(1, MAPPED);

        for i in 0..300 {
            machine
                .step()
                .unwrap_or_else(|e| panic!("valid access loop died at step {i}: {e:?}"));
        }
        assert_eq!(
            machine.cpu.get_register(2),
            SENTINEL,
            "the loop must have loaded back what it stored"
        );
    }

    /// A store into a real peripheral window (UART1 data register on the default
    /// bus) must remain `Ok`. Guards the "peripheral legitimately returns Err on
    /// a benign access" hazard.
    #[test]
    fn cortex_m_store_to_peripheral_window_still_succeeds() {
        let mut machine = machine_with_instr(STR_R0_R1);
        machine.cpu.set_register(1, 0x4000_C000); // uart1 base
        machine.cpu.set_register(0, 0x41); // 'A'

        let step = machine.step();

        assert!(
            step.is_ok(),
            "a store into a mapped peripheral window must not abort the run: {step:?}"
        );
    }
}
