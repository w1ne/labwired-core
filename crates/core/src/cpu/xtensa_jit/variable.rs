// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Xtensa LX7 JIT — generic (runtime-emitted) block wasmtime adapter
//! (#124 Phase 4.3).
//!
//! [`VariableBlock`] is the native twin of `labwired-wasm::jit_browser`'s
//! dispatch: it consumes an [`EmittedBlock`] with [`BlockAbi::Lx7Generic`]
//! (16-register ABI, five host imports), stages load values through the
//! live `Bus` *before* the call, drains committed stores after it, and
//! returns `(exit, target, r0..r15)`.
//!
//! The interpreter-identity tests at the bottom drive both sides: a
//! `XtensaLx7::step` run over the exact same seeded machine and a
//! manually-dispatched [`VariableBlock`] run, then compare the whole
//! register file, PC, CCOUNT, `branched`, and touched memory. That is
//! the strongest statement the native side can make about the browser
//! adapter, which is compile-verified only (`wasm32-unknown-unknown`)
//! because `js_sys::WebAssembly` has no native runtime.

#![cfg(feature = "jit")]

use std::sync::{Arc, Mutex};
use wasmtime::{Engine, Func, Instance, Module, Store, Val};

use super::emit_core::{BlockAbi, EmittedBlock, MemWidth, SideExitReason};

/// Compiled generic block. One `run` call executes one BB pass.
pub struct VariableBlock {
    store: Store<()>,
    run: Func,
    /// emit-core's view — `length_in_instrs`, `end_pc`, manifests.
    pub emitted: EmittedBlock,
    /// Pre-staged load values, dequeued by `host.read_u8` / `read_u32`.
    reads: Arc<Mutex<Vec<u32>>>,
    /// `(addr, value)` pairs handed back by `host.write_u8` / `write_u32`,
    /// committed by the caller via [`Self::drain_writes`].
    writes: Arc<Mutex<Vec<(u32, u32)>>>,
    /// Set when an import hit an empty staging queue (host-side bug or
    /// an under-staged block). Surfaced as [`Self::host_refused`].
    read_shortage: Arc<Mutex<bool>>,
    pub hits: u64,
}

/// Result of one generic block invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VariableResult {
    pub exit_code: i32,
    /// Branch destination when `exit_code == EXIT_BRANCH_TAKEN`.
    pub target_pc: u32,
    pub regs: [u32; 16],
}

impl VariableBlock {
    /// Compile + instantiate an emitted generic block. Refuses (as a
    /// wasmtime error) if handed the hand-baked hot-block ABI, since the
    /// import list would not match.
    pub fn build_from_emitted(engine: &Engine, emitted: EmittedBlock) -> wasmtime::Result<Self> {
        if emitted.abi != BlockAbi::Lx7Generic {
            return Err(wasmtime::Error::msg(
                "VariableBlock requires a BlockAbi::Lx7Generic emitted block",
            ));
        }
        let module = Module::new(engine, &emitted.wasm_bytes)?;
        let mut store: Store<()> = Store::new(engine, ());

        let reads: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(Vec::new()));
        let writes: Arc<Mutex<Vec<(u32, u32)>>> = Arc::new(Mutex::new(Vec::new()));
        let shortage: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));

        // host.read_u8 / read_u32: dequeue one staged value; -1 on an
        // empty queue is the refusal contract inherited from the hot
        // block (wasm side-exits with EXIT_HOST_BUS_ERROR). read_u32
        // returns the staged word unchanged; read_u8's wasm side masks
        // to 8 bits, mirroring the interpreter's zero-extend.
        let reads_u8 = reads.clone();
        let shortage_u8 = shortage.clone();
        let read_u8 = Func::wrap(&mut store, move |_addr: i32| -> i32 {
            let mut q = reads_u8.lock().unwrap();
            if q.is_empty() {
                *shortage_u8.lock().unwrap() = true;
                return -1;
            }
            q.remove(0) as i32
        });
        let reads_u32 = reads.clone();
        let shortage_u32 = shortage.clone();
        let read_u32 = Func::wrap(&mut store, move |_addr: i32| -> i32 {
            let mut q = reads_u32.lock().unwrap();
            if q.is_empty() {
                *shortage_u32.lock().unwrap() = true;
                return -1;
            }
            q.remove(0) as i32
        });

        // Stores queue `(addr, value)` for the caller to commit through
        // the `Bus` after a successful run; returning 0 keeps the
        // "negative status = refusal" contract available.
        let writes_u8 = writes.clone();
        let write_u8 = Func::wrap(&mut store, move |addr: i32, val: i32| -> i32 {
            writes_u8.lock().unwrap().push((addr as u32, val as u32));
            0
        });
        let writes_u32 = writes.clone();
        let write_u32 = Func::wrap(&mut store, move |addr: i32, val: i32| -> i32 {
            writes_u32.lock().unwrap().push((addr as u32, val as u32));
            0
        });

        // branch_target(pc, decoder_prebiased_offset) — the host owns
        // the Xtensa "taken PC" arithmetic so wasm never re-derives it.
        let branch_target = Func::wrap(&mut store, move |pc: i32, offset: i32| -> i32 {
            (pc as u32).wrapping_add(offset as u32) as i32
        });

        let instance = Instance::new(
            &mut store,
            &module,
            &[
                read_u8.into(),
                read_u32.into(),
                write_u8.into(),
                write_u32.into(),
                branch_target.into(),
            ],
        )?;
        let run = instance
            .get_func(&mut store, "run")
            .ok_or_else(|| wasmtime::Error::msg("generic block module has no `run` export"))?;

        Ok(Self {
            store,
            run,
            emitted,
            reads,
            writes,
            read_shortage: shortage,
            hits: 0,
        })
    }

    /// Pre-read every manifest load from `bus` using the current register
    /// file, exactly like the browser dispatcher does. Any bus error
    /// refuses the whole step (the caller falls back to the interpreter
    /// so the genuine fault surfaces with full context).
    pub fn resolve_loads(&self, bus: &dyn crate::Bus, regs: &[u32; 16]) -> crate::SimResult<()> {
        let mut staged = Vec::with_capacity(self.emitted.loads.len());
        for req in &self.emitted.loads {
            let addr = regs[req.base as usize].wrapping_add(req.imm) as u64;
            let val = match req.width {
                MemWidth::U8 => bus.read_u8(addr)? as u32,
                MemWidth::U32 => bus.read_u32(addr)?,
            };
            staged.push(val);
        }
        let mut q = self.reads.lock().unwrap();
        q.clear();
        q.extend(staged);
        *self.read_shortage.lock().unwrap() = false;
        let mut w = self.writes.lock().unwrap();
        w.clear();
        Ok(())
    }

    /// Run the compiled body with a full register-file snapshot.
    pub fn run(&mut self, regs: &[u32; 16]) -> wasmtime::Result<VariableResult> {
        let mut results = vec![Val::I32(0); 18];
        let params: Vec<Val> = regs.iter().map(|r| Val::I32(*r as i32)).collect();
        self.run.call(&mut self.store, &params, &mut results)?;
        let mut out = [0u32; 16];
        for (i, slot) in out.iter_mut().enumerate() {
            match results[i + 2] {
                Val::I32(v) => *slot = v as u32,
                ref other => {
                    return Err(wasmtime::Error::msg(format!(
                        "generic block returned non-i32 register slot: {other:?}"
                    )))
                }
            }
        }
        let exit_code = match results[0] {
            Val::I32(v) => v,
            ref other => {
                return Err(wasmtime::Error::msg(format!(
                    "generic block returned non-i32 exit code: {other:?}"
                )))
            }
        };
        let target_pc = match results[1] {
            Val::I32(v) => v as u32,
            ref other => {
                return Err(wasmtime::Error::msg(format!(
                    "generic block returned non-i32 target: {other:?}"
                )))
            }
        };
        self.hits += 1;
        Ok(VariableResult {
            exit_code,
            target_pc,
            regs: out,
        })
    }

    /// Drain `(addr, value)` pairs the body queued, in execution order.
    pub fn drain_writes(&self) -> Vec<(u32, u32)> {
        std::mem::take(&mut *self.writes.lock().unwrap())
    }

    /// True iff an import ran out of staged values during the last run.
    pub fn host_refused(&self) -> bool {
        *self.read_shortage.lock().unwrap()
    }

    /// Map a wire exit code to the runtime-agnostic reason vocabulary.
    pub fn classify_exit(&self, exit_code: i32) -> Option<SideExitReason> {
        self.emitted.reason_for(exit_code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SystemBus;
    use crate::cpu::xtensa_jit::emit_core::{walk_and_emit, PsBits};
    use crate::cpu::xtensa_jit_bytes::{
        EXIT_BRANCH_TAKEN, EXIT_FALL_THROUGH, EXIT_HOST_BUS_ERROR, EXIT_JUMP_TAKEN,
    };
    use crate::cpu::xtensa_lx7::XtensaLx7;
    use crate::cpu::xtensa_sr::CCOUNT;
    use crate::{Bus, Cpu, SimulationConfig};

    const TEST_PC: u32 = 0x2000_0000;
    const DATA: u64 = 0x2000_0100;

    fn le3(word: u32) -> [u8; 3] {
        [
            (word & 0xFF) as u8,
            ((word >> 8) & 0xFF) as u8,
            ((word >> 16) & 0xFF) as u8,
        ]
    }

    // Wide instruction encoders (same layouts the decoder was written
    // against; cross-checked against `tests/xtensa_exec.rs`).
    fn enc_addi(at: u32, as_: u32, imm8: i32) -> u32 {
        0x2 | (at << 4) | (as_ << 8) | (0xC << 12) | (((imm8 as u32) & 0xFF) << 16)
    }
    fn enc_l32i(at: u32, as_: u32, byte_off: u32) -> u32 {
        0x2 | (at << 4) | (as_ << 8) | (0x2 << 12) | (((byte_off >> 2) & 0xFF) << 16)
    }
    fn enc_s32i(at: u32, as_: u32, byte_off: u32) -> u32 {
        0x2 | (at << 4) | (as_ << 8) | (0x6 << 12) | (((byte_off >> 2) & 0xFF) << 16)
    }
    /// BRI12 conditional branch (BZ family, n=1): `m` selects the op
    /// (1 = BNEZ), `s` the tested register, `pc` this branch's address.
    fn enc_bz(m: u32, s: u32, pc: u32, target: u32) -> u32 {
        let offset = target.wrapping_sub(pc) as i32;
        let imm12 = ((offset - 4) as u32) & 0xFFF;
        0x6 | (1 << 4) | (m << 6) | (s << 8) | (imm12 << 12)
    }
    /// `J` (SI format, n=0): imm18 at bits[23:6], decoder pre-biases +4.
    fn enc_j(pc: u32, target: u32) -> u32 {
        let offset = target.wrapping_sub(pc) as i32;
        let imm18 = ((offset - 4) as u32) & 0x3_FFFF;
        0x6 | (imm18 << 6)
    }

    fn write_code(bus: &mut SystemBus, addr: u64, words: &[u32]) {
        let mut off = 0u64;
        for &w in words {
            for b in le3(w) {
                bus.write_u8(addr + off, b).unwrap();
                off += 1;
            }
        }
    }

    fn code_bytes(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| le3(*w)).collect()
    }

    fn seed(cpu: &mut XtensaLx7, regs: &[(u8, u32)]) {
        for &(r, v) in regs {
            cpu.regs.write_logical(r, v);
        }
    }

    fn emit(words: &[u32]) -> EmittedBlock {
        let bytes = code_bytes(words);
        walk_and_emit(
            &bytes,
            TEST_PC,
            |pc| {
                let off = pc.wrapping_sub(TEST_PC) as usize;
                if off < bytes.len() {
                    Some(off)
                } else {
                    None
                }
            },
            PsBits::default(),
        )
        .expect("generic emit")
    }

    fn fresh_cpu(bus: &mut SystemBus, regs: &[(u8, u32)]) -> XtensaLx7 {
        let mut cpu = XtensaLx7::new();
        cpu.reset(bus).unwrap();
        cpu.set_pc(TEST_PC);
        seed(&mut cpu, regs);
        cpu
    }

    fn seed_bus(words: &[u32], data: &[(u64, u32)]) -> SystemBus {
        let mut bus = SystemBus::new();
        write_code(&mut bus, TEST_PC as u64, words);
        for &(addr, v) in data {
            bus.write_u32(addr, v).unwrap();
        }
        bus
    }

    /// Interpreter path: `steps` instructions from `TEST_PC`.
    fn run_interp(
        words: &[u32],
        regs: &[(u8, u32)],
        data: &[(u64, u32)],
        steps: u32,
    ) -> (XtensaLx7, SystemBus) {
        let mut bus = seed_bus(words, data);
        let mut cpu = fresh_cpu(&mut bus, regs);
        for _ in 0..steps {
            cpu.step(&mut bus, &[], &SimulationConfig::default())
                .expect("interpreter step");
        }
        (cpu, bus)
    }

    /// JIT path: the same commit sequence the browser dispatcher performs.
    fn run_jit(words: &[u32], regs: &[(u8, u32)], data: &[(u64, u32)]) -> (XtensaLx7, SystemBus) {
        let emitted = emit(words);
        let mut bus = seed_bus(words, data);
        let mut cpu = fresh_cpu(&mut bus, regs);

        let mut snapshot = [0u32; 16];
        for (i, slot) in snapshot.iter_mut().enumerate() {
            *slot = cpu.regs.read_logical(i as u8);
        }

        let engine = Engine::default();
        let mut block =
            VariableBlock::build_from_emitted(&engine, emitted).expect("compile generic");
        block.resolve_loads(&bus, &snapshot).expect("staged loads");
        let res = block.run(&snapshot).expect("wasm call");
        assert!(!block.host_refused(), "load queue under-staged");

        // Commit stores in manifest order, exactly like the dispatcher.
        let writes = block.drain_writes();
        for (req, (addr, val)) in block.emitted.stores.iter().zip(writes) {
            match req.width {
                MemWidth::U8 => bus.write_u8(addr as u64, val as u8).unwrap(),
                MemWidth::U32 => bus.write_u32(addr as u64, val).unwrap(),
            }
        }

        for (i, v) in res.regs.iter().enumerate() {
            cpu.regs.write_logical(i as u8, *v);
        }
        // Mirror the dispatcher's post-block bookkeeping: PC at the exit
        // destination, `branched` set on the taken arm, CCOUNT advanced by
        // the full instruction count (the outer step counted one; the JIT
        // adds the remaining `length - 1`).
        let length = block.emitted.length_in_instrs;
        match res.exit_code {
            x if x == EXIT_FALL_THROUGH => {
                cpu.pc = block.emitted.end_pc;
                cpu.branched = false;
            }
            x if x == EXIT_BRANCH_TAKEN => {
                cpu.pc = res.target_pc;
                cpu.branched = true;
            }
            // `J` advances PC but does not set `branched`, matching the
            // interpreter's `J` arm (it matters for zero-overhead loops).
            x if x == EXIT_JUMP_TAKEN => {
                cpu.pc = res.target_pc;
                cpu.branched = false;
            }
            other => panic!("unexpected JIT exit code {other}"),
        }
        let cc = cpu.sr.read(CCOUNT);
        cpu.sr.write(CCOUNT, cc.wrapping_add(length));

        (cpu, bus)
    }

    /// Whole-machine comparison: every logical register, PC, CCOUNT,
    /// `branched`, and the touched RAM word.
    fn assert_same_state(interp: &XtensaLx7, ibus: &SystemBus, jit: &XtensaLx7, jbus: &SystemBus) {
        for i in 0..16u8 {
            assert_eq!(
                jit.regs.read_logical(i),
                interp.regs.read_logical(i),
                "a{i} diverged"
            );
        }
        assert_eq!(jit.pc, interp.pc, "PC diverged");
        assert_eq!(jit.branched, interp.branched, "`branched` diverged");
        assert_eq!(
            jit.sr.read(CCOUNT),
            interp.sr.read(CCOUNT),
            "CCOUNT diverged"
        );
        assert_eq!(
            jbus.read_u32(DATA).unwrap(),
            ibus.read_u32(DATA).unwrap(),
            "stored word diverged"
        );
    }

    /// Shape B: ALU + conditional branch, branch **taken** (loops back to
    /// the block start). Whole register file, PC, CCOUNT and `branched`
    /// must match a pure-interpreter run of the same two instructions.
    #[test]
    fn alu_branch_taken_matches_interpreter() {
        // addi a2,a2,7 ; bnez a2, TEST_PC (taken: 5+7 != 0)
        let words = [enc_addi(2, 2, 7), enc_bz(1, 2, TEST_PC + 3, TEST_PC)];
        let regs = [(2, 5), (3, 0xAAAA_AAAA), (7, 0x1234_5678)];
        let (interp, ibus) = run_interp(&words, &regs, &[(DATA, 0)], 2);
        let (jit, jbus) = run_jit(&words, &regs, &[(DATA, 0)]);

        assert_eq!(emit(&words).abi, BlockAbi::Lx7Generic);
        assert_eq!(emit(&words).length_in_instrs, 2);
        assert_eq!(jit.regs.read_logical(2), 12);
        assert_eq!(jit.pc, TEST_PC, "branch taken loops back");
        assert!(jit.branched);
        assert_same_state(&interp, &ibus, &jit, &jbus);
    }

    /// Shape B with the branch **not** taken (fall-through).
    #[test]
    fn alu_branch_fallthrough_matches_interpreter() {
        // addi a2,a2,7 ; bnez a2, TEST_PC (not taken: -7+7 == 0)
        let words = [enc_addi(2, 2, 7), enc_bz(1, 2, TEST_PC + 3, TEST_PC)];
        let regs = [(2, (-7i32) as u32), (4, 99)];
        let (interp, ibus) = run_interp(&words, &regs, &[(DATA, 0)], 2);
        let (jit, jbus) = run_jit(&words, &regs, &[(DATA, 0)]);

        assert_eq!(emit(&words).end_pc, TEST_PC + 6);
        assert_eq!(jit.pc, TEST_PC + 6, "branch not taken falls through");
        assert!(!jit.branched);
        assert_same_state(&interp, &ibus, &jit, &jbus);
    }

    /// Shape C: 32-bit load/store + branch, branch taken. Exercises
    /// `read_u32` / `write_u32` staging and the store commit path.
    #[test]
    fn load_store_branch_taken_matches_interpreter() {
        // l32i a5,a4,0 ; addi a5,a5,1 ; s32i a5,a4,0 ; addi a4,a4,-4 ;
        // bnez a4, TEST_PC
        let words = [
            enc_l32i(5, 4, 0),
            enc_addi(5, 5, 1),
            enc_s32i(5, 4, 0),
            enc_addi(4, 4, -4),
            enc_bz(1, 4, TEST_PC + 12, TEST_PC),
        ];
        let regs = [(4, DATA as u32), (6, 0xFEED_F00D)];
        let data = [(DATA, 0x0000_0100u32)];

        let (interp, ibus) = run_interp(&words, &regs, &data, 5);
        let (jit, jbus) = run_jit(&words, &regs, &data);

        let emitted = emit(&words);
        assert_eq!(emitted.length_in_instrs, 5);
        assert_eq!(emitted.loads.len(), 1, "one L32I in the manifest");
        assert_eq!(emitted.stores.len(), 1, "one S32I in the manifest");
        assert_eq!(
            jit.regs.read_logical(5),
            0x101,
            "loaded, incremented, stored"
        );
        assert_eq!(jit.pc, TEST_PC, "branch taken loops back");
        assert!(jit.branched);
        assert_eq!(jbus.read_u32(DATA).unwrap(), 0x101, "store committed");
        assert_same_state(&interp, &ibus, &jit, &jbus);
    }

    /// Shape C with the branch not taken: the branch tests a *separate*
    /// counter register so the load/store base stays inside RAM.
    #[test]
    fn load_store_branch_fallthrough_matches_interpreter() {
        // l32i a5,a4,0 ; addi a5,a5,1 ; s32i a5,a4,0 ; addi a7,a7,-4 ;
        // bnez a7, TEST_PC
        let words = [
            enc_l32i(5, 4, 0),
            enc_addi(5, 5, 1),
            enc_s32i(5, 4, 0),
            enc_addi(7, 7, -4),
            enc_bz(1, 7, TEST_PC + 12, TEST_PC),
        ];
        let regs = [(4, DATA as u32), (7, 4)];
        let data = [(DATA, 0x0000_00FFu32)];

        let (interp, ibus) = run_interp(&words, &regs, &data, 5);
        let (jit, jbus) = run_jit(&words, &regs, &data);

        assert_eq!(emit(&words).end_pc, TEST_PC + 15);
        assert_eq!(jit.pc, TEST_PC + 15, "branch not taken falls through");
        assert!(!jit.branched);
        assert_eq!(jbus.read_u32(DATA).unwrap(), 0x100);
        assert_same_state(&interp, &ibus, &jit, &jbus);
    }

    /// `J` terminator: PC advances to the jump target but `branched` stays
    /// clear, mirroring the interpreter's `J` arm (only `branch()` sets the
    /// flag, and the zero-overhead-loop check reads it).
    #[test]
    fn jump_taken_matches_interpreter() {
        // addi a2,a2,1 ; j TEST_PC
        let words = [enc_addi(2, 2, 1), enc_j(TEST_PC + 3, TEST_PC)];
        let regs = [(2, 5), (4, 0xDEAD_BEEF)];
        let (interp, ibus) = run_interp(&words, &regs, &[(DATA, 0)], 2);
        let (jit, jbus) = run_jit(&words, &regs, &[(DATA, 0)]);

        assert_eq!(emit(&words).length_in_instrs, 2);
        assert_eq!(jit.regs.read_logical(2), 6);
        assert_eq!(jit.pc, TEST_PC, "PC must land on the jump target");
        assert!(!jit.branched, "interpreter `J` leaves branched clear");
        assert_same_state(&interp, &ibus, &jit, &jbus);
    }

    /// Under-staged loads must side-exit with EXIT_HOST_BUS_ERROR and
    /// leave the caller's state uncommitted (the adapter signals refusal
    /// via the exit code; the dispatcher checks it before writing back).
    #[test]
    fn unstaged_load_signals_bus_error() {
        let words = [
            enc_l32i(5, 4, 0),
            enc_addi(5, 5, 1),
            enc_s32i(5, 4, 0),
            enc_addi(4, 4, -4),
            enc_bz(1, 4, TEST_PC + 12, TEST_PC),
        ];
        let engine = Engine::default();
        let mut block = VariableBlock::build_from_emitted(&engine, emit(&words)).unwrap();
        // Deliberately stage nothing: the first read_u32 depletes the
        // queue and the body exits with the bus-error wire code.
        let res = block.run(&[0u32; 16]).unwrap();
        assert_eq!(res.exit_code, EXIT_HOST_BUS_ERROR);
        assert!(block.host_refused());
        assert_eq!(
            block.classify_exit(EXIT_HOST_BUS_ERROR),
            Some(SideExitReason::HostBusError)
        );
    }

    /// The generic adapter refuses the hand-baked hot-block ABI instead
    /// of instantiating a module whose import list doesn't match.
    #[test]
    fn generic_adapter_refuses_hot_abi() {
        let hot = EmittedBlock {
            wasm_bytes: crate::cpu::xtensa_jit_bytes::HOT_BB_WASM.to_vec(),
            abi: BlockAbi::HotBb,
            length_in_instrs: crate::cpu::xtensa_jit::HOT_BB_INSTR_COUNT,
            end_pc: crate::cpu::xtensa_jit::HOT_BB_END,
            loads: Vec::new(),
            stores: Vec::new(),
            side_exit_reasons: Vec::new(),
        };
        let engine = Engine::default();
        assert!(VariableBlock::build_from_emitted(&engine, hot).is_err());
    }

    /// Reject never: a generic block with no memory ops still classifies
    /// its fall-through arm and reports the manifest as empty.
    #[test]
    fn classification_exit_codes() {
        let words = [enc_addi(2, 2, 7), enc_bz(1, 2, TEST_PC + 3, TEST_PC)];
        let engine = Engine::default();
        let block = VariableBlock::build_from_emitted(&engine, emit(&words)).unwrap();
        assert_eq!(
            block.classify_exit(EXIT_FALL_THROUGH),
            Some(SideExitReason::FallThrough)
        );
        assert_eq!(
            block.classify_exit(EXIT_BRANCH_TAKEN),
            Some(SideExitReason::BranchTaken)
        );
        assert_eq!(block.classify_exit(42), None);
    }
}
