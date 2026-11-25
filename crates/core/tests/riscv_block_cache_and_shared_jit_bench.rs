// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// Differential verification and throughput benchmark for:
//   1. `Rv32BlockCache` (decoded basic-block cache + self-loop fast path,
//      adapted from MIT `mikroverk/esp32sim` EX265 — runs on every target
//      including browser WASM32 without JIT dependencies).
//   2. Shared-memory persistent-RAM `RiscvJitEngine` (eliminates per-block
//      full-SRAM memcpy across chained compiled blocks on 512 KiB ESP32 SRAM).

use labwired_core::bus::SystemBus;
use labwired_core::cpu::RiscV;
use labwired_core::memory::LinearMemory;
use labwired_core::{DebugControl, Machine, SimulationConfig};

const RAM_BASE: u32 = 0x3FC8_0000; // ESP32-C3 / ESP32-C6 SRAM base
const ESP32_SRAM_LEN: usize = 512 * 1024; // 512 KiB SRAM
const FLASH_BASE: u32 = 0x4200_0000; // ESP32 XIP flash range
const IRAM_BASE: u32 = 0x4038_0000; // ESP32 writable IRAM

fn enc_i(rd: u32, rs1: u32, funct3: u32, imm: i32, opcode: u32) -> u32 {
    ((imm as u32 & 0xFFF) << 20) | (rs1 << 15) | (funct3 << 12) | (rd << 7) | opcode
}
fn enc_s(rs1: u32, rs2: u32, funct3: u32, imm: i32, opcode: u32) -> u32 {
    let u = imm as u32 & 0xFFF;
    ((u >> 5) << 25) | (rs2 << 20) | (rs1 << 15) | (funct3 << 12) | ((u & 0x1F) << 7) | opcode
}
fn lui(rd: u32, imm20: u32) -> u32 {
    (imm20 << 12) | (rd << 7) | 0x37
}
fn addi(rd: u32, rs1: u32, imm: i32) -> u32 {
    enc_i(rd, rs1, 0b000, imm, 0x13)
}
fn xor(rd: u32, rs1: u32, rs2: u32) -> u32 {
    (rs2 << 20) | (rs1 << 15) | (0b100 << 12) | (rd << 7) | 0x33
}
fn add(rd: u32, rs1: u32, rs2: u32) -> u32 {
    (rs2 << 20) | (rs1 << 15) | (rd << 7) | 0x33
}
fn mul(rd: u32, rs1: u32, rs2: u32) -> u32 {
    (0x01 << 25) | (rs2 << 20) | (rs1 << 15) | (rd << 7) | 0x33
}
fn lw(rd: u32, rs1: u32, imm: i32) -> u32 {
    enc_i(rd, rs1, 0b010, imm, 0x03)
}
fn sw(rs1: u32, rs2: u32, imm: i32) -> u32 {
    enc_s(rs1, rs2, 0b010, imm, 0x23)
}
fn bne(rs1: u32, rs2: u32, imm: i32) -> u32 {
    let u = imm as u32;
    ((u >> 12) & 1) << 31
        | ((u >> 5) & 0x3F) << 25
        | (rs2 << 20)
        | (rs1 << 15)
        | (0b001 << 12)
        | ((u >> 1) & 0xF) << 8
        | ((u >> 11) & 1) << 7
        | 0x63
}

/// Realistic ESP32 RV32IMC workload: 4x (load, addi, xor, add, store) over
/// 512 KiB SRAM + counter increment + backward branch.
fn esp32_sram_loop_program() -> Vec<u32> {
    let mut prog = vec![
        lui(1, RAM_BASE >> 12), // x1 = 0x3FC8_0000
        addi(2, 0, 0),          // x2 = counter
        addi(6, 0, 0),          // x6 = 0
    ];
    let head = prog.len();
    for slot in 0..4i32 {
        let off = slot * 4;
        prog.push(lw(3, 1, off));
        prog.push(addi(3, 3, 1));
        prog.push(xor(4, 3, 2));
        prog.push(add(3, 3, 4));
        prog.push(sw(1, 3, off));
    }
    prog.push(addi(2, 2, 1));
    let body_words = (prog.len() - head + 1) as i32;
    prog.push(bne(2, 6, -(body_words - 1) * 4));
    prog
}

/// Pure ALU self-loop (RV32IM: addi + xor + mul + add + bne) with no stores.
fn pure_alu_self_loop_program() -> Vec<u32> {
    let mut prog = vec![
        addi(1, 0, 1), // x1 = 1
        addi(2, 0, 7), // x2 = 7
        addi(3, 0, 3), // x3 = 3
        addi(4, 0, 0), // x4 = counter
        addi(5, 0, 0), // x5 = 0
    ];
    let head = prog.len();
    prog.push(addi(1, 1, 13));
    prog.push(xor(2, 2, 1));
    prog.push(mul(3, 3, 2));
    prog.push(add(1, 1, 3));
    prog.push(addi(4, 4, 1));
    let body_words = (prog.len() - head + 1) as i32;
    prog.push(bne(4, 5, -(body_words - 1) * 4));
    prog
}

fn build_esp32_machine(prog: &[u32], ram_len: usize) -> Machine<RiscV> {
    let mut bus = SystemBus::new();
    let mut flash = Vec::with_capacity(prog.len() * 4);
    for w in prog {
        flash.extend_from_slice(&w.to_le_bytes());
    }
    bus.flash.base_addr = FLASH_BASE as u64;
    bus.flash.data = flash;
    bus.ram.base_addr = RAM_BASE as u64;
    bus.ram.data = vec![0u8; ram_len];
    let mut cpu = RiscV::new();
    cpu.pc = FLASH_BASE;
    cpu.mtimecmp = u64::MAX;
    Machine::new(cpu, bus)
}

#[test]
fn block_cache_matches_interpreter_bit_for_bit_on_alu_and_sram() {
    for prog in [esp32_sram_loop_program(), pure_alu_self_loop_program()] {
        let mut ref_m = build_esp32_machine(&prog, 4096);
        let mut fast_m = build_esp32_machine(&prog, 4096);

        ref_m.config = SimulationConfig {
            decode_cache_enabled: false,
            riscv_jit_enabled: false,
            peripheral_tick_interval: 1,
            ..SimulationConfig::default()
        };
        fast_m.config = SimulationConfig {
            decode_cache_enabled: true,
            riscv_jit_enabled: false,
            peripheral_tick_interval: 1024,
            ..SimulationConfig::default()
        };

        ref_m.run(Some(25_000)).unwrap();
        fast_m.run(Some(25_000)).unwrap();

        assert_eq!(fast_m.cpu.pc, ref_m.cpu.pc);
        assert_eq!(fast_m.cpu.x, ref_m.cpu.x);
        assert_eq!(fast_m.cpu.mtime, ref_m.cpu.mtime);
        assert_eq!(fast_m.bus.ram.data, ref_m.bus.ram.data);
    }
}

#[test]
fn block_cache_invalidates_when_guest_modifies_iram_code() {
    // Program placed in writable IRAM (`extra_mem`):
    //   0: addi x1, x0, 1       (will be overwritten to `addi x1, x0, 99` by store)
    //   4: addi x2, x2, 1
    //   8: lui  x3, IRAM_BASE >> 12
    //  12: sw   x4, 0(x3)       (writes new opcode into IRAM[0])
    //  16: bne  x2, x5, -16     (jumps back to IRAM[0] until x2 == 2)
    let make_machine = || {
        let mut bus = SystemBus::new();
        let prog = [
            addi(1, 0, 1),
            addi(2, 2, 1),
            lui(3, IRAM_BASE >> 12),
            sw(3, 4, 0),
            bne(2, 5, -16),
        ];
        let mut iram = LinearMemory::new(256, IRAM_BASE as u64);
        for (idx, w) in prog.iter().enumerate() {
            iram.data[idx * 4..idx * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        bus.extra_mem.push(iram);

        let mut cpu = RiscV::new();
        cpu.pc = IRAM_BASE;
        cpu.x[4] = addi(1, 0, 99);
        cpu.x[5] = 2;
        cpu.mtimecmp = u64::MAX;
        Machine::new(cpu, bus)
    };

    let mut ref_m = make_machine();
    ref_m.config = SimulationConfig {
        decode_cache_enabled: false,
        riscv_jit_enabled: false,
        peripheral_tick_interval: 1,
        ..SimulationConfig::default()
    };
    let mut fast_m = make_machine();
    fast_m.config = SimulationConfig {
        decode_cache_enabled: true,
        riscv_jit_enabled: false,
        peripheral_tick_interval: 64,
        ..SimulationConfig::default()
    };

    ref_m.run(Some(10)).unwrap();
    fast_m.run(Some(10)).unwrap();

    assert_eq!(ref_m.cpu.x[1], 99);
    assert_eq!(fast_m.cpu.x[1], 99);
    assert_eq!(fast_m.cpu.x, ref_m.cpu.x);
    assert_eq!(fast_m.cpu.pc, ref_m.cpu.pc);
}

#[cfg(all(feature = "jit-framework", feature = "jit"))]
#[test]
fn shared_memory_jit_matches_interpreter_on_512k_esp32_sram() {
    let prog = esp32_sram_loop_program();
    let mut ref_m = build_esp32_machine(&prog, ESP32_SRAM_LEN);
    let mut jit_m = build_esp32_machine(&prog, ESP32_SRAM_LEN);

    ref_m.config = SimulationConfig {
        decode_cache_enabled: false,
        riscv_jit_enabled: false,
        peripheral_tick_interval: 1,
        ..SimulationConfig::default()
    };
    jit_m.config = SimulationConfig {
        decode_cache_enabled: true,
        riscv_jit_enabled: true,
        peripheral_tick_interval: 4096,
        ..SimulationConfig::default()
    };

    ref_m.run(Some(100_000)).unwrap();
    jit_m.run(Some(100_000)).unwrap();

    assert_eq!(jit_m.cpu.pc, ref_m.cpu.pc);
    assert_eq!(jit_m.cpu.x, ref_m.cpu.x);
    assert_eq!(jit_m.cpu.mtime, ref_m.cpu.mtime);
    assert_eq!(jit_m.bus.ram.data[..64], ref_m.bus.ram.data[..64]);
}

#[test]
#[ignore = "throughput benchmark; run with --release --ignored --nocapture"]
fn bench_esp32_512k_sram_interp_vs_block_cache_vs_shared_jit() {
    use std::time::Instant;

    const ITERS: u32 = 2_000_000;

    for (label, prog) in [
        (
            "ESP32 512 KiB SRAM ALU+mem+branch",
            esp32_sram_loop_program(),
        ),
        ("RV32IM pure ALU self-loop", pure_alu_self_loop_program()),
    ] {
        // 1. Baseline uncached interpreter (`decode_cache_enabled = false`)
        let mut base = build_esp32_machine(&prog, ESP32_SRAM_LEN);
        base.config = SimulationConfig {
            decode_cache_enabled: false,
            riscv_jit_enabled: false,
            peripheral_tick_interval: 4096,
            ..SimulationConfig::default()
        };
        let t0 = Instant::now();
        base.run(Some(ITERS)).unwrap();
        let dt_base = t0.elapsed();

        // 2. MIT Rv32BlockCache (all targets incl. browser WASM32)
        let mut bc = build_esp32_machine(&prog, ESP32_SRAM_LEN);
        bc.config = SimulationConfig {
            decode_cache_enabled: true,
            riscv_jit_enabled: false,
            peripheral_tick_interval: 4096,
            ..SimulationConfig::default()
        };
        let t1 = Instant::now();
        bc.run(Some(ITERS)).unwrap();
        let dt_bc = t1.elapsed();

        assert_eq!(bc.cpu.pc, base.cpu.pc);
        assert_eq!(bc.cpu.x, base.cpu.x);
        assert_eq!(bc.cpu.mtime, base.cpu.mtime);

        let base_mips = ITERS as f64 / dt_base.as_secs_f64() / 1e6;
        let bc_mips = ITERS as f64 / dt_bc.as_secs_f64() / 1e6;
        println!("=== {label} ({ITERS} guest instr) ===");
        println!("  interpreter (baseline) : {dt_base:?} ({base_mips:.1} Minsn/s)");
        println!(
            "  Rv32BlockCache (WASM32): {dt_bc:?} ({bc_mips:.1} Minsn/s, {:.2}x)",
            bc_mips / base_mips
        );

        #[cfg(all(feature = "jit-framework", feature = "jit"))]
        {
            let mut jit = build_esp32_machine(&prog, ESP32_SRAM_LEN);
            jit.config = SimulationConfig {
                decode_cache_enabled: true,
                riscv_jit_enabled: true,
                peripheral_tick_interval: 4096,
                ..SimulationConfig::default()
            };
            // Warm up compilation first, then time steady-state execution.
            jit.run(Some(50_000)).unwrap();
            let t2 = Instant::now();
            jit.run(Some(ITERS)).unwrap();
            let dt_jit = t2.elapsed();
            let jit_mips = ITERS as f64 / dt_jit.as_secs_f64() / 1e6;
            println!(
                "  RiscvWasmJit (shared)  : {dt_jit:?} ({jit_mips:.1} Minsn/s, {:.2}x)",
                jit_mips / base_mips
            );
        }
    }
}
