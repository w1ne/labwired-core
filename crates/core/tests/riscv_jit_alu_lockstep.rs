// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Differential equivalence + performance gate for the RV32IMC integer-ALU
//! JIT codegen (framework chunk C).
//!
//! Where the foundation gate (`riscv_jit_lockstep.rs`) proves the *all-bail*
//! frontend is byte-identical on the interpreter runtime, this gate proves
//! the **compiled ALU path actually executes** and is byte-identical to the
//! interpreter:
//!
//!   1. [`alu_hot_loop_is_byte_identical_and_compiles`] — an ALU-heavy hot
//!      loop (arith / logic / shift / mul / div / rem incl. the RISC-V
//!      div-by-zero and `INT_MIN/-1` edge cases) run twice in lockstep;
//!      state is compared byte-for-byte at every block/instruction boundary,
//!      and the run is asserted non-vacuous (real compiled-block executions,
//!      a floor of retired instructions).
//!   2. [`every_alu_op_matches_interpreter`] — a per-op differential fuzz:
//!      each supported op (and the divide edge cases) as a one-instruction
//!      compiled block vs. the interpreter, over random operands.
//!   3. [`bench_alu_loop_beats_interpreter`] (ignored) — a micro-benchmark
//!      showing the compiled ALU loop beats the interpreter, reporting the
//!      measured ratio.
//!
//! Gated behind `jit-framework` (the module) and `jit` (the native wasmtime
//! executor).

#![cfg(all(feature = "jit-framework", feature = "jit"))]

use labwired_core::bus::SystemBus;
use labwired_core::cpu::jit_framework::differential::{compare, DiffPolicy};
use labwired_core::cpu::jit_framework::frontend::IsaFrontend;
use labwired_core::cpu::jit_framework::riscv::{
    differential_cycle_ignore_indices, snapshot_state, RiscVFrontend, RiscvJitEngine, RiscvWasmJit,
};
use labwired_core::cpu::jit_framework::CodeView;
use labwired_core::cpu::RiscV;
use labwired_core::Machine;

// ── RV32I / RV32M encoders (standard field layouts) ───────────────────────

fn enc_i(rd: u32, rs1: u32, funct3: u32, imm: i32, opcode: u32) -> u32 {
    ((imm as u32 & 0xFFF) << 20) | (rs1 << 15) | (funct3 << 12) | (rd << 7) | opcode
}
fn enc_r(rd: u32, rs1: u32, rs2: u32, funct3: u32, funct7: u32) -> u32 {
    (funct7 << 25) | (rs2 << 20) | (rs1 << 15) | (funct3 << 12) | (rd << 7) | 0x33
}
fn addi(rd: u32, rs1: u32, imm: i32) -> u32 {
    enc_i(rd, rs1, 0b000, imm, 0x13)
}
fn ori(rd: u32, rs1: u32, imm: i32) -> u32 {
    enc_i(rd, rs1, 0b110, imm, 0x13)
}
fn andi(rd: u32, rs1: u32, imm: i32) -> u32 {
    enc_i(rd, rs1, 0b111, imm, 0x13)
}
fn slli(rd: u32, rs1: u32, sh: u32) -> u32 {
    enc_i(rd, rs1, 0b001, sh as i32, 0x13)
}
fn add(rd: u32, rs1: u32, rs2: u32) -> u32 {
    enc_r(rd, rs1, rs2, 0b000, 0)
}
fn sub(rd: u32, rs1: u32, rs2: u32) -> u32 {
    enc_r(rd, rs1, rs2, 0b000, 0x20)
}
fn xor(rd: u32, rs1: u32, rs2: u32) -> u32 {
    enc_r(rd, rs1, rs2, 0b100, 0)
}
fn sltu(rd: u32, rs1: u32, rs2: u32) -> u32 {
    enc_r(rd, rs1, rs2, 0b011, 0)
}
fn mul(rd: u32, rs1: u32, rs2: u32) -> u32 {
    enc_r(rd, rs1, rs2, 0b000, 0x01)
}
fn div(rd: u32, rs1: u32, rs2: u32) -> u32 {
    enc_r(rd, rs1, rs2, 0b100, 0x01)
}
fn rem(rd: u32, rs1: u32, rs2: u32) -> u32 {
    enc_r(rd, rs1, rs2, 0b110, 0x01)
}
fn jal(rd: u32, imm: i32) -> u32 {
    let u = imm as u32;
    let imm20 = (u >> 20) & 1;
    let imm10_1 = (u >> 1) & 0x3FF;
    let imm11 = (u >> 11) & 1;
    let imm19_12 = (u >> 12) & 0xFF;
    (imm20 << 31) | (imm10_1 << 21) | (imm11 << 20) | (imm19_12 << 12) | (rd << 7) | 0x6F
}

fn build_machine(prog: &[u32]) -> Machine<RiscV> {
    let mut bus = SystemBus::new();
    let mut bytes = Vec::with_capacity(prog.len() * 4);
    for w in prog {
        bytes.extend_from_slice(&w.to_le_bytes());
    }
    bus.flash.data = bytes;
    bus.flash.base_addr = 0;
    let mut cpu = RiscV::new();
    cpu.pc = 0;
    cpu.mtimecmp = u64::MAX; // keep the CLINT timer from ever firing
    Machine::new(cpu, bus)
}

/// An ALU-heavy hot loop. The straight-line body mixes every op class,
/// **including** the divide/remainder edge cases (`div`/`rem` by the value
/// in `x5` which is driven to 0 and to -1 across iterations, and an
/// `INT_MIN` dividend), then a `jal` closes the loop. The `jal` is chunk-D
/// territory, so each iteration executes the compiled ALU prefix and then
/// interprets the single branch.
fn alu_loop_program() -> Vec<u32> {
    vec![
        // ── loop body (all ALU, one compiled block) ──
        addi(1, 1, 1),    // x1++  (loop counter)
        slli(6, 1, 3),    // x6 = x1 << 3
        add(7, 6, 1),     // x7 = x6 + x1
        sub(8, 7, 6),     // x8 = x7 - x6  (== x1)
        xor(9, 8, 1),     // x9 = x8 ^ x1  (== 0)
        ori(10, 9, 0x2A), // x10 = x9 | 0x2A
        andi(11, 7, 0xF), // x11 = x7 & 0xF
        mul(12, 1, 7),    // x12 = x1 * x7
        // Drive x5 through {0,1,2,...} so div/rem hit divide-by-zero, then
        // normal divisors — exercising the guarded paths every iteration.
        andi(5, 1, 0x3),  // x5 = x1 & 3   (cycles 1,2,3,0,1,2,3,0,...)
        div(13, 12, 5),   // x13 = x12 / x5   (x5==0 → -1)
        rem(14, 12, 5),   // x14 = x12 % x5   (x5==0 → x12)
        addi(15, 0, -1),  // x15 = -1
        slli(16, 15, 31), // x16 = INT_MIN (0x8000_0000)
        div(17, 16, 15),  // x17 = INT_MIN / -1 → INT_MIN (overflow)
        rem(18, 16, 15),  // x18 = INT_MIN % -1 → 0 (overflow)
        sltu(19, 8, 7),   // x19 = (x8 <u x7)
        // ── loop back ──
        jal(0, -(16 * 4)), // jump to the top (16 instrs back)
    ]
}

/// (1) Differential byte-identity + non-vacuity for the ALU hot loop.
#[test]
fn alu_hot_loop_is_byte_identical_and_compiles() {
    const MAX_UNITS: u64 = 4_000;
    const INSTR_FLOOR: u64 = 3_000;

    let prog = alu_loop_program();
    let mut interp = build_machine(&prog);
    let mut jit = build_machine(&prog);

    // Low threshold so the loop head promotes + compiles quickly and the
    // compiled block is genuinely exercised.
    let mut engine = RiscvJitEngine::new(4);
    let policy = DiffPolicy {
        ignore_indices: differential_cycle_ignore_indices(),
        block_boundary_only: false,
    };

    let mut retired: u64 = 0;
    let mut units: u64 = 0;
    while retired < INSTR_FLOOR * 2 && units < MAX_UNITS {
        units += 1;
        // Advance the JIT side by one dispatch unit; it retires `n` guest
        // instructions (a whole compiled block, or one interpreted branch).
        let n = engine.step_unit(&mut jit);
        assert!(n > 0, "jit machine halted unexpectedly at unit {units}");
        // Advance the interpreter reference by the SAME instruction count to
        // stay PC-aligned across the batched block.
        for _ in 0..n {
            interp
                .step()
                .expect("interpreter must not fault on pure ALU");
        }
        retired += n as u64;

        // Byte-for-byte architectural equality at this aligned boundary.
        let si = snapshot_state(&interp.cpu);
        let sj = snapshot_state(&jit.cpu);
        if let Some(d) = compare(units, &si, &sj, &policy) {
            panic!(
                "JIT diverged from interpreter at unit {units} (retired {retired}): {d:?}\n\
                 interp x={:?}\n jit    x={:?}",
                &interp.cpu.x, &jit.cpu.x
            );
        }
    }

    // Non-vacuous: a real hot loop ran through compiled blocks, not refusal.
    let stats = engine.stats();
    assert!(
        stats.compiled > 0,
        "no ALU block ever crossed the hot threshold / compiled"
    );
    assert!(
        stats.block_runs > 0,
        "no compiled block was ever executed (JIT path not engaged)"
    );
    assert!(
        stats.block_instrs >= INSTR_FLOOR,
        "compiled blocks retired only {} instructions (floor {INSTR_FLOOR})",
        stats.block_instrs
    );
    assert!(
        interp.cpu.x[1] as u64 >= 100,
        "loop counter x1={} too low — the loop body did not run enough",
        interp.cpu.x[1]
    );
    println!(
        "alu_hot_loop: retired={retired} compiled_blocks={} block_runs={} block_instrs={} interpreted={}",
        stats.compiled, stats.block_runs, stats.block_instrs, stats.interpreted
    );
}

/// (2) Per-op differential fuzz: every supported op (and the divide edge
/// cases) as a one-instruction compiled block vs. the interpreter.
#[test]
fn every_alu_op_matches_interpreter() {
    // Small deterministic PRNG (xorshift) so the test is hermetic.
    let mut seed: u64 = 0x1234_5678_9abc_def0;
    let mut rng = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed as u32
    };

    // (encoder, name). rd=1, rs1=2, rs2=3 throughout.
    type OpEnc = fn(u32, u32, u32) -> u32;
    let ops: &[(OpEnc, &str)] = &[
        (|d, a, b| enc_r(d, a, b, 0b000, 0), "add"),
        (|d, a, b| enc_r(d, a, b, 0b000, 0x20), "sub"),
        (|d, a, b| enc_r(d, a, b, 0b001, 0), "sll"),
        (|d, a, b| enc_r(d, a, b, 0b010, 0), "slt"),
        (|d, a, b| enc_r(d, a, b, 0b011, 0), "sltu"),
        (|d, a, b| enc_r(d, a, b, 0b100, 0), "xor"),
        (|d, a, b| enc_r(d, a, b, 0b101, 0), "srl"),
        (|d, a, b| enc_r(d, a, b, 0b101, 0x20), "sra"),
        (|d, a, b| enc_r(d, a, b, 0b110, 0), "or"),
        (|d, a, b| enc_r(d, a, b, 0b111, 0), "and"),
        (|d, a, b| enc_r(d, a, b, 0b000, 0x01), "mul"),
        (|d, a, b| enc_r(d, a, b, 0b001, 0x01), "mulh"),
        (|d, a, b| enc_r(d, a, b, 0b010, 0x01), "mulhsu"),
        (|d, a, b| enc_r(d, a, b, 0b011, 0x01), "mulhu"),
        (|d, a, b| enc_r(d, a, b, 0b100, 0x01), "div"),
        (|d, a, b| enc_r(d, a, b, 0b101, 0x01), "divu"),
        (|d, a, b| enc_r(d, a, b, 0b110, 0x01), "rem"),
        (|d, a, b| enc_r(d, a, b, 0b111, 0x01), "remu"),
    ];

    // Operand pairs: random, plus the arithmetic corner cases.
    let mut pairs: Vec<(u32, u32)> = vec![
        (0, 0),
        (1, 0),                     // divide by zero
        (0x8000_0000, 0xFFFF_FFFF), // INT_MIN / -1 overflow
        (0xFFFF_FFFF, 0xFFFF_FFFF),
        (0x7FFF_FFFF, 2),
        (100, 7),
        (0x8000_0000, 2),
    ];
    for _ in 0..200 {
        pairs.push((rng(), rng()));
    }

    let frontend = RiscVFrontend::new();
    let jit = RiscvWasmJit::new();

    for &(enc, name) in ops {
        // rd=1, rs1=2, rs2=3 ; terminator so the walk ends after the op.
        let prog_words = [enc(1, 2, 3), 0x0000_0073 /* ecall */];
        let flash = {
            let mut b = Vec::new();
            for w in prog_words {
                b.extend_from_slice(&w.to_le_bytes());
            }
            b
        };
        let plan = {
            let view = CodeView::new(0, &flash);
            frontend.translate_block(0, &view).expect("translate")
        };
        assert!(!plan.is_stub(), "{name}: expected a compiled ALU block");
        assert_eq!(plan.instr_count, 1, "{name}: one-instruction block");
        let mut block = jit.compile(&plan, None).expect("compile");

        for &(a, b) in &pairs {
            // Interpreter reference.
            let mut im = build_machine(&prog_words);
            im.cpu.x[2] = a;
            im.cpu.x[3] = b;
            im.step().expect("op step");
            let want = im.cpu.x[1];

            // Compiled block run directly against a register file.
            let mut x = [0u32; 32];
            x[2] = a;
            x[3] = b;
            let (_exit, n, _clear) = block.run(&mut x, &mut []);
            assert_eq!(n, 1, "{name}: retires one instruction");
            assert_eq!(
                x[1], want,
                "{name}(a=0x{a:08x}, b=0x{b:08x}) mismatch: jit=0x{:08x} interp=0x{want:08x}",
                x[1]
            );
        }
    }
}

/// (3) Micro-benchmark: the compiled ALU loop vs. the interpreter on the
/// same firmware. Ignored by default (timing-sensitive); run with
/// `--ignored --nocapture` to see the ratio.
#[test]
#[ignore = "micro-benchmark; run with --ignored --nocapture"]
fn bench_alu_loop_beats_interpreter() {
    use std::time::Instant;

    // A long straight-line ALU body so the compiled block amortises the
    // wasm-call + register-marshalling overhead, then a jal loop-back.
    let mut prog: Vec<u32> = Vec::new();
    for _ in 0..8 {
        prog.push(addi(1, 1, 1));
        prog.push(slli(2, 1, 1));
        prog.push(add(3, 2, 1));
        prog.push(xor(4, 3, 2));
        prog.push(mul(5, 4, 3));
        prog.push(sub(6, 5, 4));
        prog.push(ori(7, 6, 0x5));
        prog.push(andi(8, 7, 0x7F));
    }
    let body = prog.len() as i32;
    prog.push(jal(0, -(body * 4)));

    const ITERS: u64 = 400_000;

    // Interpreter-only timing.
    let mut im = build_machine(&prog);
    let t0 = Instant::now();
    let mut ic = 0u64;
    while ic < ITERS {
        im.step().unwrap();
        ic += 1;
    }
    let interp_dt = t0.elapsed();

    // JIT timing (warm up so the loop block is compiled first).
    let mut jm = build_machine(&prog);
    let mut engine = RiscvJitEngine::new(4);
    for _ in 0..2_000 {
        engine.step_unit(&mut jm);
    }
    let warm = engine.stats();
    let t1 = Instant::now();
    let mut jc = 0u64;
    while jc < ITERS {
        jc += engine.step_unit(&mut jm) as u64;
    }
    let jit_dt = t1.elapsed();
    let stats = engine.stats();

    let interp_mips = ITERS as f64 / interp_dt.as_secs_f64() / 1e6;
    let jit_mips = ITERS as f64 / jit_dt.as_secs_f64() / 1e6;
    let ratio = interp_dt.as_secs_f64() / jit_dt.as_secs_f64();

    println!("--- RV32IMC ALU micro-bench ({ITERS} guest instr) ---");
    println!("interp: {interp_dt:?}  ({interp_mips:.1} MIPS)");
    println!("jit   : {jit_dt:?}  ({jit_mips:.1} MIPS)");
    println!("speedup: {ratio:.2}x");
    println!(
        "jit path: compiled_blocks={} block_runs={} block_instrs={} interpreted={} (warmup block_runs={})",
        stats.compiled, stats.block_runs, stats.block_instrs, stats.interpreted, warm.block_runs
    );

    assert!(
        stats.block_runs > warm.block_runs,
        "compiled blocks must run during the timed phase"
    );
    assert!(
        ratio > 1.0,
        "compiled ALU loop ({jit_mips:.1} MIPS) did not beat the interpreter \
         ({interp_mips:.1} MIPS): {ratio:.2}x"
    );
}
