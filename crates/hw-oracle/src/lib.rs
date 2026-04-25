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

/// Size of the oracle program scratch region within IRAM.
/// 64 KiB is sufficient for all Plan-1 oracle programs.
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

/// Snapshot of CPU register state used by setup and expect closures.
#[derive(Debug, Clone, Default)]
pub struct OracleState {
    /// Register values keyed by Xtensa name: `"a0"`..`"a15"`, `"pc"`, etc.
    pub regs: HashMap<String, u32>,
    /// Memory snapshot (address → 32-bit word); reserved for future use.
    pub mem: HashMap<u32, u32>,
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
fn capture_sim_state(case: &OracleCase) -> OracleState {
    use labwired_core::bus::SystemBus;
    use labwired_core::cpu::xtensa_lx7::XtensaLx7;
    use labwired_core::{Cpu, SimulationError};
    use ram_peripheral::RamPeripheral;

    let bytes = match &case.program {
        Program::Asm(b) => b.clone(),
        Program::Elf(_) => panic!("run_sim: ELF programs are not yet supported in sim mode"),
    };

    // Build a minimal bus: default SystemBus peripherals are all at STM32
    // addresses (0x2000_0000 RAM, 0x0 flash).  We patch it by adding an oracle
    // IRAM peripheral at IRAM_BASE.
    //
    // SystemBus::new() provides RAM at 0x2000_0000 and flash at 0x0.  Both are
    // outside the Xtensa IRAM window (0x40370000), so we register a RamPeripheral
    // at that address to hold the oracle program.
    let mut bus = SystemBus::new();

    // Build the peripheral with the program bytes pre-loaded.
    let mut iram = RamPeripheral::new(ORACLE_MEM_SIZE);
    iram.write_bytes(0, &bytes);
    bus.add_peripheral(
        "oracle_iram",
        IRAM_BASE as u64,
        ORACLE_MEM_SIZE as u64,
        None,
        Box::new(iram),
    );

    let mut cpu = XtensaLx7::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(IRAM_BASE);

    // Apply setup state.
    let mut init_state = OracleState::default();
    (case.setup)(&mut init_state);
    for (name, &val) in &init_state.regs {
        if let Some(idx) = parse_ar_name(name) {
            cpu.regs.write_logical(idx, val);
        }
        // SR / special register setup is deferred (not needed for ADD oracle).
    }

    // Step until BREAK (BreakpointHit) or limit.
    const MAX_STEPS: usize = 10_000;
    for _ in 0..MAX_STEPS {
        match cpu.step(&mut bus, &[]) {
            Ok(()) => {}
            Err(SimulationError::BreakpointHit(_)) => {
                // Normal termination.
                let mut end = OracleState::default();
                for i in 0u8..16 {
                    end.regs.insert(format!("a{}", i), cpu.regs.read_logical(i));
                }
                return end;
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
#[cfg(feature = "hw-oracle")]
fn capture_hw_state(case: &OracleCase) -> OracleState {
    use crate::flash::TargetBoard;
    use crate::openocd::OpenOcd;
    use std::time::Duration;

    let bytes = match &case.program {
        Program::Asm(b) => b.clone(),
        Program::Elf(_) => panic!("run_hw: ELF programs are not yet supported"),
    };

    let board = TargetBoard::detect()
        .expect("run_hw: ESP32-S3 board not detected; is it connected via USB-JTAG?");
    let mut oc = OpenOcd::spawn_for(&board)
        .expect("run_hw: failed to spawn OpenOCD");

    oc.reset_halt().expect("run_hw: reset_halt failed");

    // Pad bytes to 4-byte alignment for word writes.
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

    // Set PC to start of oracle program.
    oc.write_register("pc", IRAM_BASE)
        .expect("run_hw: write_register pc failed");

    // Apply register setup.
    let mut init_state = OracleState::default();
    (case.setup)(&mut init_state);
    for (name, &val) in &init_state.regs {
        oc.write_register(name, val)
            .unwrap_or_else(|e| panic!("run_hw: write_register({name}) failed: {e}"));
    }

    // Resume execution; BREAK will halt the CPU.
    oc.resume().expect("run_hw: resume failed");

    // Poll until halted (BREAK triggers a debug exception and halts the CPU).
    oc.wait_until_halted(Duration::from_secs(5))
        .expect("run_hw: CPU did not halt within 5 s after BREAK");

    // Capture end state.
    let mut end = OracleState::default();
    for i in 0u32..16 {
        let name = format!("a{}", i);
        let val = oc.read_register(&name)
            .unwrap_or_else(|e| panic!("run_hw: read_register({name}) failed: {e}"));
        end.regs.insert(name, val);
    }

    oc.shutdown().expect("run_hw: OpenOCD shutdown failed");
    end
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
