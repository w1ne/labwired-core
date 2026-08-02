#![allow(clippy::identity_op)] // Thumb encoder keeps `0x0000 |` to make opcode prefix bits explicit

//! ARM Thumb / Cortex-M oracle harness (STM32 family).
//!
//! Parallel to `riscv` (ESP32-C3) and the Xtensa harness in `lib.rs`, but
//! for ARM Cortex-M Thumb / Thumb-2 firmware.  Initial target: STM32F4
//! (Cortex-M4F).  Same model: a `ThumbOracleCase` bundles a program +
//! `setup` + `expect`, three runners are provided (`run_sim` always
//! available, `_hw` / `_diff` gated on `hw-oracle-stm32`).
//!
//! # Memory map
//!
//! Programs load at [`PROG_BASE`] (0x0800_0000, STM32 flash window).  Data
//! uses [`DATA_BASE`] (0x2000_0000, SRAM).  Matches every STM32 chip yaml
//! in `configs/chips/stm32f*.yaml`.
//!
//! # Reset bypass
//!
//! `CortexM::reset()` reads the vector table at VTOR (defaults to 0) for
//! initial SP and PC.  Rather than synthesise a vector table for every
//! oracle program, the runner calls `reset()` (which finds no vector
//! table on the empty bus and silently falls through), then explicitly
//! writes a sensible SP and `set_pc(entry_pc)`.  The Thumb mode bit is
//! handled internally by the CPU — entry PCs are passed even (low bit
//! 0) and the executor decodes Thumb naturally.
//!
//! # Program terminator
//!
//! Cortex-M has BKPT (which raises a fault and complicates assertion).
//! Oracle programs instead end with `B .` (branch-to-self, 0xE7FE) and
//! the runner detects PC stabilisation, mirroring the RISC-V harness.

use std::collections::HashMap;
use std::path::PathBuf;

/// Start of the STM32 flash window — where firmware is loaded.
pub const PROG_BASE: u32 = 0x0800_0000;

/// SRAM address where oracle programs are loaded on real STM32 silicon.
/// 8 KiB into SRAM — past the data window at [`DATA_BASE`], with room
/// for the stack to grow down from [`INIT_SP`] without colliding.
///
/// Chosen to fit on the smallest target in the bank: STM32F103RB has
/// only 20 KiB SRAM (0x2000_0000-0x2000_5000).  F4 has 96 KiB so the
/// program fits comfortably; this address works for both families.
pub const PROG_BASE_HW: u32 = 0x2000_2000;

/// Start of the STM32 SRAM window — used for both sim and HW data
/// (STR/LDR target).  The 8 KiB from `DATA_BASE..DATA_BASE+0x2000`
/// is reserved as the data window in both runners.
pub const DATA_BASE: u32 = 0x2000_0000;

/// Scratch window size.  64 KiB is comfortably larger than any oracle
/// program in the sim.  On HW we only write the actual program byte
/// count, so an over-large sim allocation doesn't hurt.
pub const ORACLE_MEM_SIZE: usize = 64 * 1024;

/// Initial stack pointer — top of the F103RB SRAM window (20 KiB at
/// 0x2000_0000-0x2000_5000), 8-byte aligned at 0x2000_4FF8.
///
/// Smaller than the F4's 96 KiB top of SRAM, but the oracle bank
/// doesn't push anything (no function calls).  Using the smaller
/// value means the same SP works for every STM32 family the harness
/// targets.
pub const INIT_SP: u32 = 0x2000_4FF8;

/// `B .` — 16-bit Thumb branch-to-self.  Used as the program terminator.
const B_SELF: u16 = 0xE7FE;

/// APSR condition-flag bits (N, Z, C, V) within `xpsr`.
pub const NZCV_MASK: u32 = 0xF000_0000;

/// Maximum simulator steps before declaring runaway.
const MAX_STEPS: usize = 10_000;

// ── State ──────────────────────────────────────────────────────────────────────

/// Snapshot of CPU + memory state used by ARM Thumb oracle tests.
#[derive(Default, Debug, Clone)]
pub struct ThumbOracleState {
    /// Register values keyed by ARM name: `"r0"`..`"r12"`, `"sp"`, `"lr"`,
    /// `"pc"`.  ABI aliases (`"a1"`, `"a2"`, …) are not recognised — the
    /// architectural names keep tests unambiguous across calling
    /// conventions.
    pub regs: HashMap<String, u32>,
    /// Memory snapshot (word-aligned address → 32-bit word, LE).
    pub mem: HashMap<u32, u32>,
    /// PC after execution halts (the address of the B-self terminator on
    /// successful completion).
    pub pc: u32,
}

impl ThumbOracleState {
    pub fn write_reg(&mut self, name: &str, v: u32) {
        self.regs.insert(name.to_string(), v);
    }

    pub fn read_reg(&self, name: &str) -> u32 {
        self.regs.get(name).copied().unwrap_or(0)
    }

    pub fn assert_reg(&self, name: &str, expected: u32) {
        let actual = self.read_reg(name);
        assert_eq!(
            actual, expected,
            "thumb oracle: register {name}: expected 0x{expected:08X}, got 0x{actual:08X}"
        );
    }

    pub fn assert_reg_masked(&self, name: &str, mask: u32, expected: u32) {
        let actual = self.read_reg(name) & mask;
        assert_eq!(
            actual, expected,
            "thumb oracle: register {name} masked by 0x{mask:08X}: \
             expected 0x{expected:08X}, got 0x{actual:08X}"
        );
    }

    /// Assert the APSR condition flags (N, Z, C, V), read from the captured
    /// `xpsr`.  Used to pin flag-setting behaviour — and, for IT-block cases,
    /// flag *preservation* — against silicon.
    pub fn assert_nzcv(&self, n: bool, z: bool, c: bool, v: bool) {
        let x = self.read_reg("xpsr");
        let got = (
            (x >> 31) & 1 == 1,
            (x >> 30) & 1 == 1,
            (x >> 29) & 1 == 1,
            (x >> 28) & 1 == 1,
        );
        assert_eq!(
            got,
            (n, z, c, v),
            "thumb oracle: APSR NZCV: expected {:?}, got {:?} (xpsr=0x{x:08X})",
            (n, z, c, v),
            got
        );
    }

    pub fn write_mem(&mut self, addr: u32, v: u32) {
        self.mem.insert(addr, v);
    }

    pub fn read_mem(&self, addr: u32) -> u32 {
        self.mem.get(&addr).copied().unwrap_or(0)
    }

    pub fn assert_mem(&self, addr: u32, expected: u32) {
        let actual = self.read_mem(addr);
        assert_eq!(
            actual, expected,
            "thumb oracle: mem[0x{addr:08X}]: expected 0x{expected:08X}, got 0x{actual:08X}"
        );
    }

    pub fn assert_pc(&self, expected: u32) {
        assert_eq!(
            self.pc, expected,
            "thumb oracle: pc: expected 0x{expected:08X}, got 0x{:08X}",
            self.pc
        );
    }
}

fn parse_r_name(name: &str) -> Option<u8> {
    if let Some(n) = name.strip_prefix('r') {
        let idx: u8 = n.parse().ok()?;
        // r0..r15 are valid; r13=sp, r14=lr, r15=pc.
        if idx < 16 {
            return Some(idx);
        }
    }
    match name {
        "sp" => Some(13),
        "lr" => Some(14),
        "pc" => Some(15),
        _ => None,
    }
}

// ── Program + case ─────────────────────────────────────────────────────────────

pub enum ThumbProgram {
    Asm(Vec<u8>),
    Elf(PathBuf),
}

/// Factory that builds the simulator bus for a case.  When `None`, the
/// sim runner uses a bare RAM-only [`SystemBus::empty`] and loads the
/// program into flash at [`PROG_BASE`] — right for pure-CPU instruction
/// oracles.  When `Some`, the runner builds a **full chip bus** from the
/// factory (peripherals mapped) and loads/runs the program from SRAM at
/// [`PROG_BASE_HW`] — the same address silicon uses — so MMIO accesses
/// in the program hit real peripheral models.  This is the peripheral-
/// execution oracle path.
type SimBusFactory = Box<dyn Fn() -> labwired_core::bus::SystemBus + Send + Sync>;

pub struct ThumbOracleCase {
    pub program: ThumbProgram,
    pub setup: Box<dyn Fn(&mut ThumbOracleState) + Send + Sync>,
    pub expect: Box<dyn Fn(&ThumbOracleState) + Send + Sync>,
    pub mem_capture_addrs: Vec<u32>,
    /// Optional full-chip sim bus factory (see [`SimBusFactory`]).  Set via
    /// [`ThumbOracleCase::sim_bus`]; the HW runner is unaffected (silicon
    /// always has its peripherals).
    pub sim_bus: Option<SimBusFactory>,
    /// After the CPU program settles on `B .`, tick the sim's peripherals this
    /// many times so an **autonomous** peripheral the program armed (e.g. a DMA
    /// mem-to-mem transfer) runs to completion.  Sim-only: on silicon such
    /// engines run concurrently and have long finished by the time the core
    /// halts at the breakpoint, so the HW runner ignores this.  Set via
    /// [`ThumbOracleCase::settle_ticks`].
    pub settle_ticks: usize,
    /// Byte offset from the load base at which execution starts. 0 means the
    /// first instruction (the default flat-program case). Interrupt oracles set
    /// this so a vector table can sit at the load base while `main` runs from
    /// after it. Set via [`ThumbOracleCase::entry_offset`].
    pub entry_offset: u32,
    /// Drive the sim's peripherals **live** during execution: after each CPU
    /// step, tick all peripherals and pend any IRQs they raise into the CPU,
    /// and build the CPU with the Cortex-M system block (shared NVIC/VTOR) so
    /// it can actually take them. Required for interrupt-delivery oracles;
    /// default off, so static oracles are unaffected. On silicon this is
    /// automatic. Set via
    /// [`ThumbOracleCase::live_peripherals`].
    pub live_peripherals: bool,
}

impl ThumbOracleCase {
    /// Build from raw little-endian bytes.  A `B .` terminator is
    /// appended automatically.
    pub fn from_bytes(mut bytes: Vec<u8>) -> Self {
        bytes.extend_from_slice(&B_SELF.to_le_bytes());
        Self {
            program: ThumbProgram::Asm(bytes),
            setup: Box::new(|_| {}),
            expect: Box::new(|_| {}),
            mem_capture_addrs: Vec::new(),
            sim_bus: None,
            settle_ticks: 0,
            entry_offset: 0,
            live_peripherals: false,
        }
    }

    /// Build from a mixed sequence of 16-bit and 32-bit Thumb instructions
    /// (see [`Thumb`]).  The `B .` terminator is appended automatically.
    pub fn mixed(insns: &[Thumb]) -> Self {
        Self::from_bytes(assemble(insns))
    }

    /// Build from a sequence of 16-bit Thumb halfwords.  Each is emitted
    /// as 2 LE bytes; the `B .` terminator is appended.
    pub fn halfwords(words: &[u16]) -> Self {
        let mut bytes = Vec::with_capacity(words.len() * 2);
        for w in words {
            bytes.extend_from_slice(&w.to_le_bytes());
        }
        Self::from_bytes(bytes)
    }

    /// Build from a mixed sequence of 16-bit and 32-bit Thumb-2
    /// instructions.  Each `u32` is split into two halfwords in the
    /// Thumb-2 order: the high halfword (containing the encoding's
    /// `op1`) is emitted first, the low halfword second.
    pub fn t2_words(insns: &[u32]) -> Self {
        let mut bytes = Vec::with_capacity(insns.len() * 4);
        for w in insns {
            let hi = ((*w >> 16) & 0xFFFF) as u16;
            let lo = (*w & 0xFFFF) as u16;
            bytes.extend_from_slice(&hi.to_le_bytes());
            bytes.extend_from_slice(&lo.to_le_bytes());
        }
        Self::from_bytes(bytes)
    }

    pub fn elf(path: &str) -> Self {
        Self {
            program: ThumbProgram::Elf(PathBuf::from(path)),
            setup: Box::new(|_| {}),
            expect: Box::new(|_| {}),
            mem_capture_addrs: Vec::new(),
            sim_bus: None,
            settle_ticks: 0,
            entry_offset: 0,
            live_peripherals: false,
        }
    }

    /// Run the sim side on a **full chip bus** built by `f` (peripherals
    /// mapped), executing the program from SRAM at [`PROG_BASE_HW`] — the
    /// same address the HW runner uses.  Turns a case into a peripheral-
    /// execution oracle: MMIO writes/reads in the program exercise real
    /// peripheral models on the sim side and real silicon on the HW side,
    /// and `_diff` cross-validates the two.
    pub fn sim_bus<F>(mut self, f: F) -> Self
    where
        F: Fn() -> labwired_core::bus::SystemBus + Send + Sync + 'static,
    {
        self.sim_bus = Some(Box::new(f));
        self
    }

    /// Tick the sim's peripherals `n` times after the program settles, so an
    /// armed autonomous engine (e.g. DMA mem-to-mem) completes.  Sim-only — see
    /// [`ThumbOracleCase::settle_ticks`].  `n` should comfortably exceed the
    /// number of elements the engine must move (extra ticks are no-ops once it
    /// is idle).
    pub fn settle_ticks(mut self, n: usize) -> Self {
        self.settle_ticks = n;
        self
    }

    /// Start execution `bytes` into the program rather than at the first byte.
    /// Lets a vector table sit at the load base while `main` runs from after it
    /// (see [`ThumbOracleCase::entry_offset`]).
    pub fn entry_offset(mut self, bytes: u32) -> Self {
        self.entry_offset = bytes;
        self
    }

    /// Drive peripherals live and let the CPU take their interrupts — required
    /// for interrupt-delivery oracles (see [`ThumbOracleCase::live_peripherals`]).
    pub fn live_peripherals(mut self, on: bool) -> Self {
        self.live_peripherals = on;
        self
    }

    pub fn setup<F>(mut self, f: F) -> Self
    where
        F: Fn(&mut ThumbOracleState) + Send + Sync + 'static,
    {
        self.setup = Box::new(f);
        self
    }

    pub fn expect<F>(mut self, f: F) -> Self
    where
        F: Fn(&ThumbOracleState) + Send + Sync + 'static,
    {
        self.expect = Box::new(f);
        self
    }

    pub fn capture_mem(mut self, addrs: &[u32]) -> Self {
        self.mem_capture_addrs.extend_from_slice(addrs);
        self
    }
}

// ── Thumb-1 (16-bit) encoders ──────────────────────────────────────────────────
//
// Encodings cross-checked against the ARMv7-M Architecture Reference Manual
// (DDI 0403E.e), Chapter A6 "Thumb instruction set encoding".  Each
// encoder asserts on out-of-range operands so test authors notice
// quickly when they overflow a 3/5/8-bit field.

/// `MOVS Rd, #imm8` — T1 encoding.  Rd is 0..7, imm is 0..255.
pub fn movs_imm8(rd: u8, imm: u8) -> u16 {
    assert!(rd < 8, "MOVS T1 requires Rd < 8 (got r{rd})");
    0x2000 | ((rd as u16) << 8) | (imm as u16)
}

/// `ADDS Rd, Rn, Rm` — T1 encoding (register form).  All low regs.
pub fn adds_reg(rd: u8, rn: u8, rm: u8) -> u16 {
    assert!(rd < 8 && rn < 8 && rm < 8, "ADDS T1 needs low registers");
    0x1800 | ((rm as u16) << 6) | ((rn as u16) << 3) | (rd as u16)
}

/// `SUBS Rd, Rn, Rm` — T1 encoding.
pub fn subs_reg(rd: u8, rn: u8, rm: u8) -> u16 {
    assert!(rd < 8 && rn < 8 && rm < 8, "SUBS T1 needs low registers");
    0x1A00 | ((rm as u16) << 6) | ((rn as u16) << 3) | (rd as u16)
}

/// `ADDS Rd, Rn, #imm3` — T1 encoding.  imm3 is 0..7.
pub fn adds_imm3(rd: u8, rn: u8, imm: u8) -> u16 {
    assert!(rd < 8 && rn < 8 && imm < 8, "ADDS imm3 fields out of range");
    0x1C00 | ((imm as u16) << 6) | ((rn as u16) << 3) | (rd as u16)
}

/// `ADDS Rd, Rd, #imm8` — T2 encoding (8-bit immediate, two-arg form).
pub fn adds_imm8(rd: u8, imm: u8) -> u16 {
    assert!(rd < 8, "ADDS imm8 needs low register");
    0x3000 | ((rd as u16) << 8) | (imm as u16)
}

/// `ANDS Rd, Rm` — T1 (two-arg, Rd = Rd & Rm).
pub fn ands(rd: u8, rm: u8) -> u16 {
    assert!(rd < 8 && rm < 8, "ANDS needs low registers");
    0x4000 | ((rm as u16) << 3) | (rd as u16)
}

/// `ORRS Rd, Rm` — T1.
pub fn orrs(rd: u8, rm: u8) -> u16 {
    assert!(rd < 8 && rm < 8, "ORRS needs low registers");
    0x4300 | ((rm as u16) << 3) | (rd as u16)
}

/// `EORS Rd, Rm` — T1.
pub fn eors(rd: u8, rm: u8) -> u16 {
    assert!(rd < 8 && rm < 8, "EORS needs low registers");
    0x4040 | ((rm as u16) << 3) | (rd as u16)
}

/// `MULS Rd, Rm, Rd` — T1.  Note the two-arg form: Rd = Rm * Rd.
pub fn muls(rd: u8, rm: u8) -> u16 {
    assert!(rd < 8 && rm < 8, "MULS needs low registers");
    0x4340 | ((rm as u16) << 3) | (rd as u16)
}

/// `LSLS Rd, Rm, #imm5` — T1.  imm5 is 0..31.
pub fn lsls_imm(rd: u8, rm: u8, imm5: u8) -> u16 {
    assert!(
        rd < 8 && rm < 8 && imm5 < 32,
        "LSLS imm5 fields out of range"
    );
    0x0000 | ((imm5 as u16) << 6) | ((rm as u16) << 3) | (rd as u16)
}

/// `LSRS Rd, Rm, #imm5` — T1.  Encoding uses imm5=0 to mean shift-32.
pub fn lsrs_imm(rd: u8, rm: u8, imm5: u8) -> u16 {
    assert!(
        rd < 8 && rm < 8 && imm5 < 32,
        "LSRS imm5 fields out of range"
    );
    0x0800 | ((imm5 as u16) << 6) | ((rm as u16) << 3) | (rd as u16)
}

/// `ASRS Rd, Rm, #imm5` — T1.
pub fn asrs_imm(rd: u8, rm: u8, imm5: u8) -> u16 {
    assert!(
        rd < 8 && rm < 8 && imm5 < 32,
        "ASRS imm5 fields out of range"
    );
    0x1000 | ((imm5 as u16) << 6) | ((rm as u16) << 3) | (rd as u16)
}

// ── Thumb-1 register-controlled shifts (`Rdn` shifted by `Rm[7:0]`) ───────────
// Two-arg data-processing form (`0b0100_00` group); pin the carry-out the
// register-shift model fix corrected.

/// `LSLS Rdn, Rm` — T1 (shift amount from Rm).
pub fn lsl_reg(rdn: u8, rm: u8) -> u16 {
    assert!(rdn < 8 && rm < 8, "LSLS reg needs low registers");
    0x4080 | ((rm as u16) << 3) | (rdn as u16)
}
/// `LSRS Rdn, Rm` — T1.
pub fn lsr_reg(rdn: u8, rm: u8) -> u16 {
    assert!(rdn < 8 && rm < 8, "LSRS reg needs low registers");
    0x40C0 | ((rm as u16) << 3) | (rdn as u16)
}
/// `ASRS Rdn, Rm` — T1.
pub fn asr_reg(rdn: u8, rm: u8) -> u16 {
    assert!(rdn < 8 && rm < 8, "ASRS reg needs low registers");
    0x4100 | ((rm as u16) << 3) | (rdn as u16)
}
/// `RORS Rdn, Rm` — T1 (rotate right by Rm[7:0]).
pub fn ror_reg(rdn: u8, rm: u8) -> u16 {
    assert!(rdn < 8 && rm < 8, "RORS reg needs low registers");
    0x41C0 | ((rm as u16) << 3) | (rdn as u16)
}

/// `CMP Rn, Rm` — T1.
pub fn cmp_reg(rn: u8, rm: u8) -> u16 {
    assert!(rn < 8 && rm < 8, "CMP T1 needs low registers");
    0x4280 | ((rm as u16) << 3) | (rn as u16)
}

/// `STR Rt, [Rn, #imm5*4]` — T1.  imm5 is 0..31; offset is imm5*4 bytes.
pub fn str_imm5(rt: u8, rn: u8, imm5: u8) -> u16 {
    assert!(
        rt < 8 && rn < 8 && imm5 < 32,
        "STR imm5 fields out of range"
    );
    0x6000 | ((imm5 as u16) << 6) | ((rn as u16) << 3) | (rt as u16)
}

/// `LDR Rt, [Rn, #imm5*4]` — T1.
pub fn ldr_imm5(rt: u8, rn: u8, imm5: u8) -> u16 {
    assert!(
        rt < 8 && rn < 8 && imm5 < 32,
        "LDR imm5 fields out of range"
    );
    0x6800 | ((imm5 as u16) << 6) | ((rn as u16) << 3) | (rt as u16)
}

// ── Thumb-1 register-offset load/store family (`[Rn, Rm]`) ────────────────────
// All share the layout `base | (rm << 6) | (rn << 3) | rt`, low registers only.
// These are the encodings whose decode the #3 model fix corrected
// (STRH/LDRSB/LDRH/LDRSH were mis-masked); the oracle cases below pin the
// sign-extension + halfword semantics to silicon.

/// Encode one register-offset load/store given its 7-bit opcode `base`.
fn reg_offset(base: u16, rt: u8, rn: u8, rm: u8) -> u16 {
    assert!(rt < 8 && rn < 8 && rm < 8, "reg-offset needs low registers");
    base | ((rm as u16) << 6) | ((rn as u16) << 3) | (rt as u16)
}

/// `STR Rt, [Rn, Rm]` — T1.
pub fn str_reg(rt: u8, rn: u8, rm: u8) -> u16 {
    reg_offset(0x5000, rt, rn, rm)
}
/// `STRH Rt, [Rn, Rm]` — T1 (stores the low halfword).
pub fn strh_reg(rt: u8, rn: u8, rm: u8) -> u16 {
    reg_offset(0x5200, rt, rn, rm)
}
/// `STRB Rt, [Rn, Rm]` — T1 (stores the low byte).
pub fn strb_reg(rt: u8, rn: u8, rm: u8) -> u16 {
    reg_offset(0x5400, rt, rn, rm)
}
/// `LDRSB Rt, [Rn, Rm]` — T1 (sign-extends the loaded byte to 32 bits).
pub fn ldrsb_reg(rt: u8, rn: u8, rm: u8) -> u16 {
    reg_offset(0x5600, rt, rn, rm)
}
/// `LDR Rt, [Rn, Rm]` — T1.
pub fn ldr_reg(rt: u8, rn: u8, rm: u8) -> u16 {
    reg_offset(0x5800, rt, rn, rm)
}
/// `LDRH Rt, [Rn, Rm]` — T1 (zero-extends the loaded halfword).
pub fn ldrh_reg(rt: u8, rn: u8, rm: u8) -> u16 {
    reg_offset(0x5A00, rt, rn, rm)
}
/// `LDRB Rt, [Rn, Rm]` — T1 (zero-extends the loaded byte).
pub fn ldrb_reg(rt: u8, rn: u8, rm: u8) -> u16 {
    reg_offset(0x5C00, rt, rn, rm)
}
/// `LDRSH Rt, [Rn, Rm]` — T1 (sign-extends the loaded halfword to 32 bits).
pub fn ldrsh_reg(rt: u8, rn: u8, rm: u8) -> u16 {
    reg_offset(0x5E00, rt, rn, rm)
}

/// `IT{x{y{z}}} <firstcond>` — T1 (`0b1011_1111_<cond>_<mask>`).  `mask`
/// selects the then/else pattern; `0b1000` (`0x8`) is a one-instruction
/// `IT <cond>` block.  Condition codes are the standard ARM encodings
/// (`EQ=0`, `NE=1`, …).  Used to pin the "16-bit data-processing inside an
/// IT block does not update APSR" rule (#2) against silicon.
pub fn it(firstcond: u8, mask: u8) -> u16 {
    assert!(
        firstcond < 16 && mask != 0 && mask < 16,
        "IT fields out of range"
    );
    0xBF00 | ((firstcond as u16) << 4) | (mask as u16)
}

/// ARM condition code `EQ` (Z == 1) — the only one the oracle cases need.
pub const COND_EQ: u8 = 0b0000;

/// `B label` (T2, unconditional) — `0b11100_iiiiiiiiiii`.  `offset` is the
/// signed byte offset from the *address of this instruction* (the
/// instruction-relative form most programmers reason about, *not* the
/// ARM "PC+4" form — the encoder subtracts 4 internally).  Must be even
/// and fit in ±2048 bytes.
pub fn b_uncond(offset_from_self: i32) -> u16 {
    let pc_relative = offset_from_self - 4; // ARM PC is +4 ahead
    assert!(
        pc_relative & 1 == 0 && (-2048..=2046).contains(&pc_relative),
        "B offset {offset_from_self} out of range or unaligned"
    );
    let imm11 = ((pc_relative >> 1) as u32) & 0x7FF;
    0xE000 | (imm11 as u16)
}

/// `BEQ label` (T1) — `0b1101_0000_iiiiiiii`.  `offset_from_self` is the
/// signed byte offset; the encoder subtracts 4 (ARM "PC+4") internally.
/// Must be even and fit in ±256 bytes (T1 8-bit offset).
pub fn beq(offset_from_self: i32) -> u16 {
    b_cond(0b0000, offset_from_self)
}

/// `BNE label` (T1).
pub fn bne(offset_from_self: i32) -> u16 {
    b_cond(0b0001, offset_from_self)
}

fn b_cond(cond: u8, offset_from_self: i32) -> u16 {
    let pc_relative = offset_from_self - 4;
    assert!(
        pc_relative & 1 == 0 && (-256..=254).contains(&pc_relative),
        "B<cond> offset {offset_from_self} out of range"
    );
    let imm8 = ((pc_relative >> 1) as u32) & 0xFF;
    0xD000 | ((cond as u16) << 8) | (imm8 as u16)
}

// ── Thumb-2 (32-bit) helpers ──────────────────────────────────────────────────
//
// Returned as u32 in "Thumb-2 hi-then-lo" order: the result has the
// op1-bearing halfword in the high 16 bits and the op2 halfword in the
// low 16 bits.  Feed the returned u32 directly into `t2_words()`.

/// `MOV.W Rd, #imm16` — T3 encoding.  Loads any 16-bit immediate into
/// a low register (and the runner can target any of r0..r12 since the
/// encoding has a 4-bit Rd field).
pub fn movw_imm16(rd: u8, imm16: u16) -> u32 {
    assert!(rd <= 12, "MOV.W T3 Rd must be r0..r12 (got r{rd})");
    movw_movt_common(0b1111_0010_0100_0000u32, rd, imm16)
}

/// `MOVT Rd, #imm16` — T1 encoding.  Writes `imm16` into Rd's top 16 bits,
/// leaving the bottom 16 unchanged.  Paired with [`movw_imm16`] this loads
/// an arbitrary 32-bit constant (e.g. a peripheral MMIO base) into a
/// register without a literal pool.  The encoding is identical to MOV.W's
/// T3 except bit 7 of the first halfword (0xF2C0 base vs 0xF240).
pub fn movt_imm16(rd: u8, imm16: u16) -> u32 {
    assert!(rd <= 12, "MOVT T1 Rd must be r0..r12 (got r{rd})");
    movw_movt_common(0b1111_0010_1100_0000u32, rd, imm16)
}

/// Shared field-packing for MOV.W T3 / MOVT T1 (same layout, different base).
fn movw_movt_common(hi_base: u32, rd: u8, imm16: u16) -> u32 {
    let imm = imm16 as u32;
    let i = (imm >> 11) & 0x1;
    let imm4 = (imm >> 12) & 0xF;
    let imm3 = (imm >> 8) & 0x7;
    let imm8 = imm & 0xFF;
    let hi = hi_base | (i << 10) | imm4;
    let lo = (imm3 << 12) | ((rd as u32) << 8) | imm8;
    (hi << 16) | lo
}

/// One Thumb instruction for the mixed-width [`assemble`] stream: a 16-bit
/// halfword (`H`) or a 32-bit Thumb-2 word (`W`, hi-then-lo order).
///
/// Peripheral-execution oracle programs interleave 32-bit `MOV.W`/`MOVT`
/// (to materialise MMIO addresses) with 16-bit `STR`/`LDR`, so neither
/// [`ThumbOracleCase::halfwords`] (16-bit only) nor
/// [`ThumbOracleCase::t2_words`] (32-bit only) fits — `Thumb` carries both.
#[derive(Clone, Copy, Debug)]
pub enum Thumb {
    /// 16-bit Thumb-1 instruction.
    H(u16),
    /// 32-bit Thumb-2 instruction (high halfword in bits 31..16).
    W(u32),
    /// Raw little-endian 32-bit data word (natural byte order). For vector
    /// tables / literal pools — NOT an instruction, so unlike `W` it is not
    /// hi-then-lo swapped.
    Data(u32),
}

/// Assemble a mixed 16/32-bit Thumb instruction stream into little-endian
/// bytes.  Thumb-2 words are emitted high-halfword-first (each halfword LE),
/// matching [`ThumbOracleCase::t2_words`]; `Data` words are plain LE.
pub fn assemble(insns: &[Thumb]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(insns.len() * 2);
    for insn in insns {
        match insn {
            Thumb::H(h) => bytes.extend_from_slice(&h.to_le_bytes()),
            Thumb::W(w) => {
                let hi = ((*w >> 16) & 0xFFFF) as u16;
                let lo = (*w & 0xFFFF) as u16;
                bytes.extend_from_slice(&hi.to_le_bytes());
                bytes.extend_from_slice(&lo.to_le_bytes());
            }
            Thumb::Data(d) => bytes.extend_from_slice(&d.to_le_bytes()),
        }
    }
    bytes
}

/// `BX Rm` — T1 branch-and-exchange (`0b0100_0111_0_rmmmm_000`). With `Rm = lr`
/// (14) this is the canonical exception return (`0x4770`).
pub fn bx(rm: u8) -> u16 {
    assert!(rm < 16, "BX Rm out of range");
    0x4700 | ((rm as u16) << 3)
}

/// `CPSIE i` — clear PRIMASK (enable IRQs). T1 encoding `0xB662`.
pub fn cpsie_i() -> u16 {
    0xB662
}

/// `UDIV Rd, Rn, Rm` — T1 encoding (ARMv7-M unsigned divide).
pub fn udiv(rd: u8, rn: u8, rm: u8) -> u32 {
    assert!(
        rd <= 12 && rn <= 12 && rm <= 12,
        "UDIV Rd/Rn/Rm must be r0..r12"
    );
    let hi = 0b1111_1011_1011_0000u32 | (rn as u32);
    let lo = 0b1111_0000_1111_0000u32 | ((rd as u32) << 8) | (rm as u32);
    (hi << 16) | lo
}

/// `SDIV Rd, Rn, Rm` — T1 encoding.
pub fn sdiv(rd: u8, rn: u8, rm: u8) -> u32 {
    assert!(
        rd <= 12 && rn <= 12 && rm <= 12,
        "SDIV Rd/Rn/Rm must be r0..r12"
    );
    let hi = 0b1111_1011_1001_0000u32 | (rn as u32);
    let lo = 0b1111_0000_1111_0000u32 | ((rd as u32) << 8) | (rm as u32);
    (hi << 16) | lo
}

// ── Register access helpers ────────────────────────────────────────────────────
//
// CortexM exposes r0..r12 / sp / lr / pc as separate `pub u32` fields
// rather than an array, so we dispatch by index here.

fn write_arm_reg(cpu: &mut labwired_core::cpu::cortex_m::CortexM, idx: u8, v: u32) {
    use labwired_core::Cpu;
    match idx {
        0 => cpu.r0 = v,
        1 => cpu.r1 = v,
        2 => cpu.r2 = v,
        3 => cpu.r3 = v,
        4 => cpu.r4 = v,
        5 => cpu.r5 = v,
        6 => cpu.r6 = v,
        7 => cpu.r7 = v,
        8 => cpu.r8 = v,
        9 => cpu.r9 = v,
        10 => cpu.r10 = v,
        11 => cpu.r11 = v,
        12 => cpu.r12 = v,
        13 => cpu.sp = v,
        14 => cpu.lr = v,
        15 => cpu.set_pc(v),
        _ => panic!("write_arm_reg: bad index {idx}"),
    }
}

fn read_arm_reg(cpu: &labwired_core::cpu::cortex_m::CortexM, idx: u8) -> u32 {
    match idx {
        0 => cpu.r0,
        1 => cpu.r1,
        2 => cpu.r2,
        3 => cpu.r3,
        4 => cpu.r4,
        5 => cpu.r5,
        6 => cpu.r6,
        7 => cpu.r7,
        8 => cpu.r8,
        9 => cpu.r9,
        10 => cpu.r10,
        11 => cpu.r11,
        12 => cpu.r12,
        13 => cpu.sp,
        14 => cpu.lr,
        15 => cpu.pc,
        _ => panic!("read_arm_reg: bad index {idx}"),
    }
}

// ── RAM peripheral ─────────────────────────────────────────────────────────────

mod ram_peripheral {
    use labwired_core::{Peripheral, SimResult};

    pub struct RamPeripheral {
        data: Vec<u8>,
    }

    impl RamPeripheral {
        pub fn new(size: usize) -> Self {
            Self {
                data: vec![0u8; size],
            }
        }

        pub fn write_bytes(&mut self, offset: usize, bytes: &[u8]) {
            self.data[offset..offset + bytes.len()].copy_from_slice(bytes);
        }
    }

    impl std::fmt::Debug for RamPeripheral {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "RamPeripheral({}B)", self.data.len())
        }
    }

    impl Peripheral for RamPeripheral {
        fn read(&self, offset: u64) -> SimResult<u8> {
            Ok(*self.data.get(offset as usize).unwrap_or(&0))
        }

        fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
            if let Some(slot) = self.data.get_mut(offset as usize) {
                *slot = value;
            }
            Ok(())
        }
    }
}

// ── Sim capture ────────────────────────────────────────────────────────────────

fn capture_sim_state(case: &ThumbOracleCase) -> ThumbOracleState {
    use labwired_core::bus::SystemBus;
    use ram_peripheral::RamPeripheral;

    // Peripheral-execution path: a full chip bus was supplied.  Build it,
    // load the program into SRAM at PROG_BASE_HW (the same address the HW
    // runner uses), and run from there so the program's MMIO accesses hit
    // real peripheral models — exactly mirroring silicon.
    if let Some(factory) = &case.sim_bus {
        let mut bus = factory();
        let bytes: &[u8] = match &case.program {
            ThumbProgram::Asm(b) => b.as_slice(),
            ThumbProgram::Elf(_) => panic!(
                "thumb oracle: ELF programs are not supported on the full-chip \
                 sim bus; use mixed()/halfwords()/t2_words()"
            ),
        };
        // Write the program word by word into SRAM via the bus (the chip
        // config maps real RAM at this window).
        let mut i = 0;
        while i < bytes.len() {
            let mut w = [0u8; 4];
            let n = (bytes.len() - i).min(4);
            w[..n].copy_from_slice(&bytes[i..i + n]);
            let word = u32::from_le_bytes(w);
            labwired_core::Bus::write_u32(&mut bus, (PROG_BASE_HW + i as u32) as u64, word)
                .unwrap_or_else(|e| {
                    panic!(
                        "thumb oracle: load program word at 0x{:08X} failed: {e:?}",
                        PROG_BASE_HW + i as u32
                    )
                });
            i += 4;
        }
        return run_capture(case, bus, PROG_BASE_HW + case.entry_offset, true);
    }

    // Use empty() to avoid the STM32-default peripherals colliding with
    // our oracle program at PROG_BASE.  Bit-band is also disabled here,
    // which matches the C3 harness reasoning.
    let mut bus = SystemBus::empty();
    let entry_pc: u32;

    match &case.program {
        ThumbProgram::Asm(bytes) => {
            let mut prog = RamPeripheral::new(ORACLE_MEM_SIZE);
            prog.write_bytes(0, bytes);
            bus.add_peripheral(
                "oracle_prog",
                PROG_BASE as u64,
                ORACLE_MEM_SIZE as u64,
                None,
                Box::new(prog),
            );
            entry_pc = PROG_BASE;
        }
        ThumbProgram::Elf(path) => {
            use goblin::elf::program_header::PT_LOAD;
            use goblin::elf::Elf;

            let elf_bytes = std::fs::read(path)
                .unwrap_or_else(|e| panic!("thumb oracle: failed to read ELF {path:?}: {e}"));
            let elf = Elf::parse(&elf_bytes)
                .unwrap_or_else(|e| panic!("thumb oracle: failed to parse ELF {path:?}: {e}"));

            entry_pc = (elf.entry as u32) & !1; // strip Thumb bit

            let mut prog = RamPeripheral::new(ORACLE_MEM_SIZE);
            for ph in &elf.program_headers {
                if ph.p_type != PT_LOAD || ph.p_filesz == 0 {
                    continue;
                }
                let vaddr = ph.p_vaddr as u32;
                let offset_in_prog = vaddr.checked_sub(PROG_BASE).unwrap_or_else(|| {
                    panic!(
                        "thumb oracle: ELF VAddr 0x{vaddr:08X} is below PROG_BASE 0x{PROG_BASE:08X}"
                    )
                }) as usize;
                let size = ph.p_filesz as usize;
                let file_offset = ph.p_offset as usize;
                let seg_data = &elf_bytes[file_offset..file_offset + size];
                prog.write_bytes(offset_in_prog, seg_data);
            }
            bus.add_peripheral(
                "oracle_prog",
                PROG_BASE as u64,
                ORACLE_MEM_SIZE as u64,
                None,
                Box::new(prog),
            );
        }
    }

    // Data window for memory tests.
    bus.add_peripheral(
        "oracle_data",
        DATA_BASE as u64,
        ORACLE_MEM_SIZE as u64,
        None,
        Box::new(ram_peripheral::RamPeripheral::new(ORACLE_MEM_SIZE)),
    );

    run_capture(case, bus, entry_pc, false)
}

/// Shared sim driver: reset the CPU on `bus`, apply `case.setup`, run from
/// `entry_pc` until the `B .` terminator settles the PC, and snapshot the
/// end state (registers + captured memory).  Used by both the bare-bus
/// instruction-oracle path and the full-chip peripheral-execution path.
fn run_capture(
    case: &ThumbOracleCase,
    mut bus: labwired_core::bus::SystemBus,
    entry_pc: u32,
    cortex_m_system: bool,
) -> ThumbOracleState {
    use labwired_core::cpu::cortex_m::CortexM;
    use labwired_core::Cpu;

    // For interrupt-delivery oracles the CPU must share the bus's NVIC/SCB/VTOR
    // state, so build it through the Cortex-M system wiring; otherwise a bare
    // CPU is enough (and is what the RAM-only instruction oracles use).
    let mut cpu = if cortex_m_system {
        let (cpu, _nvic) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
        cpu
    } else {
        CortexM::new()
    };
    // reset() reads SP+PC from the vector table at VTOR (0).  On the bare
    // bus that window is unmapped (reads fail → defaults kept); on a full
    // chip bus flash is mapped but unprogrammed (zeros) — either way the
    // explicit overrides below set the real entry state.
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(entry_pc);
    cpu.sp = INIT_SP;
    // Match real Cortex-M post-reset convention: per ARMv7-M ARM B1.4.3,
    // LR is set to 0xFFFFFFFF after reset (treated as the EXC_RETURN
    // sentinel in handler mode).  The CortexM::reset path zeroes LR by
    // default; the oracle harness aligns it with silicon so `_diff`
    // runners don't trip on this purely-architectural cosmetic difference.
    cpu.lr = 0xFFFF_FFFF;

    // Apply setup.  CortexM exposes r0..r12 as separate `pub` fields
    // rather than an array, so we dispatch by index.
    let mut init_state = ThumbOracleState::default();
    (case.setup)(&mut init_state);
    for (name, &val) in &init_state.regs {
        if let Some(idx) = parse_r_name(name) {
            write_arm_reg(&mut cpu, idx, val);
        }
    }
    for (&addr, &val) in &init_state.mem {
        labwired_core::Bus::write_u32(&mut bus, addr as u64, val).unwrap_or_else(|e| {
            panic!("thumb oracle: setup write_u32(0x{addr:08X}) failed: {e:?}")
        });
    }

    // Step until PC settles on the B-self terminator (or limit). This is a
    // specialized bare-CPU oracle boundary: when live peripherals are enabled,
    // publish one cycle before each instruction so attached lazy-clock models
    // (notably DWT under `event-scheduler`) observe advancing simulated time.
    let sim_config = labwired_core::SimulationConfig::default();
    let mut live_cycle = 0u64;
    let mut last_pc = cpu.pc;
    let mut stable_count: u32 = 0;
    for _ in 0..MAX_STEPS {
        if case.live_peripherals {
            live_cycle += 1;
            bus.set_current_cycle(live_cycle);
        }
        cpu.step(&mut bus, &[], &sim_config)
            .unwrap_or_else(|e| panic!("thumb oracle sim error at pc=0x{:08X}: {e:?}", cpu.pc));
        // Force the compatibility walk even when the event-scheduler feature
        // deletes production's legacy walk. This oracle intentionally owns its
        // bare-CPU/peripheral lifecycle instead of draining Machine events.
        if case.live_peripherals {
            let (interrupts, _costs) = bus.tick_peripherals_fully_forced();
            for irq in interrupts {
                cpu.set_exception_pending(irq);
            }
        }
        if cpu.pc == last_pc {
            stable_count += 1;
            if stable_count >= 2 {
                break;
            }
        } else {
            stable_count = 0;
            last_pc = cpu.pc;
        }
    }

    // Let any autonomous engine the program armed (DMA mem-to-mem, …) run to
    // completion before the snapshot.  On silicon these run concurrently and
    // have long finished by the breakpoint halt; in sim they advance one
    // element per peripheral tick, so tick explicitly with the CPU frozen.
    for _ in 0..case.settle_ticks {
        live_cycle += 1;
        bus.set_current_cycle(live_cycle);
        let _ = bus.tick_peripherals_fully_forced();
    }

    // Build end state.
    let mut end = ThumbOracleState::default();
    for i in 0..13u8 {
        end.regs.insert(format!("r{i}"), read_arm_reg(&cpu, i));
    }
    end.regs.insert("sp".to_string(), cpu.sp);
    end.regs.insert("lr".to_string(), cpu.lr);
    end.regs.insert("pc".to_string(), cpu.pc);
    // Capture the program status register so cases can assert the APSR
    // condition flags (NZCV in bits 31..28).
    end.regs.insert("xpsr".to_string(), cpu.xpsr);
    end.pc = cpu.pc;

    let mut addrs: Vec<u32> = init_state.mem.keys().copied().collect();
    addrs.extend_from_slice(&case.mem_capture_addrs);
    addrs.sort_unstable();
    addrs.dedup();
    for addr in addrs {
        let val = labwired_core::Bus::read_u32(&bus, addr as u64)
            .unwrap_or_else(|e| panic!("thumb oracle: end read_u32(0x{addr:08X}) failed: {e:?}"));
        end.mem.insert(addr, val);
    }
    end
}

/// Execute `case` in the software simulator and run its expect closure.
pub fn run_sim(case: ThumbOracleCase) {
    let end_state = capture_sim_state(&case);
    (case.expect)(&end_state);
}

/// Capture HW end state by executing `case` on a physical STM32 board
/// via OpenOCD over ST-Link SWD.
///
/// Procedure:
///   1. Spawn OpenOCD with `interface/stlink.cfg` + `target/stm32f4x.cfg`.
///   2. `reset halt` — bring the CPU to a known stopped state.
///   3. Write program bytes into SRAM at [`PROG_BASE_HW`] (4-byte chunks).
///   4. Compute the terminator address (= PROG_BASE_HW + program_len − 2;
///      `from_bytes()`/`halfwords()`/`t2_words()` always append the
///      `B .` halfword as the last 2 bytes).
///   5. Set a hardware breakpoint at the terminator address via the
///      OpenOCD `bp` TCL command (Cortex-M's FPB unit gives us 6
///      breakpoints; we only need 1).
///   6. Apply setup: write r0..r12 / sp / lr, then memory.  SP is also
///      set to [`INIT_SP`] (top of SRAM) so the program has a valid
///      stack even if it never touches one.
///   7. `reg pc PROG_BASE_HW | 1` — Thumb bit set so the CPU executes
///      in Thumb mode (Cortex-M is always Thumb, but the bit is part
///      of the ISA mode encoding and PC writes that clear it cause
///      INVSTATE).
///   8. `resume` — execution runs from PROG_BASE_HW through the program
///      and halts at the breakpoint.
///   9. `wait_until_halted` (5-second budget — even pathological tests
///      finish well under MAX_STEPS cycles at silicon speed).
///  10. Snapshot end state: read r0..r12, sp, lr, pc, then re-read the
///      memory addresses the caller cares about.
///  11. Remove the breakpoint and shutdown OpenOCD.
#[cfg(feature = "hw-oracle-stm32")]
fn capture_hw_state(case: &ThumbOracleCase) -> ThumbOracleState {
    use crate::openocd::OpenOcd;
    use std::time::Duration;

    // 1. Spawn OpenOCD against the attached ST-Link.  STM32 family is
    // selectable via the `STM32_TARGET` env var (e.g. "stm32f1x" for
    // F103, "stm32f4x" for F401); defaults to F4 since that's the chip
    // the bank was originally silicon-validated against.
    let target = std::env::var("STM32_TARGET").unwrap_or_else(|_| "stm32f4x".to_string());
    let mut oc = OpenOcd::spawn_stm32(&target).unwrap_or_else(|e| {
        panic!("run_hw: failed to spawn OpenOCD for {target} — is the board attached? {e:?}")
    });

    // 2. Reset + halt the CPU.
    oc.reset_halt().expect("run_hw: reset_halt failed");
    oc.halt().expect("run_hw: halt after reset_halt failed");

    // 3. Determine program bytes.  Only Asm programs are supported via
    // this slice — ELF loading on HW would require resolving multiple
    // PT_LOAD segments to specific RAM addresses, which is a follow-up.
    let bytes: &[u8] = match &case.program {
        ThumbProgram::Asm(b) => b.as_slice(),
        ThumbProgram::Elf(_) => panic!(
            "run_hw: ELF programs not yet supported for STM32 HW oracle. \
             Use halfwords/t2_words/from_bytes-built cases."
        ),
    };
    assert!(
        bytes.len() >= 2 && bytes.len() % 2 == 0,
        "run_hw: program must be a non-empty halfword sequence"
    );

    // Write program to SRAM in 32-bit chunks.  If the program length is
    // odd in halfwords (i.e. byte length % 4 == 2), pad the final word
    // with a NOP so we always write whole words.  The B-self terminator
    // is the last halfword and gets the breakpoint, so trailing pad
    // bytes never execute.
    let mut words: Vec<u32> = Vec::with_capacity((bytes.len() + 3) / 4);
    let mut i = 0;
    while i + 4 <= bytes.len() {
        words.push(u32::from_le_bytes([
            bytes[i],
            bytes[i + 1],
            bytes[i + 2],
            bytes[i + 3],
        ]));
        i += 4;
    }
    if i < bytes.len() {
        // 2 trailing bytes — pad with 0xBF00 NOP in the upper halfword.
        let lo = u16::from_le_bytes([bytes[i], bytes[i + 1]]) as u32;
        words.push(lo | 0xBF00_0000);
    }
    oc.write_memory(PROG_BASE_HW, &words)
        .expect("run_hw: write_memory(program) failed");

    // 4. Terminator address — last 2 bytes of the program.  Cortex-M
    // hardware breakpoints take the *halfword-aligned* instruction
    // address with the Thumb bit cleared.
    let terminator_addr = PROG_BASE_HW + (bytes.len() as u32) - 2;

    // 5. Hardware breakpoint via OpenOCD TCL.  Format: `bp <addr> 2 hw`
    // for a 2-byte (Thumb-1) HW breakpoint.  Returns "breakpoint set …"
    // or an error string.
    let bp_resp = oc
        .tcl(&format!("bp 0x{terminator_addr:08X} 2 hw"))
        .expect("run_hw: bp TCL command failed");
    assert!(
        !bp_resp.contains("Error"),
        "run_hw: setting breakpoint failed: {bp_resp}"
    );

    // 6. Apply setup state.  First zero r0..r12 so the HW baseline
    // matches the sim baseline — silicon (or OpenOCD's reset path)
    // leaves arbitrary values in unused registers, which would surface
    // as spurious mismatches in `_diff` tests that only write a subset
    // of registers explicitly.  LR is set to the architectural
    // post-reset convention (0xFFFFFFFF, EXC_RETURN sentinel) to match
    // what `capture_sim_state` does on the sim side.
    for i in 0..13u8 {
        oc.write_register(&format!("r{i}"), 0)
            .unwrap_or_else(|e| panic!("run_hw: zero r{i} failed: {e:?}"));
    }
    oc.write_register("lr", 0xFFFF_FFFF)
        .expect("run_hw: write lr failed");

    let mut init_state = ThumbOracleState::default();
    (case.setup)(&mut init_state);
    // Default SP — overridden if setup wrote "sp".
    oc.write_register("sp", INIT_SP)
        .expect("run_hw: write sp failed");
    for (name, &val) in &init_state.regs {
        if parse_r_name(name).is_some() {
            // OpenOCD accepts r0..r15 / sp / lr / pc as register names.
            oc.write_register(name, val)
                .unwrap_or_else(|e| panic!("run_hw: write reg {name} failed: {e:?}"));
        }
    }
    for (&addr, &val) in &init_state.mem {
        oc.write_memory(addr, &[val])
            .unwrap_or_else(|e| panic!("run_hw: setup write_memory(0x{addr:08X}) failed: {e:?}"));
    }

    // 7. PC = entry (PROG_BASE_HW + entry_offset) with Thumb bit set. The
    // offset lets a vector table sit at the load base while `main` runs after
    // it (interrupt-delivery oracles).
    oc.write_register("pc", (PROG_BASE_HW + case.entry_offset) | 1)
        .expect("run_hw: write pc failed");

    // 8. Resume execution.
    oc.resume().expect("run_hw: resume failed");

    // 9. Wait for the breakpoint to fire.
    oc.wait_until_halted(Duration::from_secs(5))
        .expect("run_hw: program did not halt within 5s — runaway?");

    // 10. Snapshot end state.
    let mut end = ThumbOracleState::default();
    for i in 0..13u8 {
        let v = oc
            .read_register(&format!("r{i}"))
            .unwrap_or_else(|e| panic!("run_hw: read reg r{i} failed: {e:?}"));
        end.regs.insert(format!("r{i}"), v);
    }
    end.regs
        .insert("sp".to_string(), oc.read_register("sp").unwrap());
    end.regs
        .insert("lr".to_string(), oc.read_register("lr").unwrap());
    let final_pc = oc.read_register("pc").unwrap();
    end.regs.insert("pc".to_string(), final_pc);
    // OpenOCD exposes the Cortex-M program status register as "xpsr".
    end.regs
        .insert("xpsr".to_string(), oc.read_register("xpsr").unwrap_or(0));
    end.pc = final_pc;

    let mut addrs: Vec<u32> = init_state.mem.keys().copied().collect();
    addrs.extend_from_slice(&case.mem_capture_addrs);
    addrs.sort_unstable();
    addrs.dedup();
    for addr in addrs {
        let val = oc
            .read_memory(addr, 1)
            .unwrap_or_else(|e| panic!("run_hw: end read_memory(0x{addr:08X}) failed: {e:?}"));
        end.mem.insert(addr, val[0]);
    }

    // 11. Remove breakpoint + shutdown.
    let _ = oc.tcl(&format!("rbp 0x{terminator_addr:08X}"));
    let _ = oc.shutdown();
    end
}

/// Execute `case` against a physical STM32 board via SWD / OpenOCD and
/// run the expect closure on the HW end state.
#[cfg(feature = "hw-oracle-stm32")]
pub fn run_hw(case: ThumbOracleCase) {
    let end = capture_hw_state(&case);
    (case.expect)(&end);
}

/// Run `case` in both the simulator and on real silicon, diff the end
/// states.  Mirrors the S3 `run_diff` shape.
#[cfg(feature = "hw-oracle-stm32")]
pub fn run_diff(case: ThumbOracleCase) {
    // Capture sim first (no hardware contention).
    let sim_end = capture_sim_state(&case);
    let hw_end = capture_hw_state(&case);

    // Diff every captured register.
    for name in [
        "r0", "r1", "r2", "r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10", "r11", "r12", "sp", "lr",
    ] {
        let sim_v = sim_end.read_reg(name);
        let hw_v = hw_end.read_reg(name);
        assert_eq!(
            sim_v, hw_v,
            "diff: register {name}: sim 0x{sim_v:08X} vs hw 0x{hw_v:08X}"
        );
    }
    // Diff the APSR condition flags (NZCV, bits 31..28). Mode bits (T-bit,
    // IPSR) are masked out: they're architecturally fixed here (thread mode,
    // Thumb) and not what these oracles validate.
    let sim_nzcv = sim_end.read_reg("xpsr") & NZCV_MASK;
    let hw_nzcv = hw_end.read_reg("xpsr") & NZCV_MASK;
    assert_eq!(
        sim_nzcv, hw_nzcv,
        "diff: APSR NZCV: sim 0x{sim_nzcv:08X} vs hw 0x{hw_nzcv:08X}"
    );

    // Don't diff PC — sim halts at the B-self instruction address, HW
    // halts one halfword *into* it because the FPB unit fires before
    // the instruction executes; the absolute addresses differ anyway
    // since sim uses PROG_BASE and HW uses PROG_BASE_HW.

    // Diff every memory address the case captured.
    let mut addrs: Vec<u32> = case.mem_capture_addrs.clone();
    addrs.sort_unstable();
    addrs.dedup();
    for addr in addrs {
        let sim_v = sim_end.read_mem(addr);
        let hw_v = hw_end.read_mem(addr);
        assert_eq!(
            sim_v, hw_v,
            "diff: mem[0x{addr:08X}]: sim 0x{sim_v:08X} vs hw 0x{hw_v:08X}"
        );
    }

    // After cross-validation, also assert the case's own expectations on
    // the HW side — catches cases where sim and HW agree but BOTH drift
    // from the spec.
    (case.expect)(&hw_end);
}

#[cfg(test)]
mod encoder_tests {
    use super::*;

    #[derive(Debug, Default)]
    struct DelayedIrq {
        ticks: u32,
    }

    impl labwired_core::Peripheral for DelayedIrq {
        fn read(&self, _offset: u64) -> labwired_core::SimResult<u8> {
            Ok(0)
        }

        fn write(&mut self, _offset: u64, _value: u8) -> labwired_core::SimResult<()> {
            Ok(())
        }

        fn tick(&mut self) -> labwired_core::PeripheralTickResult {
            self.ticks += 1;
            labwired_core::PeripheralTickResult {
                irq: self.ticks == 20,
                ..Default::default()
            }
        }
    }

    #[test]
    fn settle_ticks_freeze_cpu_even_when_an_irq_becomes_pending() {
        const NVIC_ISER0: u32 = 0xE000_E100;
        const IRQ: u32 = 6;

        let prog = vec![
            Thumb::W(movw_imm16(0, (NVIC_ISER0 & 0xFFFF) as u16)),
            Thumb::W(movt_imm16(0, (NVIC_ISER0 >> 16) as u16)),
            Thumb::W(movw_imm16(1, (1 << IRQ) as u16)),
            Thumb::W(movt_imm16(1, 0)),
            Thumb::H(str_imm5(1, 0, 0)),
        ];
        let halt_pc = PROG_BASE_HW + assemble(&prog).len() as u32;

        let case = ThumbOracleCase::mixed(&prog)
            .sim_bus(|| {
                let mut bus = labwired_core::bus::SystemBus::new();
                bus.add_peripheral(
                    "delayed_irq",
                    0x5000_0000,
                    0x100,
                    Some(IRQ),
                    Box::new(DelayedIrq::default()),
                );
                bus
            })
            .live_peripherals(true)
            .settle_ticks(32);

        let state = capture_sim_state(&case);
        state.assert_pc(halt_pc);
    }

    // Encodings cross-checked against ARMv7-M ARM (DDI 0403E.e).

    #[test]
    fn movs_r0_imm5_encoding() {
        // MOVS r0, #5 = 0b0010_0_000_00000101 = 0x2005
        assert_eq!(movs_imm8(0, 5), 0x2005);
    }

    #[test]
    fn movs_r3_imm0x42_encoding() {
        // MOVS r3, #0x42 = 0b0010_0_011_01000010 = 0x2342
        assert_eq!(movs_imm8(3, 0x42), 0x2342);
    }

    #[test]
    fn adds_r0_r1_r2_encoding() {
        // ADDS r0, r1, r2 = 0b0001100_010_001_000 = 0x1888
        assert_eq!(adds_reg(0, 1, 2), 0x1888);
    }

    #[test]
    fn subs_r0_r1_r2_encoding() {
        // SUBS r0, r1, r2 = 0b0001101_010_001_000 = 0x1A88
        assert_eq!(subs_reg(0, 1, 2), 0x1A88);
    }

    #[test]
    fn muls_r0_r1_encoding() {
        // MULS r0, r1, r0 (r0 = r1 * r0) = 0b0100001101_001_000 = 0x4348
        assert_eq!(muls(0, 1), 0x4348);
    }

    #[test]
    fn b_self_is_canonical_terminator() {
        // B . — branch with offset 0 from this instruction.  Encoding
        // subtracts 4 internally → imm11 = -2 → 0x7FE.  Final = 0xE7FE.
        assert_eq!(b_uncond(0), B_SELF);
    }

    #[test]
    fn movw_r0_0xbeef_encoding() {
        // MOV.W r0, #0xBEEF (T3 encoding) per ARMv7-M ARM §A6.7.74:
        //   imm32 = ZeroExtend(imm4:i:imm3:imm8, 32)
        // For 0xBEEF: imm4=0xB, i=1, imm3=6, imm8=0xEF.
        //   hi = 0xF240 | (i<<10) | imm4         = 0xF240 | 0x0400 | 0x000B = 0xF64B
        //   lo = (imm3<<12) | (Rd<<8) | imm8     = 0x6000 | 0x0000 | 0x00EF = 0x60EF
        // Cross-checked: arm-none-eabi-as `movw r0, #0xBEEF` → 4b f6 ef 60
        // (LE byte stream: hi=0xF64B, lo=0x60EF).
        assert_eq!(movw_imm16(0, 0xBEEF), 0xF64B_60EF);
    }

    // Register-offset load/store family (ARMv7-M ARM §A6.7, T1):
    //   base | (Rm << 6) | (Rn << 3) | Rt, with Rt=2/3, Rn=1, Rm=0.
    #[test]
    fn reg_offset_encodings() {
        assert_eq!(str_reg(2, 1, 0), 0x500A); // STR  r2,[r1,r0]
        assert_eq!(strh_reg(2, 1, 0), 0x520A); // STRH r2,[r1,r0]
        assert_eq!(strb_reg(2, 1, 0), 0x540A); // STRB r2,[r1,r0]
        assert_eq!(ldrsb_reg(3, 1, 0), 0x560B); // LDRSB r3,[r1,r0]
        assert_eq!(ldr_reg(3, 1, 0), 0x580B); // LDR  r3,[r1,r0]
        assert_eq!(ldrh_reg(3, 1, 0), 0x5A0B); // LDRH r3,[r1,r0]
        assert_eq!(ldrb_reg(3, 1, 0), 0x5C0B); // LDRB r3,[r1,r0]
        assert_eq!(ldrsh_reg(3, 1, 0), 0x5E0B); // LDRSH r3,[r1,r0]
    }

    #[test]
    fn register_shift_encodings() {
        // Two-arg register-shift T1 (Rdn=2, Rm=1): base | (Rm<<3) | Rdn.
        assert_eq!(lsl_reg(2, 1), 0x408A); // LSLS r2, r1
        assert_eq!(lsr_reg(2, 1), 0x40CA); // LSRS r2, r1
        assert_eq!(asr_reg(2, 1), 0x410A); // ASRS r2, r1
        assert_eq!(ror_reg(2, 1), 0x41CA); // RORS r2, r1
    }

    #[test]
    fn movt_r0_0x4002_encoding() {
        // MOVT r0, #0x4002 (T1) per ARMv7-M ARM §A6.7.79: same field layout
        // as MOV.W T3 but hi base 0xF2C0. imm16=0x4002 → imm4=4,i=0,imm3=0,
        // imm8=0x02. hi=0xF2C0|4=0xF2C4, lo=0x0002.
        // Cross-checked: arm-none-eabi-as `movt r0,#0x4002` → c4 f2 02 00.
        assert_eq!(movt_imm16(0, 0x4002), 0xF2C4_0002);
    }

    #[test]
    fn assemble_mixes_16_and_32_bit() {
        // W(movw r0,#0x101C) then H(b .) → hi,lo of the word (each LE) then
        // the halfword (LE).  movw r0,#0x101C: imm4=1,i=0,imm3=0,imm8=0x1C →
        // hi=0xF241, lo=0x001C.
        let bytes = assemble(&[Thumb::W(movw_imm16(0, 0x101C)), Thumb::H(B_SELF)]);
        assert_eq!(bytes, vec![0x41, 0xF2, 0x1C, 0x00, 0xFE, 0xE7]);
    }

    #[test]
    fn it_eq_single_encoding() {
        // IT EQ (one-instruction block): 0xBF00 | (cond<<4) | mask, cond=EQ=0,
        // mask=0b1000. Cross-checked: arm-none-eabi-as `it eq` → 08 bf (0xBF08).
        assert_eq!(it(COND_EQ, 0x8), 0xBF08);
    }

    #[test]
    fn bx_lr_and_cpsie_encodings() {
        // BX LR = 0x4700 | (14<<3) = 0x4770 (arm-none-eabi-as `bx lr` → 70 47).
        assert_eq!(bx(14), 0x4770);
        // CPSIE i = 0xB662 (arm-none-eabi-as `cpsie i` → 62 b6).
        assert_eq!(cpsie_i(), 0xB662);
    }

    #[test]
    fn data_word_is_plain_little_endian() {
        // Unlike a 32-bit instruction (`W`, hi-then-lo), a `Data` word is emitted
        // in natural LE order — required for vector-table entries.
        assert_eq!(
            assemble(&[Thumb::Data(0x2000_2081)]),
            vec![0x81, 0x20, 0x00, 0x20]
        );
    }
}
