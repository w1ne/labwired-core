//! RISC-V (ESP32-C3) oracle harness.
//!
//! Parallel to the Xtensa `OracleCase` in `lib.rs`, but for RV32IMC firmware
//! on ESP32-C3.  An `RiscVOracleCase` bundles a small program (raw bytes or
//! an ELF), a setup closure that writes initial register state, and an expect
//! closure that asserts post-execution register and memory values.
//!
//! # Runners
//!
//! * [`run_sim`] — executes the program in `RiscV` + `SystemBus`; always
//!   available.
//! * [`run_hw`] — flashes/writes the program to a real ESP32-C3 via OpenOCD;
//!   gated on the `hw-oracle-c3` feature.
//! * [`run_diff`] — runs both and diffs the end state; gated on
//!   `hw-oracle-c3`.
//!
//! # Memory map
//!
//! Programs load at [`PROG_BASE`] (0x4200_0000, ESP32-C3 flash XIP window) so
//! they live where real C3 firmware lives.  Data uses [`DATA_BASE`]
//! (0x3FC8_0000, SRAM).  Both match the addresses in
//! `core/configs/chips/esp32c3.yaml` and the ESP32-C3 TRM §3.1.
//!
//! # Program terminator
//!
//! RV32 has no instruction that cleanly halts the simulator (EBREAK on the
//! RiscV core invokes the trap handler).  Oracle programs therefore end with
//! a `J . ` (jump-to-self) sentinel; the runner steps until the PC settles
//! on the sentinel address.

use std::collections::HashMap;
use std::path::PathBuf;

/// Start of the ESP32-C3 flash XIP window — where firmware is executed
/// from in normal operation, and where oracle programs are loaded.
pub const PROG_BASE: u32 = 0x4200_0000;

/// Start of the ESP32-C3 SRAM (data) window.
pub const DATA_BASE: u32 = 0x3FC8_0000;

/// Size of the scratch program/data window we register on the bus.
/// 64 KiB is comfortably larger than any oracle program.
pub const ORACLE_MEM_SIZE: usize = 64 * 1024;

/// `J 0` — jump-to-self.  Encoded as `JAL x0, 0` (opcode 0x6F, rd=0,
/// imm=0).  Used as the program terminator: the simulator runs until PC
/// pins on the sentinel address.
const J_SELF: u32 = 0x0000_006F;

/// Maximum simulator steps before declaring the program runaway.
const MAX_STEPS: usize = 10_000;

// ── State ──────────────────────────────────────────────────────────────────────

/// Snapshot of CPU + memory state used by RISC-V oracle tests.
///
/// In **setup**: populated by `write_reg`/`write_mem` to describe initial
/// conditions; the runner writes these into the CPU and bus before
/// execution.
///
/// In **end state**: re-read from the CPU/bus after execution halts.  All
/// 32 GPRs are captured; memory is re-read for every address mentioned in
/// setup plus every address in `RiscVOracleCase::mem_capture_addrs`.
#[derive(Default, Debug, Clone)]
pub struct RiscVOracleState {
    /// Register values keyed by RV32 name: `"x0"`..`"x31"`.  ABI names
    /// (`"ra"`, `"sp"`, `"a0"` …) are NOT recognised — use the architectural
    /// names so tests stay unambiguous across calling conventions.
    pub regs: HashMap<String, u32>,
    /// Memory snapshot (word-aligned address → 32-bit word, little-endian).
    pub mem: HashMap<u32, u32>,
    /// PC after execution halts (the address of the J-self terminator on
    /// successful completion).
    pub pc: u32,
}

impl RiscVOracleState {
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
            "riscv oracle: register {name}: expected 0x{expected:08X}, got 0x{actual:08X}"
        );
    }

    pub fn assert_reg_masked(&self, name: &str, mask: u32, expected: u32) {
        let actual = self.read_reg(name) & mask;
        assert_eq!(
            actual, expected,
            "riscv oracle: register {name} masked by 0x{mask:08X}: \
             expected 0x{expected:08X}, got 0x{actual:08X}"
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
            "riscv oracle: mem[0x{addr:08X}]: expected 0x{expected:08X}, got 0x{actual:08X}"
        );
    }

    pub fn assert_pc(&self, expected: u32) {
        assert_eq!(
            self.pc, expected,
            "riscv oracle: pc: expected 0x{expected:08X}, got 0x{:08X}",
            self.pc
        );
    }
}

/// Parse `"xN"` → register index N (0..=31).  Returns `None` for other names.
fn parse_x_name(name: &str) -> Option<u8> {
    let n = name.strip_prefix('x')?;
    let idx: u8 = n.parse().ok()?;
    if idx < 32 {
        Some(idx)
    } else {
        None
    }
}

// ── Program + case ─────────────────────────────────────────────────────────────

/// A program to execute: either raw bytes (already 4-byte LE encoded) or an
/// ELF file on disk.
pub enum RiscVProgram {
    Asm(Vec<u8>),
    Elf(PathBuf),
}

/// A RISC-V oracle test case: program + initial state + expected end state.
pub struct RiscVOracleCase {
    pub program: RiscVProgram,
    pub setup: Box<dyn Fn(&mut RiscVOracleState) + Send + Sync>,
    pub expect: Box<dyn Fn(&RiscVOracleState) + Send + Sync>,
    /// Additional addresses to read from the bus into `end_state.mem` after
    /// execution.  Used for store-only tests (SB/SH/SW) where setup does
    /// not pre-populate `mem` but the test wants to assert what was written.
    pub mem_capture_addrs: Vec<u32>,
}

impl RiscVOracleCase {
    /// Build an oracle case from raw little-endian instruction bytes.
    ///
    /// A `J . ` (jump-to-self) terminator is appended automatically so that
    /// `run_sim` knows when to stop.
    pub fn from_bytes(mut bytes: Vec<u8>) -> Self {
        bytes.extend_from_slice(&J_SELF.to_le_bytes());
        Self {
            program: RiscVProgram::Asm(bytes),
            setup: Box::new(|_| {}),
            expect: Box::new(|_| {}),
            mem_capture_addrs: Vec::new(),
        }
    }

    /// Build an oracle case from a sequence of 32-bit RV32 words.
    ///
    /// Each word is emitted as 4 little-endian bytes.  A `J . ` terminator
    /// is appended automatically.
    pub fn words(words: &[u32]) -> Self {
        let mut bytes = Vec::with_capacity(words.len() * 4);
        for w in words {
            bytes.extend_from_slice(&w.to_le_bytes());
        }
        Self::from_bytes(bytes)
    }

    /// Build an oracle case by parsing one or more `.word 0xXXXXXXXX` lines.
    ///
    /// Each line is expected to be of the form `.word 0x<hex>` (case-
    /// insensitive, optional whitespace).  The 32-bit value is emitted as 4
    /// little-endian bytes — RV32 instructions are 4 bytes each (or 2 if
    /// compressed, but the parser does not distinguish; emit two `.word`
    /// halfwords back-to-back if you need to mix 16-bit insns).
    ///
    /// A `J . ` terminator is appended automatically.
    pub fn asm(s: &str) -> Self {
        let bytes = parse_dot_word(s);
        Self::from_bytes(bytes)
    }

    pub fn elf(path: &str) -> Self {
        Self {
            program: RiscVProgram::Elf(PathBuf::from(path)),
            setup: Box::new(|_| {}),
            expect: Box::new(|_| {}),
            mem_capture_addrs: Vec::new(),
        }
    }

    pub fn setup<F>(mut self, f: F) -> Self
    where
        F: Fn(&mut RiscVOracleState) + Send + Sync + 'static,
    {
        self.setup = Box::new(f);
        self
    }

    pub fn expect<F>(mut self, f: F) -> Self
    where
        F: Fn(&RiscVOracleState) + Send + Sync + 'static,
    {
        self.expect = Box::new(f);
        self
    }

    pub fn capture_mem(mut self, addrs: &[u32]) -> Self {
        self.mem_capture_addrs.extend_from_slice(addrs);
        self
    }
}

fn parse_dot_word(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for line in s.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('#') {
            continue;
        }
        let rest = line.strip_prefix(".word").unwrap_or(line).trim();
        for token in rest.split_whitespace() {
            let token = token.trim_end_matches(',');
            let hex = token
                .strip_prefix("0x")
                .or_else(|| token.strip_prefix("0X"))
                .unwrap_or(token);
            let val = u32::from_str_radix(hex, 16).unwrap_or_else(|_| {
                panic!("RiscVOracleCase::asm: cannot parse '{token}' as hex u32")
            });
            out.extend_from_slice(&val.to_le_bytes());
        }
    }
    out
}

// ── RV32 encoders ──────────────────────────────────────────────────────────────
//
// These produce 32-bit RV32 instruction words.  They are *not* exhaustive —
// each oracle test only needs a handful of opcodes — and they trade brevity
// for verifiability against the spec.  The encodings are taken from
// RISC-V Unprivileged ISA v2.2 §2.4 (Integer Computational Instructions),
// §2.5 (Control Transfer), and §2.6 (Load and Store).

/// R-type: `funct7 rs2 rs1 funct3 rd opcode`
pub fn enc_r(opcode: u32, funct3: u32, funct7: u32, rd: u32, rs1: u32, rs2: u32) -> u32 {
    (funct7 << 25) | (rs2 << 20) | (rs1 << 15) | (funct3 << 12) | (rd << 7) | opcode
}

/// I-type: `imm[11:0] rs1 funct3 rd opcode`
pub fn enc_i(opcode: u32, funct3: u32, rd: u32, rs1: u32, imm: i32) -> u32 {
    let imm12 = (imm as u32) & 0xFFF;
    (imm12 << 20) | (rs1 << 15) | (funct3 << 12) | (rd << 7) | opcode
}

/// S-type: `imm[11:5] rs2 rs1 funct3 imm[4:0] opcode`
pub fn enc_s(opcode: u32, funct3: u32, rs1: u32, rs2: u32, imm: i32) -> u32 {
    let imm12 = (imm as u32) & 0xFFF;
    let imm_hi = (imm12 >> 5) & 0x7F;
    let imm_lo = imm12 & 0x1F;
    (imm_hi << 25) | (rs2 << 20) | (rs1 << 15) | (funct3 << 12) | (imm_lo << 7) | opcode
}

/// B-type: `imm[12,10:5] rs2 rs1 funct3 imm[4:1,11] opcode`
pub fn enc_b(opcode: u32, funct3: u32, rs1: u32, rs2: u32, offset: i32) -> u32 {
    let imm = (offset as u32) & 0x1FFE; // bit 0 always 0
    let b12 = (imm >> 12) & 0x1;
    let b11 = (imm >> 11) & 0x1;
    let b10_5 = (imm >> 5) & 0x3F;
    let b4_1 = (imm >> 1) & 0xF;
    (b12 << 31)
        | (b10_5 << 25)
        | (rs2 << 20)
        | (rs1 << 15)
        | (funct3 << 12)
        | (b4_1 << 8)
        | (b11 << 7)
        | opcode
}

/// U-type: `imm[31:12] rd opcode`.  `imm` is the full 32-bit value; the
/// low 12 bits are dropped.
pub fn enc_u(opcode: u32, rd: u32, imm: u32) -> u32 {
    (imm & 0xFFFF_F000) | (rd << 7) | opcode
}

/// J-type: `imm[20,10:1,11,19:12] rd opcode`
pub fn enc_j(opcode: u32, rd: u32, offset: i32) -> u32 {
    let imm = (offset as u32) & 0x1F_FFFE; // bit 0 always 0
    let b20 = (imm >> 20) & 0x1;
    let b19_12 = (imm >> 12) & 0xFF;
    let b11 = (imm >> 11) & 0x1;
    let b10_1 = (imm >> 1) & 0x3FF;
    (b20 << 31) | (b10_1 << 21) | (b11 << 20) | (b19_12 << 12) | (rd << 7) | opcode
}

// Common RV32I/M opcodes.
pub const OP_LUI: u32 = 0x37;
pub const OP_AUIPC: u32 = 0x17;
pub const OP_JAL: u32 = 0x6F;
pub const OP_JALR: u32 = 0x67;
pub const OP_BRANCH: u32 = 0x63;
pub const OP_LOAD: u32 = 0x03;
pub const OP_STORE: u32 = 0x23;
pub const OP_OPIMM: u32 = 0x13;
pub const OP_OP: u32 = 0x33;

// Common mnemonic helpers — by no means exhaustive, just enough for the
// initial RV32IMC oracle bank.
pub fn addi(rd: u32, rs1: u32, imm: i32) -> u32 {
    enc_i(OP_OPIMM, 0, rd, rs1, imm)
}
pub fn ori(rd: u32, rs1: u32, imm: i32) -> u32 {
    enc_i(OP_OPIMM, 6, rd, rs1, imm)
}
pub fn xori(rd: u32, rs1: u32, imm: i32) -> u32 {
    enc_i(OP_OPIMM, 4, rd, rs1, imm)
}
pub fn andi(rd: u32, rs1: u32, imm: i32) -> u32 {
    enc_i(OP_OPIMM, 7, rd, rs1, imm)
}
pub fn slli(rd: u32, rs1: u32, shamt: u32) -> u32 {
    enc_i(OP_OPIMM, 1, rd, rs1, (shamt & 0x1F) as i32)
}
pub fn srli(rd: u32, rs1: u32, shamt: u32) -> u32 {
    enc_i(OP_OPIMM, 5, rd, rs1, (shamt & 0x1F) as i32)
}
pub fn srai(rd: u32, rs1: u32, shamt: u32) -> u32 {
    enc_i(OP_OPIMM, 5, rd, rs1, ((shamt & 0x1F) | 0x400) as i32)
}
pub fn add(rd: u32, rs1: u32, rs2: u32) -> u32 {
    enc_r(OP_OP, 0, 0, rd, rs1, rs2)
}
pub fn sub(rd: u32, rs1: u32, rs2: u32) -> u32 {
    enc_r(OP_OP, 0, 0x20, rd, rs1, rs2)
}
pub fn and(rd: u32, rs1: u32, rs2: u32) -> u32 {
    enc_r(OP_OP, 7, 0, rd, rs1, rs2)
}
pub fn or(rd: u32, rs1: u32, rs2: u32) -> u32 {
    enc_r(OP_OP, 6, 0, rd, rs1, rs2)
}
pub fn xor(rd: u32, rs1: u32, rs2: u32) -> u32 {
    enc_r(OP_OP, 4, 0, rd, rs1, rs2)
}
pub fn sll(rd: u32, rs1: u32, rs2: u32) -> u32 {
    enc_r(OP_OP, 1, 0, rd, rs1, rs2)
}
pub fn srl(rd: u32, rs1: u32, rs2: u32) -> u32 {
    enc_r(OP_OP, 5, 0, rd, rs1, rs2)
}
pub fn sra(rd: u32, rs1: u32, rs2: u32) -> u32 {
    enc_r(OP_OP, 5, 0x20, rd, rs1, rs2)
}
pub fn mul(rd: u32, rs1: u32, rs2: u32) -> u32 {
    enc_r(OP_OP, 0, 0x01, rd, rs1, rs2)
}
pub fn div(rd: u32, rs1: u32, rs2: u32) -> u32 {
    enc_r(OP_OP, 4, 0x01, rd, rs1, rs2)
}
pub fn rem(rd: u32, rs1: u32, rs2: u32) -> u32 {
    enc_r(OP_OP, 6, 0x01, rd, rs1, rs2)
}
pub fn lui(rd: u32, imm: u32) -> u32 {
    enc_u(OP_LUI, rd, imm)
}
pub fn lw(rd: u32, rs1: u32, offset: i32) -> u32 {
    enc_i(OP_LOAD, 2, rd, rs1, offset)
}
pub fn lb(rd: u32, rs1: u32, offset: i32) -> u32 {
    enc_i(OP_LOAD, 0, rd, rs1, offset)
}
pub fn lbu(rd: u32, rs1: u32, offset: i32) -> u32 {
    enc_i(OP_LOAD, 4, rd, rs1, offset)
}
pub fn sw(rs2: u32, rs1: u32, offset: i32) -> u32 {
    enc_s(OP_STORE, 2, rs1, rs2, offset)
}
pub fn beq(rs1: u32, rs2: u32, offset: i32) -> u32 {
    enc_b(OP_BRANCH, 0, rs1, rs2, offset)
}
pub fn bne(rs1: u32, rs2: u32, offset: i32) -> u32 {
    enc_b(OP_BRANCH, 1, rs1, rs2, offset)
}
pub fn jal(rd: u32, offset: i32) -> u32 {
    enc_j(OP_JAL, rd, offset)
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

/// Execute `case.program` in the simulator, apply `case.setup`, step until
/// the J-self terminator pins the PC, and return the end `RiscVOracleState`
/// with all 32 GPRs captured.
fn capture_sim_state(case: &RiscVOracleCase) -> RiscVOracleState {
    use labwired_core::bus::SystemBus;
    use labwired_core::cpu::riscv::RiscV;
    use labwired_core::Bus;
    use labwired_core::Cpu;
    use ram_peripheral::RamPeripheral;

    // Use empty() rather than new() — new() seeds STM32 defaults *and*
    // enables the Cortex-M bit-band alias (0x4200_0000-0x43FF_FFFF), which
    // would intercept reads at PROG_BASE and translate them to the bit-band
    // physical region. Empty bus + only the oracle peripherals = clean
    // RISC-V address space.
    let mut bus = SystemBus::empty();
    let entry_pc: u32;

    match &case.program {
        RiscVProgram::Asm(bytes) => {
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
        RiscVProgram::Elf(path) => {
            use goblin::elf::program_header::PT_LOAD;
            use goblin::elf::Elf;

            let elf_bytes = std::fs::read(path)
                .unwrap_or_else(|e| panic!("riscv oracle: failed to read ELF {path:?}: {e}"));
            let elf = Elf::parse(&elf_bytes)
                .unwrap_or_else(|e| panic!("riscv oracle: failed to parse ELF {path:?}: {e}"));

            entry_pc = elf.entry as u32;

            // Single PROG_BASE-anchored peripheral large enough for the ELF.
            let mut prog = RamPeripheral::new(ORACLE_MEM_SIZE);
            for ph in &elf.program_headers {
                if ph.p_type != PT_LOAD || ph.p_filesz == 0 {
                    continue;
                }
                let vaddr = ph.p_vaddr as u32;
                let offset_in_prog = vaddr.checked_sub(PROG_BASE).unwrap_or_else(|| {
                    panic!(
                        "riscv oracle: ELF VAddr 0x{vaddr:08X} is below PROG_BASE 0x{PROG_BASE:08X}"
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

    let mut cpu = RiscV::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(entry_pc);

    // Apply setup.
    let mut init_state = RiscVOracleState::default();
    (case.setup)(&mut init_state);
    for (name, &val) in &init_state.regs {
        if let Some(idx) = parse_x_name(name) {
            // x0 is hardwired to 0 — silently ignore writes.
            if idx != 0 {
                cpu.x[idx as usize] = val;
            }
        }
    }
    for (&addr, &val) in &init_state.mem {
        bus.write_u32(addr as u64, val).unwrap_or_else(|e| {
            panic!("riscv oracle: setup write_u32(0x{addr:08X}) failed: {e:?}")
        });
    }

    // Step until PC settles on the J-self terminator (or limit).
    let sim_config = labwired_core::SimulationConfig::default();
    let mut last_pc = cpu.pc;
    let mut stable_count: u32 = 0;
    for _ in 0..MAX_STEPS {
        cpu.step(&mut bus, &[], &sim_config)
            .unwrap_or_else(|e| panic!("riscv oracle sim error at pc=0x{:08X}: {e:?}", cpu.pc));
        if cpu.pc == last_pc {
            stable_count += 1;
            // A J-self pins the PC indefinitely; once we see two consecutive
            // steps at the same PC the program is done.
            if stable_count >= 2 {
                break;
            }
        } else {
            stable_count = 0;
            last_pc = cpu.pc;
        }
    }

    // Build end state.
    let mut end = RiscVOracleState::default();
    for i in 0u8..32 {
        end.regs.insert(format!("x{i}"), cpu.x[i as usize]);
    }
    end.pc = cpu.pc;
    let mut addrs: Vec<u32> = init_state.mem.keys().copied().collect();
    addrs.extend_from_slice(&case.mem_capture_addrs);
    addrs.sort_unstable();
    addrs.dedup();
    for addr in addrs {
        let val = bus
            .read_u32(addr as u64)
            .unwrap_or_else(|e| panic!("riscv oracle: end read_u32(0x{addr:08X}) failed: {e:?}"));
        end.mem.insert(addr, val);
    }
    end
}

/// Execute `case` in the software simulator and run its expect closure.
pub fn run_sim(case: RiscVOracleCase) {
    let end_state = capture_sim_state(&case);
    (case.expect)(&end_state);
}

/// Execute `case` against a physical ESP32-C3 board via USB-JTAG / OpenOCD.
///
/// Gated behind the `hw-oracle-c3` feature; not implemented in this slice
/// — the function exists so the macro can reference it.  Wiring it up
/// requires:
/// * OpenOCD config: `interface/esp_usb_jtag.cfg` + `target/esp32c3.cfg`.
/// * A C3 board attached over USB-JTAG.
/// * Memory-write/read primitives via the existing `openocd` module
///   (which is currently S3-specific in its `spawn_default` helper but
///   the `OpenOcd::spawn_with_args` constructor takes arbitrary args).
#[cfg(feature = "hw-oracle-c3")]
pub fn run_hw(_case: RiscVOracleCase) {
    unimplemented!(
        "riscv oracle hw runner: ESP32-C3 USB-JTAG support pending. \
         OpenOcd::spawn_with_args(&[\"-f\", \"interface/esp_usb_jtag.cfg\", \
         \"-f\", \"target/esp32c3.cfg\"]) is the entry point; see the S3 \
         capture_hw_state in lib.rs for the pattern to mirror."
    );
}

#[cfg(feature = "hw-oracle-c3")]
pub fn run_diff(case: RiscVOracleCase) {
    let sim_end = capture_sim_state(&case);
    run_hw(case);
    // Once run_hw is implemented, capture the HW end state and diff against
    // sim_end here.
    let _ = sim_end;
}

#[cfg(test)]
mod encoder_tests {
    use super::*;

    // Encodings cross-checked against the RISC-V Unprivileged ISA v2.2 spec.

    #[test]
    fn addi_x10_x0_5_encoding() {
        // ADDI x10, x0, 5 — opcode 0x13, funct3=0, rd=10, rs1=0, imm=5
        // 0x00500513
        assert_eq!(addi(10, 0, 5), 0x0050_0513);
    }

    #[test]
    fn add_x12_x10_x11_encoding() {
        // ADD x12, x10, x11 — opcode 0x33, funct3=0, funct7=0, rd=12, rs1=10, rs2=11
        // 0x00B50633
        assert_eq!(add(12, 10, 11), 0x00B5_0633);
    }

    #[test]
    fn sub_x12_x10_x11_encoding() {
        // SUB x12, x10, x11 — funct7=0x20
        // 0x40B50633
        assert_eq!(sub(12, 10, 11), 0x40B5_0633);
    }

    #[test]
    fn lui_x5_0xdeadb000_encoding() {
        // LUI x5, 0xDEADB — opcode 0x37, rd=5, imm high 20 bits = 0xDEADB
        // 0xDEADB2B7
        assert_eq!(lui(5, 0xDEAD_B000), 0xDEAD_B2B7);
    }

    #[test]
    fn jal_x0_zero_is_jself_sentinel() {
        // J . (JAL x0, 0) — opcode 0x6F, rd=0, offset=0 → 0x0000_006F.
        // The terminator constant in this module must match.
        assert_eq!(jal(0, 0), J_SELF);
    }
}
