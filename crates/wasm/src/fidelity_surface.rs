//! The browser must be told what the model could not model.
//!
//! Phases 3.1-3.3 built the census: `record_undecoded` / `record_unmapped` on
//! the silent paths, and `FidelityReport::to_gaps()` to flatten it for a
//! consumer. Then only the CLI read it — `to_gaps` had three callers, all under
//! `crates/cli`, and "fidelity" appeared in this crate only inside comments. The
//! engine knew exactly which instructions it had skipped, and the surface where
//! nearly every user actually runs a lab never said so.
//!
//! That matters because the failure is silent by construction: an undecoded
//! instruction is skipped with the registers left stale, which is
//! indistinguishable from firmware running correctly. The UADD8/SEL incident in
//! `fidelity.rs`'s own header is the canonical case — every C-string length came
//! out garbage and it took a deep debugging session to notice.
//!
//! These tests pin the two properties the browser surface depends on:
//!   1. a recorded gap is visible through the accessor, with its shape intact;
//!   2. the accessor is scoped to the current machine — the log is
//!      THREAD-LOCAL and outlives any one `WasmSimulator`, so without the reset
//!      in the constructors a second lab inherits the first one's gaps.
//!
//! Property 2 is the one that cannot be caught by reading the accessor alone,
//! and is why the reset lives in `new`/`new_from_config` rather than here.

#[cfg(test)]
mod tests {
    use crate::WasmSimulator;
    use labwired_core::fidelity;

    #[test]
    fn a_recorded_gap_is_visible_and_keeps_its_shape() {
        fidelity::reset();
        fidelity::record_undecoded(0x0800_0abc, 0xfa80_f040, "UADD8 (unmodelled)");
        fidelity::record_unmapped(0x4002_0000, "read");

        let gaps = fidelity::report().to_gaps();
        assert_eq!(gaps.len(), 2, "both gap kinds must survive to_gaps()");

        // unmapped_mmio comes first and carries an address but no opcode.
        let mmio = gaps
            .iter()
            .find(|g| g.kind == "unmapped_mmio")
            .expect("unmapped_mmio gap missing");
        assert_eq!(mmio.address.as_deref(), Some("0x40020000"));
        assert!(mmio.opcode.is_none(), "MMIO gap must not carry an opcode");
        assert_eq!(mmio.detail, "read");

        // undecoded_instruction carries an opcode and the PC of first sighting.
        let insn = gaps
            .iter()
            .find(|g| g.kind == "undecoded_instruction")
            .expect("undecoded_instruction gap missing");
        assert_eq!(insn.opcode.as_deref(), Some("0xfa80f040"));
        assert!(
            insn.address.is_none(),
            "instruction gap must not carry an address"
        );
        assert_eq!(insn.first_pc, "0x8000abc");
        assert_eq!(insn.count, 1);

        fidelity::reset();
    }

    /// `count` must be HITS, not distinct gaps — one unmodelled instruction in a
    /// hot loop is a far louder signal than a single sighting, and collapsing
    /// the two would hide exactly the case worth surfacing. The browser renders
    /// its total by summing this field, so the arithmetic has to hold here.
    #[test]
    fn repeated_hits_accumulate_rather_than_dedup_to_one() {
        fidelity::reset();
        for _ in 0..5 {
            fidelity::record_undecoded(0x0800_0abc, 0xfa80_f040, "UADD8 (unmodelled)");
        }
        let report = fidelity::report();
        assert_eq!(report.to_gaps().len(), 1, "same opcode is one gap entry");
        assert_eq!(report.to_gaps()[0].count, 5, "but five hits");
        assert_eq!(report.total_hits(), 5);
        fidelity::reset();
    }

    /// `report()` must NOT drain. A polling UI calls this repeatedly, and
    /// `take()` semantics would hand the gaps to whichever poll landed first and
    /// show nothing to the next — a warning that blinks out once is worse than
    /// no warning, because it teaches the reader that the panel is noise.
    #[test]
    fn reading_twice_returns_the_same_gaps() {
        fidelity::reset();
        fidelity::record_unmapped(0x4002_0000, "write");

        let first = fidelity::report().to_gaps();
        let second = fidelity::report().to_gaps();
        assert_eq!(first, second, "report() must not drain the log");
        assert_eq!(second.len(), 1);

        // ...whereas take() does, which is why the accessor does not use it.
        let drained = fidelity::take().to_gaps();
        assert_eq!(drained.len(), 1);
        assert!(
            fidelity::report().to_gaps().is_empty(),
            "take() is the draining one — kept here so the difference stays pinned"
        );
    }

    /// The scoping property. The log is thread-local and outlives any single
    /// machine, so a second lab opened in the same browser tab would otherwise
    /// show the first lab's gaps as its own.
    #[test]
    fn a_new_machine_does_not_inherit_the_previous_machines_gaps() {
        fidelity::reset();
        fidelity::record_undecoded(0x0800_0abc, 0xfa80_f040, "lab one's gap");
        assert_eq!(
            fidelity::report().total_hits(),
            1,
            "precondition: log is dirty"
        );

        // Drive the REAL constructor, not a stand-in that calls reset() itself —
        // the property under test is that `WasmSimulator::new` clears the log,
        // and a helper asserting `reset()` resets would pass with the production
        // line deleted.
        //
        // It must SUCCEED, so a committed Cortex-M ELF from this crate's own
        // fixtures is used rather than junk bytes. The error arms build a
        // `JsValue`, which aborts the process outside wasm (non-unwinding panic,
        // SIGABRT) — so a deliberately-invalid image cannot be used to reach
        // this assertion natively.
        let firmware = include_bytes!("../tests/fixtures/firmware-l476-bldc-six-step.elf");
        let sim = WasmSimulator::new(firmware).expect("committed l476 fixture must load");

        assert!(
            fidelity::report().to_gaps().is_empty(),
            "a freshly constructed machine must start with a clean fidelity log — \
             the thread-local outlives the machine, so without the reset in \
             WasmSimulator::new the next lab shows the previous lab's gaps"
        );

        // And the machine really was built — otherwise this test would pass on a
        // constructor that never ran.
        assert!(sim.get_pc().is_ok(), "constructed machine must be usable");
    }
}
