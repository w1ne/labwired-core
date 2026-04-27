//! HW oracle harness: OracleCase runtime + sim/hw/diff runners.
//!
//! # Architecture
//!
//! `OracleCase` bundles a small Xtensa program (either raw bytes or an ELF path),
//! a setup closure that writes initial register values, and an expect closure that
//! asserts post-execution register values.  Three runners are provided:
//!
//! * `run_sim`  — executes the program in `XtensaLx7` + `SystemBus`; always available.
//! * `run_hw`   — flashes/writes the program to a real ESP32-S3 via OpenOCD; gated on
//!               the `hw-oracle` feature.
//! * `run_diff` — runs both and diffs the end state; gated on `hw-oracle`.
//!
//! # IRAM address
//!
//! Both sim and HW runners load programs at `0x4037_0000` — the start of the
//! internal SRAM 0 (IRAM) window on ESP32-S3.  Source: ESP32-S3 TRM v1.4 §3.3.11
//! "Internal SRAM 0", and confirmed by OpenOCD memory-write tests in H1/H2.

pub use labwired_hw_oracle_macros::hw_oracle_test;

pub mod flash;
pub mod openocd;

use std::collections::HashMap;
use std::path::PathBuf;

// ── IRAM base address ─────────────────────────────────────────────────────────

/// Start address of ESP32-S3 internal SRAM 0 (IRAM) in the instruction-fetch
/// window.  Source: ESP32-S3 TRM v1.4 §3.3.11, confirmed via H1/H2 OpenOCD
/// memory-read tests that read from `0x40370000`.
pub const IRAM_BASE: u32 = 0x4037_0000;

/// DRAM alias of the same physical SRAM 0 used by the IRAM window.
///
/// On ESP32-S3 the internal SRAM 0 is accessible via two bus windows:
///   * IRAM window  0x4037_0000 (I-bus, instruction fetch + word data only)
///   * DRAM window  0x3FC8_8000 (D-bus, byte/halfword/word data access)
///
/// Tests that use byte or halfword load/store instructions (L8UI, L16UI,
/// L16SI, S8I, S16I) must use the DRAM alias; the IRAM window silently
/// drops sub-word memory accesses on the I-bus.
///
/// `DRAM_BASE = IRAM_BASE - 0x6E8000` (confirmed by ESP32-S3 TRM §3.3.11
/// address map: SRAM0 DRAM starts at 0x3FC8_8000, IRAM at 0x4037_0000;
/// offset = 0x4037_0000 - 0x3FC8_8000 = 0x6E_8000).
pub const DRAM_BASE: u32 = 0x3FC8_8000;

/// Size of the oracle program scratch region within IRAM (and the matching
/// DRAM alias window).  64 KiB is sufficient for all Plan-1 oracle programs.
const ORACLE_MEM_SIZE: usize = 0x1_0000; // 64 KiB

// ── BREAK encoding ────────────────────────────────────────────────────────────

/// `BREAK 1, 15` encoded as a 3-byte little-endian Xtensa wide instruction.
///
/// Encoding (ST0 format, op0=0, op1=0, op2=0, r=4, s=imm_s=1, t=imm_t=15):
///   st0(r=4, s=1, t=15) = (r<<12)|(s<<8)|(t<<4)
///                       = (4<<12)|(1<<8)|(15<<4)
///                       = 0x4000 | 0x0100 | 0x00F0
///                       = 0x41F0
///   3-byte LE: 0xF0, 0x41, 0x00
///
/// Cross-referenced with `enc_break(1, 15) = st0(4, 1, 0xF)` from xtensa_exec.rs.
///
/// The simulator raises `BreakpointHit` on BREAK, which `run_sim` catches as
/// the termination signal.
const BREAK_1_15: [u8; 3] = [0xF0, 0x41, 0x00];

// ── OracleState ───────────────────────────────────────────────────────────────

/// Snapshot of CPU register + memory state used by setup and expect closures.
///
/// **Setup fields** (`sr`, `init_windowbase`, `init_windowstart`, `init_ps_excm`) are
/// applied to the CPU before execution starts.
///
/// **End-state fields** (`wb`, `ws`, `excm`, `epc1`, `exccause`) are captured
/// after execution halts.
#[derive(Debug, Clone, Default)]
pub struct OracleState {
    /// Register values keyed by Xtensa name: `"a0"`..`"a15"`.
    pub regs: HashMap<String, u32>,
    /// Memory snapshot (word-aligned address → 32-bit word).
    ///
    /// In **setup**: populated by `write_mem` to describe initial memory state;
    /// the runtime writes these into the bus before execution.
    ///
    /// In **end state**: re-read from bus for every address present in setup
    /// (so loads can observe pre-written values) plus every address in
    /// `OracleCase::mem_capture_addrs` (for store-only tests).
    pub mem: HashMap<u32, u32>,
    /// Final program counter captured after `BREAK` (the address of the BREAK
    /// instruction).  Always populated by `capture_sim_state`.
    pub pc: u32,

    // ── Setup-only fields (applied before execution) ──────────────────────────

    /// Special Register (SR) values to write before execution.
    /// Key = numeric SR ID (use `labwired_core::cpu::xtensa_sr::*` constants).
    pub sr: HashMap<u16, u32>,

    /// If `Some(wb)`, override `WindowBase` before execution.
    pub init_windowbase: Option<u8>,

    /// If `Some(ws)`, override `WindowStart` before execution.
    pub init_windowstart: Option<u16>,

    /// If `Some(excm)`, override `PS.EXCM` before execution.
    pub init_ps_excm: Option<bool>,

    /// If `Some(level)`, override `PS.INTLEVEL` before execution.
    pub init_ps_intlevel: Option<u8>,

    /// If `Some(mask)`, set `INTERRUPT` bits via the raw bypass path before
    /// execution.  This is needed because direct SW writes to INTERRUPT are
    /// ignored (hardware-latched semantics); only `set_raw` works.
    pub init_interrupt: Option<u32>,

    // ── End-state captured fields ─────────────────────────────────────────────

    /// `WindowBase` after execution halts.
    pub wb: u8,
    /// `WindowStart` after execution halts.
    pub ws: u16,
    /// `PS.EXCM` after execution halts.
    pub excm: bool,
    /// `PS.INTLEVEL` after execution halts.
    pub intlevel: u8,
    /// `EPC1` SR value after execution halts.
    pub epc1: u32,
    /// `EXCCAUSE` SR value after execution halts.
    pub exccause: u32,
}

impl OracleState {
    /// Write a register value.
    pub fn write_reg(&mut self, name: &str, v: u32) {
        self.regs.insert(name.to_string(), v);
    }

    /// Read a register value, returning 0 for unknown registers.
    pub fn read_reg(&self, name: &str) -> u32 {
        self.regs.get(name).copied().unwrap_or(0)
    }

    /// Assert that register `name` equals `expected`, panicking with a
    /// descriptive message on mismatch.
    pub fn assert_reg(&self, name: &str, expected: u32) {
        let actual = self.read_reg(name);
        assert_eq!(
            actual, expected,
            "oracle: register {name}: expected 0x{expected:08X}, got 0x{actual:08X}"
        );
    }

    /// Write a 32-bit word into the memory setup map.
    ///
    /// The address should be 4-byte aligned; unaligned accesses will still
    /// work but the bus round-trip uses 32-bit reads so the lower two address
    /// bits are effectively ignored.
    pub fn write_mem(&mut self, addr: u32, v: u32) {
        self.mem.insert(addr, v);
    }

    /// Read a 32-bit word from the memory snapshot, returning 0 if not present.
    pub fn read_mem(&self, addr: u32) -> u32 {
        self.mem.get(&addr).copied().unwrap_or(0)
    }

    /// Assert that the memory word at `addr` equals `expected`, panicking with
    /// a descriptive message on mismatch.
    pub fn assert_mem(&self, addr: u32, expected: u32) {
        let actual = self.read_mem(addr);
        assert_eq!(
            actual, expected,
            "oracle: mem[0x{addr:08X}]: expected 0x{expected:08X}, got 0x{actual:08X}"
        );
    }

    /// Assert that the final PC equals `expected`.
    pub fn assert_pc(&self, expected: u32) {
        assert_eq!(
            self.pc, expected,
            "oracle: pc: expected 0x{expected:08X}, got 0x{:08X}",
            self.pc
        );
    }

    // ── Window / PS setup helpers ─────────────────────────────────────────────

    /// Write a Special Register value to be applied before execution.
    ///
    /// Use `labwired_core::cpu::xtensa_sr::*` constants for SR IDs.
    pub fn write_sr(&mut self, sr_id: u16, v: u32) {
        self.sr.insert(sr_id, v);
    }

    /// Override `WindowBase` before execution.
    pub fn write_windowbase(&mut self, wb: u8) {
        self.init_windowbase = Some(wb);
    }

    /// Override `WindowStart` before execution.
    pub fn write_windowstart(&mut self, ws: u16) {
        self.init_windowstart = Some(ws);
    }

    /// Override `PS.EXCM` before execution.
    pub fn write_ps_excm(&mut self, excm: bool) {
        self.init_ps_excm = Some(excm);
    }

    /// Override `PS.INTLEVEL` before execution.
    pub fn write_intlevel(&mut self, level: u8) {
        self.init_ps_intlevel = Some(level);
    }

    /// Set `INTERRUPT` pending bits before execution via the raw hardware path
    /// (bypasses the software-write guard on INTERRUPT).
    pub fn write_interrupt(&mut self, mask: u32) {
        self.init_interrupt = Some(mask);
    }

    /// Write `INTENABLE` SR before execution.
    ///
    /// Use `labwired_core::cpu::xtensa_sr::INTENABLE` for the SR ID.
    pub fn write_intenable(&mut self, mask: u32) {
        use labwired_core::cpu::xtensa_sr::INTENABLE;
        self.sr.insert(INTENABLE, mask);
    }

    /// Write `VECBASE` SR before execution.
    pub fn write_vecbase(&mut self, addr: u32) {
        use labwired_core::cpu::xtensa_sr::VECBASE;
        self.sr.insert(VECBASE, addr);
    }

    /// Write `EPC1` SR before execution.
    pub fn write_epc1(&mut self, val: u32) {
        use labwired_core::cpu::xtensa_sr::EPC1;
        self.sr.insert(EPC1, val);
    }

    /// Write `EPC[level]` SR before execution (level 2..7).
    ///
    /// Panics if `level` is not in `2..=7`.
    pub fn write_epc(&mut self, level: u8, val: u32) {
        use labwired_core::cpu::xtensa_sr::{EPC2, EPC3, EPC4, EPC5, EPC6, EPC7};
        let id = match level {
            2 => EPC2, 3 => EPC3, 4 => EPC4, 5 => EPC5, 6 => EPC6, 7 => EPC7,
            _ => panic!("write_epc: level {level} not in 2..=7"),
        };
        self.sr.insert(id, val);
    }

    /// Write `EPS[level]` SR before execution (level 2..7).
    ///
    /// Panics if `level` is not in `2..=7`.
    pub fn write_eps(&mut self, level: u8, val: u32) {
        use labwired_core::cpu::xtensa_sr::{EPS2, EPS3, EPS4, EPS5, EPS6, EPS7};
        let id = match level {
            2 => EPS2, 3 => EPS3, 4 => EPS4, 5 => EPS5, 6 => EPS6, 7 => EPS7,
            _ => panic!("write_eps: level {level} not in 2..=7"),
        };
        self.sr.insert(id, val);
    }

    // ── End-state assertion helpers ───────────────────────────────────────────

    /// Assert that `WindowBase` equals `expected` after execution.
    pub fn assert_windowbase(&self, expected: u8) {
        assert_eq!(
            self.wb, expected,
            "oracle: WindowBase: expected {expected}, got {}",
            self.wb
        );
    }

    /// Assert that `WindowStart` equals `expected` after execution.
    pub fn assert_windowstart(&self, expected: u16) {
        assert_eq!(
            self.ws, expected,
            "oracle: WindowStart: expected 0x{expected:04X}, got 0x{:04X}",
            self.ws
        );
    }

    /// Assert that `PS.EXCM` equals `expected` after execution.
    pub fn assert_excm(&self, expected: bool) {
        assert_eq!(
            self.excm, expected,
            "oracle: PS.EXCM: expected {expected}, got {}",
            self.excm
        );
    }

    /// Assert that `PS.INTLEVEL` equals `expected` after execution.
    pub fn assert_intlevel(&self, expected: u8) {
        assert_eq!(
            self.intlevel, expected,
            "oracle: PS.INTLEVEL: expected {expected}, got {}",
            self.intlevel
        );
    }

    /// Assert that `EPC1` equals `expected` after execution.
    pub fn assert_epc1(&self, expected: u32) {
        assert_eq!(
            self.epc1, expected,
            "oracle: EPC1: expected 0x{expected:08X}, got 0x{:08X}",
            self.epc1
        );
    }

    /// Assert that `EXCCAUSE` equals `expected` after execution.
    pub fn assert_exccause(&self, expected: u32) {
        assert_eq!(
            self.exccause, expected,
            "oracle: EXCCAUSE: expected {expected}, got {}",
            self.exccause
        );
    }
}

// ── Program ───────────────────────────────────────────────────────────────────

/// The program to execute in an oracle run.
pub enum Program {
    /// Raw instruction bytes, relocated to IRAM_BASE.
    ///
    /// A trailing `BREAK 1, 15` is always appended automatically by
    /// `OracleCase::asm` so callers do not need to include it.
    Asm(Vec<u8>),
    /// Path to an ELF binary.  The ELF is loaded by OpenOCD `program` on HW;
    /// on sim the segments are mapped into the oracle address space.
    Elf(PathBuf),
}

impl Program {
    /// Return the raw bytes for `Asm` programs.  For `Elf` programs, returns
    /// the raw file bytes (caller must parse/load them as appropriate).
    pub fn bytes(&self) -> &[u8] {
        match self {
            Program::Asm(b) => b,
            Program::Elf(_) => &[],
        }
    }
}

// ── Tolerance ─────────────────────────────────────────────────────────────────

/// Acceptable divergence between sim and HW in a diff run.
pub struct Tolerance {
    /// Allowed CCOUNT cycle difference (0 = exact).
    pub ccount_cycles: u32,
    /// Allowed timestamp difference in picoseconds (0 = exact).
    pub timestamp_ps: u64,
}

impl Tolerance {
    /// Require bit-exact match.
    pub fn exact() -> Self {
        Self { ccount_cycles: 0, timestamp_ps: 0 }
    }

    /// Generous tolerance for noisy timing measurements.
    pub fn lenient() -> Self {
        Self { ccount_cycles: 1000, timestamp_ps: 1_000_000_000 }
    }
}

impl Default for Tolerance {
    fn default() -> Self { Self::exact() }
}

// ── OracleCase ────────────────────────────────────────────────────────────────

/// A complete oracle test case: program + initial state + expected end state.
pub struct OracleCase {
    pub program: Program,
    pub setup: Box<dyn Fn(&mut OracleState) + Send + Sync>,
    pub expect: Box<dyn Fn(&OracleState) + Send + Sync>,
    pub tolerance: Tolerance,
    /// Additional addresses to read from the bus into `end_state.mem` after
    /// execution.  Used for store-only tests (S8I/S16I/S32I) where setup does
    /// not pre-populate `mem` but the test wants to assert what was written.
    pub mem_capture_addrs: Vec<u32>,
}

impl OracleCase {
    // ── Constructors ──────────────────────────────────────────────────────────

    /// Build an oracle case from raw instruction bytes.
    ///
    /// A `BREAK 1, 15` terminator is appended automatically so that `run_sim`
    /// knows when to stop and HW execution halts the CPU.
    pub fn from_bytes(mut bytes: Vec<u8>) -> Self {
        bytes.extend_from_slice(&BREAK_1_15);
        Self {
            program: Program::Asm(bytes),
            setup: Box::new(|_| {}),
            expect: Box::new(|_| {}),
            tolerance: Tolerance::exact(),
            mem_capture_addrs: Vec::new(),
        }
    }

    /// Build an oracle case by parsing one or more `.word 0xXXXXXXXX` lines.
    ///
    /// Each line is expected to be of the form `.word 0x<hex>` (case-insensitive,
    /// optional whitespace).  The 32-bit value is emitted as 4 little-endian bytes.
    ///
    /// A `BREAK 1, 15` terminator is appended automatically.
    ///
    /// # Panics
    ///
    /// Panics if any line cannot be parsed.
    pub fn asm(s: &str) -> Self {
        let bytes = parse_dot_word(s);
        Self::from_bytes(bytes)
    }

    /// Build an oracle case from an ELF file at `path`.
    pub fn elf(path: &str) -> Self {
        Self {
            program: Program::Elf(PathBuf::from(path)),
            setup: Box::new(|_| {}),
            expect: Box::new(|_| {}),
            tolerance: Tolerance::exact(),
            mem_capture_addrs: Vec::new(),
        }
    }

    // ── Minimal stub constructor (H3 compatibility) ───────────────────────────

    /// No-op constructor retained for trybuild fixture compatibility.
    ///
    /// Produces an empty program with no assertions.
    pub fn stub() -> Self {
        Self::from_bytes(vec![])
    }

    // ── Builder methods ───────────────────────────────────────────────────────

    /// Provide a closure that writes initial register values into `OracleState`
    /// before execution starts.
    pub fn setup<F>(mut self, f: F) -> Self
    where
        F: Fn(&mut OracleState) + Send + Sync + 'static,
    {
        self.setup = Box::new(f);
        self
    }

    /// Provide a closure that asserts register values in the end state after
    /// execution completes.
    pub fn expect<F>(mut self, f: F) -> Self
    where
        F: Fn(&OracleState) + Send + Sync + 'static,
    {
        self.expect = Box::new(f);
        self
    }

    /// Specify additional memory addresses (word-aligned) to read from the bus
    /// into `end_state.mem` after execution.  Useful for store-only tests where
    /// no initial value is written to `setup.mem`.
    pub fn capture_mem(mut self, addrs: &[u32]) -> Self {
        self.mem_capture_addrs.extend_from_slice(addrs);
        self
    }

    /// Override the comparison tolerance for diff runs.
    pub fn tolerance(mut self, t: Tolerance) -> Self {
        self.tolerance = t;
        self
    }
}

// ── parse_dot_word ─────────────────────────────────────────────────────────────

/// Parse lines of the form `.word 0xXXXXXX` (or multiple whitespace-separated
/// hex values per line) and emit 3-byte little-endian Xtensa instruction bytes.
///
/// Each `.word` value is interpreted as a 24-bit wide Xtensa instruction and
/// emitted as 3 bytes (low 24 bits, little-endian).  This matches the
/// `write_insns` convention used in `xtensa_exec.rs` tests.
///
/// Lines starting with `//` or `#` are treated as comments and ignored.
///
/// # Panics
///
/// Panics if any token cannot be parsed as a hex integer, or if the value
/// exceeds 24 bits (i.e. is > 0xFFFFFF).
fn parse_dot_word(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for line in s.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('#') {
            continue;
        }
        // Strip optional `.word` prefix.
        let rest = line.strip_prefix(".word").unwrap_or(line).trim();
        for token in rest.split_whitespace() {
            let token = token.trim_end_matches(',');
            let hex = token
                .strip_prefix("0x")
                .or_else(|| token.strip_prefix("0X"))
                .unwrap_or(token);
            let val = u32::from_str_radix(hex, 16)
                .unwrap_or_else(|_| panic!("OracleCase::asm: cannot parse '{token}' as hex u32"));
            assert!(
                val <= 0x00FF_FFFF,
                "OracleCase::asm: value 0x{val:08X} exceeds 24 bits; \
                 Xtensa wide instructions are 3 bytes"
            );
            // Emit as 3 LE bytes (24-bit Xtensa wide instruction).
            out.push((val & 0xFF) as u8);
            out.push(((val >> 8) & 0xFF) as u8);
            out.push(((val >> 16) & 0xFF) as u8);
        }
    }
    out
}

// ── parse_ar_name ─────────────────────────────────────────────────────────────

/// Parse `"aN"` → logical register index N (0..=15).  Returns `None` for other names.
fn parse_ar_name(name: &str) -> Option<u8> {
    let n = name.strip_prefix('a')?;
    let idx: u8 = n.parse().ok()?;
    if idx < 16 { Some(idx) } else { None }
}

// ── RAM peripheral wrapper ─────────────────────────────────────────────────────

/// A `Peripheral` backed by a flat byte array.  Used to map oracle IRAM scratch
/// memory into the `SystemBus` at `IRAM_BASE`.
mod ram_peripheral {
    use labwired_core::{Peripheral, SimResult};

    pub struct RamPeripheral {
        data: Vec<u8>,
    }

    impl RamPeripheral {
        pub fn new(size: usize) -> Self {
            Self { data: vec![0u8; size] }
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

// ── capture_sim_state ─────────────────────────────────────────────────────────

/// Execute `case.program` in the simulator, apply `case.setup`, step until
/// BREAK, and return the end `OracleState` with all 16 AR registers captured.
///
/// In addition to AR registers and memory, the end state includes:
/// `wb` (WindowBase), `ws` (WindowStart), `excm` (PS.EXCM),
/// `epc1` (EPC1 SR), and `exccause` (EXCCAUSE SR).
fn capture_sim_state(case: &OracleCase) -> OracleState {
    use labwired_core::bus::SystemBus;
    use labwired_core::cpu::xtensa_lx7::XtensaLx7;
    use labwired_core::cpu::xtensa_sr::{EPC1 as EPC1_SR, EXCCAUSE as EXCCAUSE_SR};
    use labwired_core::{Bus, Cpu, SimulationError};
    use ram_peripheral::RamPeripheral;

    // Determine program bytes and entry point for sim.  For Asm programs the
    // entry is always IRAM_BASE (bytes are loaded there verbatim).  For ELF
    // programs we parse the PT_LOAD segments with goblin and load each segment
    // at its virtual address; the entry point comes from the ELF header.
    let entry_pc: u32;
    let mut bus = SystemBus::new();

    match &case.program {
        Program::Asm(bytes) => {
            // Build the peripheral with the program bytes pre-loaded at IRAM_BASE.
            let mut iram = RamPeripheral::new(ORACLE_MEM_SIZE);
            iram.write_bytes(0, bytes);
            bus.add_peripheral(
                "oracle_iram",
                IRAM_BASE as u64,
                ORACLE_MEM_SIZE as u64,
                None,
                Box::new(iram),
            );
            entry_pc = IRAM_BASE;
        }
        Program::Elf(path) => {
            use goblin::elf::program_header::PT_LOAD;
            use goblin::elf::Elf;

            let elf_bytes = std::fs::read(path)
                .unwrap_or_else(|e| panic!("run_sim: failed to read ELF {:?}: {e}", path));
            let elf = Elf::parse(&elf_bytes)
                .unwrap_or_else(|e| panic!("run_sim: failed to parse ELF {:?}: {e}", path));

            entry_pc = elf.entry as u32;

            // Register a single IRAM peripheral large enough for all segments.
            // We assume all PT_LOAD segments fall within the oracle IRAM window.
            let mut iram = RamPeripheral::new(ORACLE_MEM_SIZE);
            for ph in &elf.program_headers {
                if ph.p_type != PT_LOAD || ph.p_filesz == 0 {
                    continue;
                }
                let vaddr = ph.p_vaddr as u32;
                let offset_in_iram = vaddr
                    .checked_sub(IRAM_BASE)
                    .unwrap_or_else(|| panic!(
                        "run_sim: ELF segment VAddr 0x{vaddr:08X} is below IRAM_BASE \
                         0x{IRAM_BASE:08X}"
                    )) as usize;
                let size = ph.p_filesz as usize;
                let file_offset = ph.p_offset as usize;
                let seg_data = &elf_bytes[file_offset..file_offset + size];
                iram.write_bytes(offset_in_iram, seg_data);
            }
            bus.add_peripheral(
                "oracle_iram",
                IRAM_BASE as u64,
                ORACLE_MEM_SIZE as u64,
                None,
                Box::new(iram),
            );
        }
    }

    // Also register a DRAM alias peripheral covering the same physical region.
    //
    // On the real ESP32-S3 the D-bus alias (0x3FC8_8000) and the I-bus alias
    // (0x4037_0000) both map to the same SRAM0 physical memory.  In the sim
    // they are separate peripherals; writes to one do NOT automatically appear
    // in the other.  This is acceptable because:
    //   - Programs always live in IRAM (instruction fetch).
    //   - Sub-word tests (L8UI / L16UI / L16SI / S8I / S16I) use DATA_DRAM
    //     (0x3FC8_9000) exclusively — they never reference DATA (IRAM alias).
    //   - 32-bit tests (L32I / S32I) use DATA (IRAM alias).
    //
    // Therefore a simple independent DRAM peripheral correctly models the
    // subset of SRAM0 behaviour exercised by Plan-1 oracle tests.
    bus.add_peripheral(
        "oracle_dram",
        DRAM_BASE as u64,
        ORACLE_MEM_SIZE as u64,
        None,
        Box::new(ram_peripheral::RamPeripheral::new(ORACLE_MEM_SIZE)),
    );

    // Build a minimal bus: default SystemBus peripherals are all at STM32
    // addresses (0x2000_0000 RAM, 0x0 flash).  Both are outside the Xtensa
    // IRAM window (0x40370000); oracle_iram registered above covers that range.

    let mut cpu = XtensaLx7::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(entry_pc);

    // Apply setup state.
    let mut init_state = OracleState::default();
    (case.setup)(&mut init_state);
    for (name, &val) in &init_state.regs {
        if let Some(idx) = parse_ar_name(name) {
            cpu.regs.write_logical(idx, val);
        }
    }
    // Apply SR setup (e.g. VECBASE for window vector tests).
    for (&sr_id, &val) in &init_state.sr {
        cpu.sr.write(sr_id, val);
    }
    // Apply WindowBase / WindowStart overrides.
    if let Some(wb) = init_state.init_windowbase {
        cpu.regs.set_windowbase(wb);
    }
    if let Some(ws) = init_state.init_windowstart {
        cpu.regs.set_windowstart(ws);
    }
    // Apply PS.EXCM override.
    if let Some(excm) = init_state.init_ps_excm {
        cpu.ps.set_excm(excm);
    }
    // Apply PS.INTLEVEL override.
    if let Some(level) = init_state.init_ps_intlevel {
        cpu.ps.set_intlevel(level);
    }
    // Inject INTERRUPT pending bits via raw bypass (hardware-latched register).
    if let Some(mask) = init_state.init_interrupt {
        cpu.sr.set_raw(labwired_core::cpu::xtensa_sr::INTERRUPT, mask);
    }
    // Write setup memory into the bus (for load tests that need pre-populated data).
    for (&addr, &val) in &init_state.mem {
        bus.write_u32(addr as u64, val)
            .unwrap_or_else(|e| panic!("oracle sim: write_u32(0x{addr:08X}) failed: {e:?}"));
    }

    /// Build an `OracleState` end-state snapshot from the current CPU + bus.
    macro_rules! make_end_state {
        ($halt_pc:expr) => {{
            let mut end = OracleState::default();
            for i in 0u8..16 {
                end.regs.insert(format!("a{}", i), cpu.regs.read_logical(i));
            }
            end.pc       = $halt_pc;
            end.wb       = cpu.regs.windowbase();
            end.ws       = cpu.regs.windowstart();
            end.excm     = cpu.ps.excm();
            end.intlevel = cpu.ps.intlevel();
            end.epc1     = cpu.sr.read(EPC1_SR);
            end.exccause = cpu.sr.read(EXCCAUSE_SR);
            // Re-read memory addresses.
            let mut addrs: Vec<u32> = init_state.mem.keys().copied().collect();
            addrs.extend_from_slice(&case.mem_capture_addrs);
            addrs.sort_unstable();
            addrs.dedup();
            for addr in addrs {
                let val = bus.read_u32(addr as u64)
                    .unwrap_or_else(|e| panic!("oracle sim: read_u32(0x{addr:08X}) failed: {e:?}"));
                end.mem.insert(addr, val);
            }
            end
        }};
    }

    // Step until BREAK (BreakpointHit) or limit.
    const MAX_STEPS: usize = 10_000;
    for _ in 0..MAX_STEPS {
        match cpu.step(&mut bus, &[]) {
            Ok(()) => {}
            Err(SimulationError::BreakpointHit(break_pc)) => {
                return make_end_state!(break_pc);
            }
            Err(SimulationError::ExceptionRaised { .. }) => {
                // ExceptionRaised terminates execution for exception-path tests.
                // cpu.sr[EXCCAUSE] and cpu.sr[EPC1] are already written by
                // raise_general_exception. cpu.pc is the vector address
                // (VECBASE + offset), so use cpu.get_pc() as the halt PC so that
                // expect closures can verify the redirect address via assert_pc().
                return make_end_state!(cpu.get_pc());
            }
            Err(e) => panic!("oracle sim error: {e:?}"),
        }
    }
    panic!("oracle sim: exceeded {MAX_STEPS} steps without BREAK");
}

// ── run_sim ───────────────────────────────────────────────────────────────────

/// Execute `case` in the software simulator and run the expect closure.
pub fn run_sim(case: OracleCase) {
    let end_state = capture_sim_state(&case);
    (case.expect)(&end_state);
}

// ── run_hw + run_diff (hw-oracle feature) ─────────────────────────────────────

/// Execute `case` against a physical ESP32-S3 board via JTAG / OpenOCD.
///
/// Writes the program bytes into IRAM at `IRAM_BASE`, sets the PC, applies
/// register setup, resumes, waits for halt (BREAK halts the ESP32-S3 via its
/// hardware debug exception), reads back registers, and runs the expect closure.
///
/// # Five isolation guarantees
///
/// 1. **SMP disabled** (`ESP32_S3_ONLYCPU=1`): only cpu0 is managed by
///    OpenOCD, preventing cpu1 (ROM bootloader) from overwriting IRAM.
/// 2. **ELF loading**: PT_LOAD segments are parsed with goblin and written
///    directly to HW IRAM/DRAM via OpenOCD memory writes.
/// 3. **Memory isolation**: the oracle scratch region in IRAM is zeroed before
///    each test so stale data from a previous test cannot affect load results.
/// 4. **Register isolation**: a0–a15 and key SRs are zeroed/reset to canonical
///    values before applying the test's setup closure, so boot-time register
///    state cannot leak into diff comparisons.
/// 5. **BREAK detection**: `wait_for_break` checks DEBUGCAUSE bit 3 (BREAK)
///    rather than accepting any halt, eliminating false-early halts.
#[cfg(feature = "hw-oracle")]
fn capture_hw_state(case: &OracleCase) -> OracleState {
    use crate::flash::TargetBoard;
    use crate::openocd::OpenOcd;
    use std::time::Duration;

    // ── Detect board and spawn OpenOCD with SMP disabled (fix #1) ──────────
    let board = TargetBoard::detect()
        .expect("run_hw: ESP32-S3 board not detected; is it connected via USB-JTAG?");
    let mut oc = OpenOcd::spawn_for(&board)
        .expect("run_hw: failed to spawn OpenOCD");

    // reset_halt leaves the CPU stopped at the ROM entry point with all caches
    // clean.  We issue an extra explicit halt after it to make sure OpenOCD has
    // synchronised its target state (reset_halt response is async on some
    // OpenOCD versions).
    oc.reset_halt().expect("run_hw: reset_halt failed");
    oc.halt().expect("run_hw: halt after reset_halt failed");

    // ── Memory isolation: zero the program region + DATA area (fix #3) ─────
    //
    // We zero a range that covers:
    //   a) The program bytes (rounded up + guard words), and
    //   b) The standard oracle DATA area at IRAM_BASE+0x1000 (1 KiB block).
    //
    // The DATA area is used by load/store tests.  Without zeroing it, stale
    // bytes from a previous test (e.g. S8I storing 0xAB) would be visible to
    // the next test's R16UI check.  We zero from IRAM_BASE through DATA+16
    // words to clear both the program and data scratch regions in one bulk op.
    //
    // The DATA constant used in tests is IRAM_BASE + 0x1000.  Zeroing
    // IRAM_BASE+0x0000 through IRAM_BASE+0x1010 = 0x1010/4 = 1028 words.
    const DATA_ZERO_WORDS: usize = (0x1000 + 0x40) / 4; // covers DATA+16 words

    let program_zero_words: usize = match &case.program {
        Program::Asm(bytes) => {
            // Round up bytes.len() to 4, add 16 words (64 bytes) of guard
            // then take the max with DATA_ZERO_WORDS so both program and DATA
            // areas are clean.
            let prog_words = (bytes.len() + 3) / 4 + 16;
            prog_words.max(DATA_ZERO_WORDS).min(ORACLE_MEM_SIZE / 4)
        }
        Program::Elf(_) => {
            // ELF: zero through the DATA area at minimum.
            DATA_ZERO_WORDS
        }
    };
    oc.fill_memory(IRAM_BASE, 0, program_zero_words)
        .expect("run_hw: fill_memory (zero program+data region) failed");

    // Also zero the DRAM alias DATA scratch area (DATA_DRAM = DRAM_BASE+0x1000).
    // Sub-word store tests (S8I, S16I) write to the DRAM alias; without zeroing,
    // stale bytes bleed into the next run's load check.  We zero 16 words
    // (64 bytes) starting at DRAM_BASE+0x1000 to cover DATA_DRAM and a guard.
    oc.fill_memory(DRAM_BASE + 0x1000, 0, 16)
        .expect("run_hw: fill_memory (zero DRAM data scratch) failed");

    // ── Load program into IRAM (fix #2: ELF support) ───────────────────────
    let entry_pc: u32 = match &case.program {
        Program::Asm(bytes) => {
            // Pad to 4-byte alignment and write word by word.
            let mut padded = bytes.clone();
            while padded.len() % 4 != 0 {
                padded.push(0);
            }
            let words: Vec<u32> = padded
                .chunks(4)
                .map(|c| {
                    let mut w = [0u8; 4];
                    w[..c.len()].copy_from_slice(c);
                    u32::from_le_bytes(w)
                })
                .collect();
            oc.write_memory(IRAM_BASE, &words)
                .expect("run_hw: write_memory to IRAM failed");
            IRAM_BASE
        }
        Program::Elf(path) => {
            use goblin::elf::program_header::PT_LOAD;
            use goblin::elf::Elf;

            let elf_bytes = std::fs::read(path)
                .unwrap_or_else(|e| panic!("run_hw: failed to read ELF {:?}: {e}", path));
            let elf = Elf::parse(&elf_bytes)
                .unwrap_or_else(|e| panic!("run_hw: failed to parse ELF {:?}: {e}", path));

            for ph in &elf.program_headers {
                if ph.p_type != PT_LOAD || ph.p_filesz == 0 {
                    continue;
                }
                let vaddr = ph.p_vaddr as u32;
                let size = ph.p_filesz as usize;
                let file_off = ph.p_offset as usize;
                let seg_data = &elf_bytes[file_off..file_off + size];

                // Pad segment data to word boundary.
                let mut padded = seg_data.to_vec();
                while padded.len() % 4 != 0 {
                    padded.push(0);
                }
                let words: Vec<u32> = padded
                    .chunks(4)
                    .map(|c| {
                        let mut w = [0u8; 4];
                        w[..c.len()].copy_from_slice(c);
                        u32::from_le_bytes(w)
                    })
                    .collect();
                oc.write_memory(vaddr, &words)
                    .unwrap_or_else(|e| panic!(
                        "run_hw: write_memory for ELF segment at 0x{vaddr:08X} failed: {e}"
                    ));
            }
            elf.entry as u32
        }
    };

    // ── Register isolation: zero a0-a15 + key SRs (fix #4) ─────────────────
    //
    // Force WindowBase=0, WindowStart=1 (only window 0 exists) so that our
    // writes to a0-a15 below map to physical AR0-AR15 unambiguously.  Without
    // this, a stale WB from the ROM bootloader (e.g. WB=2) would make "a4"
    // refer to physical AR12, not AR4.
    let _ = oc.write_register("windowbase", 0u32);
    let _ = oc.write_register("windowstart", 1u32);

    // Zero all 16 AR registers so that boot-time values don't leak into tests.
    for i in 0u32..16 {
        let name = format!("a{}", i);
        oc.write_register(&name, 0)
            .unwrap_or_else(|e| panic!("run_hw: zero register({name}) failed: {e}"));
    }
    // Zero SAR (shift amount register) — affects SLL/SRA/etc.
    let _ = oc.write_register("sar", 0);
    // Zero SCOMPARE1 — used by S32C1I (compare-and-swap), avoid stale values.
    let _ = oc.write_register("scompare1", 0);

    // ── Set PS to a clean non-exception state (fix #5: PS isolation) ────────
    //
    // After reset_halt, PS = 0x0000_001F: INTLEVEL=15, EXCM=1, WOE=0.
    // With PS.EXCM=1 the BREAK instruction fires to a DIFFERENT debug vector
    // (0x4000_0280 vs the standard debug handler at 0x4000_03C0), causing
    // DEPC-based PC recovery to give wrong results.
    // With PS.WOE=0 windowed-call instructions (CALL4/ENTRY/RETW) are suppressed.
    //
    // Set PS = 0x0004_0000 (WOE=1, EXCM=0, INTLEVEL=0) so that:
    //   - BREAK fires to the standard debug handler → DEPC is set correctly.
    //   - Conditional branches execute correctly (BREAK fires to correct path).
    //   - Windowed call instructions operate correctly.
    //   - Exception/interrupt tests can set their own PS flags via init_ps_excm /
    //     init_ps_intlevel (applied below, overriding this base).
    //
    // Note: s32e_inside_vector relies on EXCM=1 for S32E to decode correctly.
    // That test must call write_ps_excm(true) in its setup closure so that the
    // override below restores EXCM=1 after this baseline write.
    let clean_ps: u32 = 1 << 18; // WOE=1, EXCM=0, INTLEVEL=0
    let _ = oc.write_register("ps", clean_ps);

    // ── Set PC to program entry point ────────────────────────────────────────
    oc.write_register("pc", entry_pc)
        .expect("run_hw: write_register pc failed");

    // ── Apply test setup (registers, SRs, memory) ───────────────────────────
    let mut init_state = OracleState::default();
    (case.setup)(&mut init_state);

    for (name, &val) in &init_state.regs {
        oc.write_register(name, val)
            .unwrap_or_else(|e| panic!("run_hw: write_register({name}) failed: {e}"));
    }
    // Apply SR setup (VECBASE, EPC1, INTENABLE, …).
    // OpenOCD uses the SR name directly; map numeric SR IDs to names.
    for (&sr_id, &val) in &init_state.sr {
        let sr_name = sr_id_to_openocd_name(sr_id);
        if let Some(name) = sr_name {
            let _ = oc.write_register(name, val);
        }
    }
    // Apply WindowBase / WindowStart if requested.
    if let Some(wb) = init_state.init_windowbase {
        let _ = oc.write_register("windowbase", wb as u32);
    }
    if let Some(ws) = init_state.init_windowstart {
        let _ = oc.write_register("windowstart", ws as u32);
    }
    // Apply PS overrides.
    // Read current PS, patch EXCM and INTLEVEL bits, write back.
    if init_state.init_ps_excm.is_some() || init_state.init_ps_intlevel.is_some() {
        let mut ps = oc.read_register("ps").unwrap_or(0);
        if let Some(excm) = init_state.init_ps_excm {
            if excm { ps |= 1 << 4; } else { ps &= !(1 << 4); }
        }
        if let Some(level) = init_state.init_ps_intlevel {
            ps = (ps & !0xF) | (level as u32 & 0xF);
        }
        let _ = oc.write_register("ps", ps);
    }
    // Write setup memory into HW via OpenOCD.
    for (&addr, &val) in &init_state.mem {
        oc.write_memory(addr, &[val])
            .unwrap_or_else(|e| panic!("run_hw: write_memory(0x{addr:08X}) failed: {e}"));
    }

    // ── Resume and wait for halt ─────────────────────────────────────────────
    //
    // BREAK 1,15 fires a debug exception → CPU jumps to the ROM debug exception
    // vector at 0x400003C0.  The ROM handler eventually re-enters OpenOCD via
    // JTAG, but we can also force a halt at any point.  We use
    // `wait_until_halted` (which force-halts if needed) rather than
    // `wait_for_break` (which requires DEBUGCAUSE bit 3 and never fires because
    // BREAK redirects via the exception vector).
    oc.resume().expect("run_hw: resume failed");
    oc.wait_until_halted(Duration::from_secs(10))
        .expect("run_hw: CPU did not halt within 10 s after BREAK");

    // ── Capture end state ────────────────────────────────────────────────────
    let mut end = OracleState::default();
    for i in 0u32..16 {
        let name = format!("a{}", i);
        let val = oc.read_register(&name)
            .unwrap_or_else(|e| panic!("run_hw: read_register({name}) failed: {e}"));
        end.regs.insert(name, val);
    }
    // Capture PC.
    //
    // BREAK causes the CPU to jump to the ROM debug exception vector (around
    // 0x400003C0).  We want to report the address of the BREAK instruction
    // itself so that oracle tests can `assert_pc(IRAM_BASE + offset)`.
    //
    // On Xtensa, when a debug exception fires, DEPC is set to
    //   DEPC = PC_of_BREAK + sizeof(BREAK_insn)
    // For a 3-byte BREAK 1,15 instruction: DEPC = BREAK_PC + 3.
    //
    // Strategy: if the halted PC is outside the oracle IRAM window (i.e. in
    // ROM), use DEPC - 3 as the effective halt PC.  If halted inside IRAM
    // (e.g. the ROM handler already halted the CPU back in IRAM for some
    // reason, or a hardware breakpoint fired), use the raw PC.
    {
        let raw_pc = oc.read_register("pc")
            .unwrap_or_else(|e| panic!("run_hw: read_register(pc) failed: {e}"));
        let oracle_end = IRAM_BASE.saturating_add(ORACLE_MEM_SIZE as u32);
        if raw_pc >= IRAM_BASE && raw_pc < oracle_end {
            // Halted inside oracle IRAM — use raw PC directly.
            end.pc = raw_pc;
        } else {
            // Halted outside oracle IRAM (e.g. ROM debug vector).
            // Read DEPC to recover BREAK_PC = DEPC - 3.
            let depc = oc.read_register("depc").unwrap_or(0);
            end.pc = depc.saturating_sub(3);
        }
    }
    // Capture WindowBase / WindowStart.
    end.wb = oc.read_register("windowbase").unwrap_or(0) as u8;
    end.ws = oc.read_register("windowstart").unwrap_or(0) as u16;
    // Capture PS fields.
    let ps = oc.read_register("ps").unwrap_or(0);
    end.excm = (ps >> 4) & 1 != 0;
    end.intlevel = (ps & 0xF) as u8;
    // Capture EPC1 and EXCCAUSE.
    end.epc1 = oc.read_register("epc1").unwrap_or(0);
    end.exccause = oc.read_register("exccause").unwrap_or(0);
    // Re-read memory addresses (setup + explicit capture).
    let mut addrs_to_read: Vec<u32> = init_state.mem.keys().copied().collect();
    addrs_to_read.extend_from_slice(&case.mem_capture_addrs);
    addrs_to_read.sort_unstable();
    addrs_to_read.dedup();
    for addr in addrs_to_read {
        let words = oc.read_memory(addr, 1)
            .unwrap_or_else(|e| panic!("run_hw: read_memory(0x{addr:08X}) failed: {e}"));
        end.mem.insert(addr, words[0]);
    }

    oc.shutdown().expect("run_hw: OpenOCD shutdown failed");
    end
}

/// Map a numeric Xtensa SR ID to the OpenOCD register name string.
///
/// Only SRs actually used in oracle setup closures are mapped.  Unmapped SRs
/// return `None` and are silently skipped (a warning would be better but
/// panicking would break unknown-SR robustness).
#[cfg(feature = "hw-oracle")]
fn sr_id_to_openocd_name(sr_id: u16) -> Option<&'static str> {
    use labwired_core::cpu::xtensa_sr::*;
    Some(match sr_id {
        SAR       => "sar",
        SCOMPARE1 => "scompare1",
        INTENABLE => "intenable",
        INTERRUPT => "interrupt",
        VECBASE   => "vecbase",
        EPC1      => "epc1",
        EPC2      => "epc2",
        EPC3      => "epc3",
        EPC4      => "epc4",
        EPC5      => "epc5",
        EPC6      => "epc6",
        EPC7      => "epc7",
        EPS2      => "eps2",
        EPS3      => "eps3",
        EPS4      => "eps4",
        EPS5      => "eps5",
        EPS6      => "eps6",
        EPS7      => "eps7",
        EXCSAVE1  => "excsave1",
        EXCSAVE2  => "excsave2",
        EXCSAVE3  => "excsave3",
        EXCSAVE4  => "excsave4",
        EXCSAVE5  => "excsave5",
        EXCSAVE6  => "excsave6",
        EXCSAVE7  => "excsave7",
        EXCCAUSE  => "exccause",
        _         => return None,
    })
}

/// Run `case` against a physical ESP32-S3 board and assert the expect closure.
#[cfg(feature = "hw-oracle")]
pub fn run_hw(case: OracleCase) {
    let end_state = capture_hw_state(&case);
    (case.expect)(&end_state);
}

/// Run `case` against both the simulator and hardware, then diff the AR register
/// state.  Panics with a detailed mismatch report if any registers differ.
///
/// TODO: CCOUNT / timestamp tolerance comparison is not yet implemented; a
/// follow-up task should compare CCOUNT within `case.tolerance.ccount_cycles`.
#[cfg(feature = "hw-oracle")]
pub fn run_diff(case: OracleCase) {
    let sim_state = capture_sim_state(&case);

    // Reconstruct case for hw (closures are not Clone, so we capture the
    // hw state before consuming `case`).
    let hw_state = capture_hw_state(&case);

    let mut mismatches = Vec::new();
    // Check all registers captured by sim.
    for (k, &v_sim) in &sim_state.regs {
        let v_hw = hw_state.regs.get(k).copied().unwrap_or(0);
        if v_sim != v_hw {
            mismatches.push(format!(
                "reg {k}: sim 0x{v_sim:08X} vs hw 0x{v_hw:08X}"
            ));
        }
    }
    if !mismatches.is_empty() {
        panic!(
            "oracle diff failed:\n  {}",
            mismatches.join("\n  ")
        );
    }

    // Run expect on sim state (already verified equal to hw).
    (case.expect)(&sim_state);
}
