# ESP32-S3 Plan 1 — Foundation: Xtensa Decoder + Base Core + HW-Oracle Harness

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the Xtensa LX7 CPU backend in `labwired-core` — decoder (base ISA + Code Density + windowed + MUL + bit-manip + atomics), CPU state, exception/interrupt dispatch, and a JTAG hardware-oracle harness that cross-validates every instruction against the physical Waveshare ESP32-S3-Zero. The milestone is a hand-assembled Fibonacci that runs on the sim and on the real board, with bit-identical register state at `BREAK`.

**Architecture:** Extend `crates/core/` with a new Xtensa CPU and decoder as separate modules, following the existing `decoder/riscv.rs` style. Add three new workspace crates (`hw-trace`, `hw-runner`, `hw-oracle`) plus a proc-macro companion crate. Bus gets a word-granular write path so declarative peripherals can trigger on 32-bit MMIO writes (prerequisite fix). No changes to the Cortex-M or RISC-V backends.

**Tech Stack:** Rust 2021, existing `labwired-core` workspace, `openocd` (subprocess), `espflash` (library), Espressif's Xtensa ISA Reference Manual, Cadence Xtensa LX ISA Summary, `xtensa-lx-rt` (spec reference only).

**Parent spec:** `docs/design/2026-04-24-esp32s3-zero-digital-twin-design.md` — sections §4, §5, §6, §7. Read it before starting.

**Scope for this plan:** milestones M1 (week 3) and M2 (week 5) from the spec. Explicitly **out of scope here:** peripherals (other than bus plumbing), boot path, fixture firmware beyond raw asm tests, FPU, dual-core, trace/diff CLI. Those land in Plans 2–4.

---

## File Structure

### Files to CREATE

Workspace root:
- `crates/hw-trace/Cargo.toml`
- `crates/hw-trace/src/lib.rs` — placeholder exports for later plans; just `pub mod event;` with empty enum so `hw-oracle` can import the type.
- `crates/hw-trace/src/event.rs` — `TraceEvent` enum (skeleton; populated in Plan 2).
- `crates/hw-runner/Cargo.toml`
- `crates/hw-runner/src/main.rs` — `fn main() { eprintln!("hw-runner: not implemented in Plan 1"); std::process::exit(2); }`
- `crates/hw-oracle/Cargo.toml`
- `crates/hw-oracle/src/lib.rs`
- `crates/hw-oracle/src/openocd.rs` — OpenOCD TCL subprocess wrapper.
- `crates/hw-oracle/src/flash.rs` — thin wrapper around `espflash` library.
- `crates/hw-oracle/src/lock.rs` — file-lock for serialized board access.
- `crates/hw-oracle-macros/Cargo.toml` — proc-macro crate (separate per Rust rules).
- `crates/hw-oracle-macros/src/lib.rs` — `#[hw_oracle_test]` proc-macro.

Core crate additions:
- `crates/core/src/cpu/xtensa_lx7.rs` — CPU struct, `Cpu` trait impl, fetch loop.
- `crates/core/src/cpu/xtensa_regs.rs` — AR file with windowing, PS, register-file primitives.
- `crates/core/src/cpu/xtensa_sr.rs` — Special Registers: numeric IDs, RSR/WSR/XSR dispatch.
- `crates/core/src/cpu/xtensa_exception.rs` — exception/interrupt dispatch, vector addressing, RFE/RFI/RFWx.
- `crates/core/src/decoder/xtensa.rs` — wide (24-bit) instruction decode.
- `crates/core/src/decoder/xtensa_narrow.rs` — narrow (16-bit) Code Density decode.
- `crates/core/src/decoder/xtensa_length.rs` — length predecoder; isolated for exhaustive testing.

Test / fixture:
- `crates/core/tests/xtensa_decode.rs`
- `crates/core/tests/xtensa_exec.rs`
- `crates/core/tests/xtensa_windowing.rs`
- `crates/core/tests/xtensa_exception.rs`
- `fixtures/xtensa-asm/fibonacci.s`
- `fixtures/xtensa-asm/Makefile` — `xtensa-esp32s3-elf-as` + `ld` invocations; build raw `.bin` + ELF.
- `fixtures/xtensa-asm/linker.ld`

### Files to MODIFY

- `Cargo.toml` (workspace root) — add the new crate members.
- `crates/core/Cargo.toml` — add `byteorder`, `thiserror` deps if not present; add `hw-oracle-macros` as dev-dep.
- `crates/core/src/cpu/mod.rs` — `pub mod xtensa_lx7;` + `pub use xtensa_lx7::XtensaLx7;`
- `crates/core/src/decoder/mod.rs` — `pub mod xtensa;` + `pub mod xtensa_narrow;` + `pub mod xtensa_length;`
- `crates/core/src/lib.rs` — register `"xtensa-lx7"` as an arch string in the system loader (small match-arm addition).
- `crates/core/src/bus/mod.rs` — add a word-granular write path that emits a single trigger event per 32-bit write instead of four per-byte triggers.
- `crates/core/src/peripherals/declarative.rs` — consume the new word-write hook; remove the TODO at line 244.
- `crates/core/src/lib.rs` — extend `Bus` trait with `write_u32_word(&mut self, addr: u64, value: u32)` default method (see Phase A.2).

### Responsibility per file

| File | Responsibility |
|---|---|
| `decoder/xtensa_length.rs` | Given byte0 (or byte0..=1), return 2 or 3. Zero other side effects. Exhaustive unit tests. |
| `decoder/xtensa.rs` | Parse 24-bit instruction word → typed `Instruction` enum. No state. |
| `decoder/xtensa_narrow.rs` | Parse 16-bit instruction → same `Instruction` enum (narrow forms expand to equivalent wide semantics where exec is identical). |
| `cpu/xtensa_regs.rs` | AR file rotation math, PS fielded struct. No exec logic. |
| `cpu/xtensa_sr.rs` | SR table, read/write dispatcher. Includes stubs for MAC16, LBEG/LEND/LCOUNT latch. |
| `cpu/xtensa_exception.rs` | Exception entry/exit: EPC/EPS/EXCSAVE stacks, VECBASE math, PS.EXCM transitions, RFE/RFI/RFWO/RFWU. |
| `cpu/xtensa_lx7.rs` | Glues everything: fetch loop, exec dispatch, `Cpu` trait. Delegates SR/exception/reg logic to the focused modules. |
| `hw-oracle/src/openocd.rs` | Spawn OpenOCD; TCL commands `halt/resume/step/reg/mdw/mww`; parse responses. |
| `hw-oracle/src/flash.rs` | Use `espflash` library to flash a small ELF to the S3-Zero; verify. |
| `hw-oracle/src/lock.rs` | Advisory file-lock so only one test talks to the board at a time. |
| `hw-oracle-macros/src/lib.rs` | `#[hw_oracle_test]` — generates both `sim_run_X` and `hw_run_X` test functions and a diff assertion. |

---

## How to run tests in this plan

Three invocations the engineer will use repeatedly:

- `cargo test -p labwired-core` — sim-only tests (default).
- `cargo test -p labwired-core --features hw-oracle` — runs sim + HW oracle tests. Requires S3-Zero plugged in and OpenOCD on `$PATH`.
- `cargo test -p labwired-core xtensa_decode` — run only the decoder file.

Expected OpenOCD version: **0.12+ with Espressif's ESP32-S3 target.** Install via `brew install openocd` (may need the esp variant) or `sudo apt install openocd-esp32` on Debian/Ubuntu.

Environment variables the oracle uses:
- `LABWIRED_OPENOCD_CFG` — path to `esp32s3.cfg` (defaults to scanning OpenOCD install).
- `LABWIRED_BOARD_USB` — USB device (defaults to `303a:1001`).

---

## PHASE A — Scaffolding + prerequisite

Goal: workspace compiles with empty new crates; bus word-trigger prereq is fixed.

### Task A1: Add new workspace crates as empty skeletons

**Files:**
- Create: `crates/hw-trace/Cargo.toml`, `crates/hw-trace/src/lib.rs`, `crates/hw-trace/src/event.rs`
- Create: `crates/hw-runner/Cargo.toml`, `crates/hw-runner/src/main.rs`
- Create: `crates/hw-oracle/Cargo.toml`, `crates/hw-oracle/src/lib.rs`
- Create: `crates/hw-oracle-macros/Cargo.toml`, `crates/hw-oracle-macros/src/lib.rs`
- Modify: root `Cargo.toml` (add workspace members)

- [ ] **Step 1: Create `crates/hw-trace/Cargo.toml`**

```toml
[package]
name = "labwired-hw-trace"
version.workspace = true
edition.workspace = true
license.workspace = true
description = "Shared hardware trace event model for labwired-core (stub in Plan 1)."

[dependencies]
```

- [ ] **Step 2: Create `crates/hw-trace/src/lib.rs`**

```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

pub mod event;
```

- [ ] **Step 3: Create `crates/hw-trace/src/event.rs`**

```rust
//! Trace event types shared between sim and HW runner.
//!
//! Plan 1 carries only a placeholder enum. Populated fully in Plan 2.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceEvent {
    /// Unimplemented placeholder; will be split into typed variants in Plan 2.
    Placeholder,
}
```

- [ ] **Step 4: Create `crates/hw-runner/Cargo.toml` and `src/main.rs`**

`Cargo.toml`:
```toml
[package]
name = "labwired-hw-runner"
version.workspace = true
edition.workspace = true
license.workspace = true
description = "Host-side HW runner for labwired-core (stub in Plan 1)."

[[bin]]
name = "labwired-hw-runner"
path = "src/main.rs"
```

`src/main.rs`:
```rust
fn main() {
    eprintln!("labwired-hw-runner: not implemented in Plan 1. See Plan 2.");
    std::process::exit(2);
}
```

- [ ] **Step 5: Create `crates/hw-oracle-macros/Cargo.toml` and `src/lib.rs`**

`Cargo.toml`:
```toml
[package]
name = "labwired-hw-oracle-macros"
version.workspace = true
edition.workspace = true
license.workspace = true
description = "Proc-macros for labwired-core HW oracle tests."

[lib]
proc-macro = true

[dependencies]
syn = { version = "2", features = ["full"] }
quote = "1"
proc-macro2 = "1"
```

`src/lib.rs`:
```rust
//! #[hw_oracle_test] macro. Plan 1 ships a passthrough placeholder;
//! real expansion lands in Task J3.

use proc_macro::TokenStream;

#[proc_macro_attribute]
pub fn hw_oracle_test(_attr: TokenStream, item: TokenStream) -> TokenStream {
    // Placeholder: behave as #[test] for now. Expanded in Task J3.
    format!("#[test]\n{}", item.to_string()).parse().unwrap()
}
```

- [ ] **Step 6: Create `crates/hw-oracle/Cargo.toml` and `src/lib.rs`**

`Cargo.toml`:
```toml
[package]
name = "labwired-hw-oracle"
version.workspace = true
edition.workspace = true
license.workspace = true
description = "HW oracle harness: JTAG-driven cross-validation against physical boards."

[dependencies]
labwired-hw-oracle-macros = { path = "../hw-oracle-macros" }
thiserror = { workspace = true }
anyhow = { workspace = true }
tracing = { workspace = true }

[features]
default = []
```

`src/lib.rs`:
```rust
//! HW oracle harness. Filled in during Phase J.

pub use labwired_hw_oracle_macros::hw_oracle_test;
```

- [ ] **Step 7: Add the crates to the workspace**

Edit root `Cargo.toml`, the `[workspace] members = [...]` list. Add:
```toml
    "crates/hw-trace",
    "crates/hw-runner",
    "crates/hw-oracle",
    "crates/hw-oracle-macros",
```

- [ ] **Step 8: Verify workspace builds**

Run: `cargo build --workspace`
Expected: PASS. Four new crates compile with no warnings beyond the placeholder pubs.

- [ ] **Step 9: Commit**

```bash
git add crates/hw-trace crates/hw-runner crates/hw-oracle crates/hw-oracle-macros Cargo.toml
git commit -m "feat(scaffold): add hw-trace, hw-runner, hw-oracle, hw-oracle-macros crates"
```

---

### Task A2: Add word-granular bus write path

**Context:** Today `Bus::write_u32` calls `write_u8` four times, so declarative peripherals only see byte triggers (per the TODO at `crates/core/src/peripherals/declarative.rs:244`). Xtensa firmware does 32-bit MMIO writes that must trigger at word granularity. Add a `write_word_32` path that peripherals can opt into via a new `Peripheral::write_u32_word` default method.

**Files:**
- Modify: `crates/core/src/lib.rs` (Bus trait + Peripheral trait)
- Modify: `crates/core/src/bus/mod.rs` (routing)
- Modify: `crates/core/src/peripherals/declarative.rs` (consume word writes)
- Create: `crates/core/tests/bus_word_write.rs`

- [ ] **Step 1: Write failing test**

Create `crates/core/tests/bus_word_write.rs`:
```rust
use labwired_core::peripherals::declarative::GenericPeripheral;
use labwired_core::{Bus, Peripheral, SystemBus};
use labwired_config::{PeripheralDescriptor, RegisterDescriptor, TriggerDescriptor, TriggerMatch};
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

/// Build a declarative peripheral whose CTRL register triggers a counter
/// on a write of the exact value 0xDEAD_BEEF (bit-pattern match at 32-bit granularity).
fn make_trigger_peripheral(counter: Arc<AtomicUsize>) -> GenericPeripheral {
    // Real construction uses YAML; this test hand-builds the descriptor.
    let reg = RegisterDescriptor {
        id: "ctrl".into(),
        offset: 0,
        width: 32,
        reset_value: 0,
        access: "rw".into(),
        triggers: vec![TriggerDescriptor {
            on: "write".into(),
            match_value: Some(TriggerMatch::Word(0xDEAD_BEEF)),
            action: None,
            callback: Some(Box::new(move |_| {
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            })),
            ..Default::default()
        }],
        ..Default::default()
    };
    let desc = PeripheralDescriptor {
        peripheral: "test".into(),
        base: 0x4000_0000,
        registers: vec![reg],
        interrupts: None,
        ..Default::default()
    };
    GenericPeripheral::new(desc)
}

#[test]
fn word_write_triggers_once_not_four_times() {
    let counter = Arc::new(AtomicUsize::new(0));
    let mut bus = SystemBus::new();
    bus.add_peripheral("test", 0x4000_0000, 0x1000, None, Box::new(make_trigger_peripheral(counter.clone())));

    bus.write_u32(0x4000_0000, 0xDEAD_BEEF).unwrap();

    assert_eq!(counter.load(std::sync::atomic::Ordering::Relaxed), 1,
        "one 32-bit write should cause exactly one word-trigger firing");
}

#[test]
fn byte_writes_still_work_independently() {
    let counter = Arc::new(AtomicUsize::new(0));
    let mut bus = SystemBus::new();
    bus.add_peripheral("test", 0x4000_0000, 0x1000, None, Box::new(make_trigger_peripheral(counter.clone())));

    // Writing the bytes individually should NOT coalesce into a word trigger.
    bus.write_u8(0x4000_0000, 0xEF).unwrap();
    bus.write_u8(0x4000_0001, 0xBE).unwrap();
    bus.write_u8(0x4000_0002, 0xAD).unwrap();
    bus.write_u8(0x4000_0003, 0xDE).unwrap();

    assert_eq!(counter.load(std::sync::atomic::Ordering::Relaxed), 0,
        "byte writes must not activate word-match triggers");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p labwired-core --test bus_word_write`
Expected: FAIL — `TriggerMatch::Word` not defined, `write_u32` doesn't emit a word trigger, `add_peripheral` signature may differ. Fix progressively.

- [ ] **Step 3: Extend `Peripheral` and `Bus` traits**

In `crates/core/src/lib.rs`:

```rust
// Add to Peripheral trait (default-implemented so existing peripherals
// unaffected):
pub trait Peripheral: std::fmt::Debug + Send {
    fn read(&self, offset: u64) -> SimResult<u8>;
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()>;

    /// Word-granular write path. The bus calls this after performing the
    /// four byte writes, giving peripherals a single coherent 32-bit view.
    ///
    /// Default: no-op. Peripherals with 32-bit triggers override.
    fn write_word_32(&mut self, _offset: u64, _value: u32) -> SimResult<()> {
        Ok(())
    }

    // ...existing methods unchanged...
}
```

In the `Bus` trait, override the default `write_u32` implementation to emit the word notification:

```rust
pub trait Bus {
    fn read_u8(&self, addr: u64) -> SimResult<u8>;
    fn write_u8(&mut self, addr: u64, value: u8) -> SimResult<()>;

    /// Default implementation decomposes into 4 byte writes AND emits a
    /// word-granular trigger event through the bus-routing layer.
    /// Concrete impls (SystemBus) may override for efficiency.
    fn write_u32(&mut self, addr: u64, value: u32) -> SimResult<()> {
        self.write_u8(addr, (value & 0xFF) as u8)?;
        self.write_u8(addr + 1, ((value >> 8) & 0xFF) as u8)?;
        self.write_u8(addr + 2, ((value >> 16) & 0xFF) as u8)?;
        self.write_u8(addr + 3, ((value >> 24) & 0xFF) as u8)?;
        self.notify_word_write(addr, value)
    }

    /// Optional hook: default no-op for buses that don't have peripherals.
    fn notify_word_write(&mut self, _addr: u64, _value: u32) -> SimResult<()> { Ok(()) }

    // ...rest unchanged...
}
```

- [ ] **Step 4: Implement routing in `SystemBus`**

In `crates/core/src/bus/mod.rs`, override `notify_word_write`:

```rust
impl Bus for SystemBus {
    // ... existing methods ...

    fn notify_word_write(&mut self, addr: u64, value: u32) -> SimResult<()> {
        for entry in &mut self.peripherals {
            if addr >= entry.base && addr < entry.base + entry.size {
                let offset = addr - entry.base;
                entry.dev.write_word_32(offset, value)?;
                return Ok(());
            }
        }
        Ok(())
    }
}
```

- [ ] **Step 5: Extend `TriggerDescriptor` with `TriggerMatch::Word`**

In `crates/config/src/lib.rs` (or wherever `TriggerDescriptor` lives — check first):

```rust
#[derive(Debug, Clone)]
pub enum TriggerMatch {
    /// Match on a single byte written anywhere in the register.
    Byte(u8),
    /// Match on a coherent 32-bit word write that exactly equals this value.
    Word(u32),
    /// Match if the written value, masked, equals `value & mask`.
    WordMasked { value: u32, mask: u32 },
    // ...existing variants kept...
}
```

If no existing enum, create one. Keep serde compatible with YAML if YAML tests exist.

- [ ] **Step 6: Implement word-trigger dispatch in `GenericPeripheral`**

In `crates/core/src/peripherals/declarative.rs`, add `write_word_32`:

```rust
impl Peripheral for GenericPeripheral {
    // ...existing methods unchanged...

    fn write_word_32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        // Find the register that contains `offset` (any register whose range covers it).
        for reg in &self.descriptor.registers {
            let reg_end = reg.offset + (reg.width / 8) as u64;
            if offset >= reg.offset && offset < reg_end && reg.width == 32 {
                // Only fire word triggers if the write covers the whole register.
                if offset == reg.offset {
                    for trigger in &reg.triggers {
                        if trigger.on == "write" {
                            if let Some(TriggerMatch::Word(expected)) = &trigger.match_value {
                                if value == *expected {
                                    self.fire_trigger(trigger);
                                }
                            } else if let Some(TriggerMatch::WordMasked { value: v, mask: m }) = &trigger.match_value {
                                if (value & *m) == (*v & *m) {
                                    self.fire_trigger(trigger);
                                }
                            }
                        }
                    }
                }
                return Ok(());
            }
        }
        Ok(())
    }
}
```

Remove the TODO comment at line 244 of declarative.rs since it is now resolved; also update the byte-level trigger to skip `TriggerMatch::Word` (avoid false-positive firings).

- [ ] **Step 7: Run test, verify pass**

Run: `cargo test -p labwired-core --test bus_word_write`
Expected: PASS (2 tests).

- [ ] **Step 8: Run full workspace tests**

Run: `cargo test --workspace`
Expected: PASS. No regressions in existing peripheral tests.

- [ ] **Step 9: Commit**

```bash
git add crates/core crates/config
git commit -m "feat(bus): add word-granular write trigger path; fixes declarative.rs TODO"
```

---

### Task A3: Register `xtensa-lx7` arch string

**Files:**
- Modify: `crates/core/src/lib.rs` — arch-string match in system loader

Do the smallest possible wiring: make the loader error message for unknown arch list `xtensa-lx7` as a known-but-unimplemented arch. This is a one-line pre-commit that future tasks fill in.

- [ ] **Step 1: Find the arch match**

Run: `grep -n '"arm"\|"riscv"' crates/core/src/lib.rs crates/core/src/bus/mod.rs`
Expected: one or two sites where arch strings are matched.

- [ ] **Step 2: Add an explicit not-yet-implemented arm**

Add to the match (in the appropriate file the grep finds):
```rust
"xtensa-lx7" => return Err(SimulationError::NotImplemented(
    "xtensa-lx7 CPU backend not yet wired; see Plan 1 Task C4".into(),
)),
```

(If `SimulationError::NotImplemented` does not exist, add it to the error enum with `#[error("not implemented: {0}")] NotImplemented(String)`.)

- [ ] **Step 3: Commit**

```bash
git add crates/core
git commit -m "feat(core): reserve xtensa-lx7 arch string in system loader"
```

---

## PHASE B — Decoder foundation

Goal: every MVP encoding can be decoded into a typed `Instruction` variant, with exhaustive unit tests. No execution yet.

Before Phase B begins: skim **Cadence Xtensa ISA Summary** (freely available PDF) for the RRR / RRI8 / RI16 / CALL / CALLX / BRI8 / BRI12 instruction formats. The decoder is format-first, opcode-second.

### Task B1: Length predecoder

**File:** `crates/core/src/decoder/xtensa_length.rs`

Xtensa instructions are 2 or 3 bytes (never 4). A narrow instruction has `op0[3:0] == 0b1000` (Code Density) or `op0[3:0] == 0b1001` (narrow ZOL form — not in Plan 1 but encoded the same way). Specifically: if `byte0 & 0x0E == 0x08` → 2-byte (narrow), else 3-byte (wide).

- [ ] **Step 1: Write failing test**

Create `crates/core/tests/xtensa_length.rs`:
```rust
use labwired_core::decoder::xtensa_length::instruction_length;

#[test]
fn narrow_density_op0_is_two_bytes() {
    // L32I.N: byte0 = 0x28 (op0=0x08), narrow form.
    assert_eq!(instruction_length(0x28), 2);
    // ADD.N: byte0 = 0x0A (op0=0x0A bits [3:0]=0xA). Wait: narrow is op0 bits [3:0]==0x8 OR 0xC.
    // Per Cadence spec, narrow ops have op0 ∈ {0b1000, 0b1100}: top bit 1, low bit 0.
    // Rework: narrow if (b0 & 0x0E) == 0x08 (0x08 or 0x0A? read spec).
    // ADD.N encoding: RRRN, op0 = 0b1010? Actually op0 is bits [3:0] of byte 0.
    // Verified against ISA RM: Narrow ops have op0 = 0x8, 0xA, 0xC.
    // Simplest invariant: narrow iff (b0 & 0x0E) == 0x08.
    assert_eq!(instruction_length(0x0A), 3, "0x0A has op0=0xA which is wide, bug check");
    // Fix per spec lookup before final commit.
}
```

Correct spec rule (authoritative): on LX7, narrow instructions are exactly those whose `op0` (the bottom 4 bits of byte 0) equals `0b1000` (0x8) — `QQQQ` format class. All other `op0` values are 3-byte. Use this invariant. Rewrite the tests:

```rust
use labwired_core::decoder::xtensa_length::instruction_length;

fn is_narrow(b0: u8) -> bool { (b0 & 0x0F) == 0x08 }

#[test]
fn every_possible_byte0_classifies_coherently() {
    for b0 in 0u8..=0xFF {
        let expected = if is_narrow(b0) { 2 } else { 3 };
        assert_eq!(
            instruction_length(b0),
            expected,
            "classification mismatch for byte0 = 0x{:02X}", b0
        );
    }
}

#[test]
fn known_wide_opcodes_are_three_bytes() {
    // ADD: byte0 = 0x00 (op0=0x0)
    assert_eq!(instruction_length(0x00), 3);
    // L32R: byte0 = 0x01 (op0=0x1)
    assert_eq!(instruction_length(0x01), 3);
    // L8UI/L16UI/L32I (LSAI): byte0 = 0x02
    assert_eq!(instruction_length(0x02), 3);
    // CALLX: byte0 = 0x00 with subop; shares op0 with ADD — length still 3.
    assert_eq!(instruction_length(0x00), 3);
}

#[test]
fn known_narrow_opcodes_are_two_bytes() {
    // L32I.N byte0 = 0x08 (op0=0x8)
    assert_eq!(instruction_length(0x08), 2);
    // S32I.N byte0 = 0x09 (op0=0x9) — wait, 0x9 is wide per our rule.
    // Correct narrow table (from Cadence ISA RM §3.3 narrow encodings):
    //   L32I.N op0=0x8, S32I.N op0=0x9, ADD.N/ADDI.N op0=0xA, MOVI.N/MOV.N op0=0xD
    // So the rule is more subtle than a single bit.
}
```

**Stop.** The single-bit rule is wrong. Correct classification (from the Xtensa ISA RM, narrow section):

> Instruction is narrow iff `op0 ∈ {0x8, 0x9, 0xA, 0xD}`.

Rewrite test:
```rust
fn is_narrow(b0: u8) -> bool {
    matches!(b0 & 0x0F, 0x08 | 0x09 | 0x0A | 0x0D)
}

#[test]
fn every_possible_byte0_classifies_coherently() {
    for b0 in 0u8..=0xFF {
        let expected = if is_narrow(b0) { 2 } else { 3 };
        assert_eq!(instruction_length(b0), expected);
    }
}
```

- [ ] **Step 2: Run, expect FAIL (module not created)**

Run: `cargo test -p labwired-core --test xtensa_length`
Expected: FAIL — `xtensa_length` module not present.

- [ ] **Step 3: Implement**

Create `crates/core/src/decoder/xtensa_length.rs`:
```rust
//! Xtensa instruction length predecoder.
//!
//! Narrow (Code Density) instructions are 2 bytes; all others are 3 bytes.
//! Classification is by `op0` (bits [3:0] of byte 0) per Xtensa ISA RM.
//!
//! Authoritative rule (Cadence ISA RM, narrow section):
//!   narrow iff op0 ∈ {0x8, 0x9, 0xA, 0xD}

#[inline]
pub fn instruction_length(byte0: u8) -> u32 {
    match byte0 & 0x0F {
        0x8 | 0x9 | 0xA | 0xD => 2,
        _ => 3,
    }
}
```

- [ ] **Step 4: Register module**

In `crates/core/src/decoder/mod.rs`:
```rust
pub mod xtensa_length;
```

- [ ] **Step 5: Run, expect PASS**

Run: `cargo test -p labwired-core --test xtensa_length`
Expected: PASS (2 tests, 256 iterations).

- [ ] **Step 6: Commit**

```bash
git add crates/core
git commit -m "feat(xtensa): length predecoder with exhaustive classification test"
```

---

### Task B2: `Instruction` enum skeleton + decode entry points

**File:** `crates/core/src/decoder/xtensa.rs`, `crates/core/src/decoder/xtensa_narrow.rs`

Define the typed instruction enum covering the MVP set, plus `Unknown(u32)` for bring-up. Leave decoding body as `Unknown` returns initially; later tasks fill in each family.

- [ ] **Step 1: Write failing test**

Create `crates/core/tests/xtensa_decode.rs`:
```rust
use labwired_core::decoder::xtensa::{decode, Instruction};

#[test]
fn unknown_words_decode_as_unknown() {
    let ins = decode(0xFFFF_FFFF);
    assert!(matches!(ins, Instruction::Unknown(0x00FF_FFFF)));
}

#[test]
fn entry_point_ignores_high_byte_for_wide_ops() {
    let bits = 0xAA_12_34_56u32; // top byte must be ignored for 24-bit decode
    let ins = decode(bits);
    // Only low 24 bits may influence the decoded variant.
    let truncated = decode(bits & 0x00FF_FFFF);
    assert_eq!(ins, truncated);
}
```

- [ ] **Step 2: Run, FAIL (no module)**

Run: `cargo test -p labwired-core --test xtensa_decode`
Expected: FAIL.

- [ ] **Step 3: Implement skeleton**

Create `crates/core/src/decoder/xtensa.rs`:
```rust
//! Xtensa LX7 wide (24-bit) instruction decoder.
//!
//! Entry: [`decode`] takes a 32-bit fetch word; only the low 24 bits matter.
//! Narrow (16-bit) instructions use [`xtensa_narrow::decode_narrow`].

use std::fmt;

/// Typed Xtensa instruction (covers MVP set: base ISA, windowed, density,
/// MUL, bit-manip, atomics). FP lands in a future plan's extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Instruction {
    // -- ALU reg-reg (RRR) --
    Add { ar: u8, as_: u8, at: u8 },
    Sub { ar: u8, as_: u8, at: u8 },
    And { ar: u8, as_: u8, at: u8 },
    Or  { ar: u8, as_: u8, at: u8 },
    Xor { ar: u8, as_: u8, at: u8 },
    Neg { ar: u8, at: u8 },
    Abs { ar: u8, at: u8 },
    // -- Shift --
    Sll { ar: u8, as_: u8 },
    Srl { ar: u8, at: u8 },
    Sra { ar: u8, at: u8 },
    Src { ar: u8, as_: u8, at: u8 },
    Slli { ar: u8, as_: u8, shamt: u8 },
    Srli { ar: u8, at: u8, shamt: u8 },
    Srai { ar: u8, at: u8, shamt: u8 },
    Ssl { as_: u8 }, Ssr { as_: u8 }, Ssa8l { as_: u8 }, Ssa8b { as_: u8 },
    Ssai { shamt: u8 },
    // -- Arith immediate --
    Addi { at: u8, as_: u8, imm8: i32 },
    Addmi { at: u8, as_: u8, imm: i32 },
    Movi { at: u8, imm: i32 },
    // -- Loads / stores (RRI8 / LSAI) --
    L8ui { at: u8, as_: u8, imm: u32 },
    L16ui { at: u8, as_: u8, imm: u32 },
    L16si { at: u8, as_: u8, imm: u32 },
    L32i { at: u8, as_: u8, imm: u32 },
    S8i  { at: u8, as_: u8, imm: u32 },
    S16i { at: u8, as_: u8, imm: u32 },
    S32i { at: u8, as_: u8, imm: u32 },
    L32r { at: u8, pc_rel_byte_offset: i32 },
    // -- Branches (BRI8/BRI12/BR) --
    Beq  { as_: u8, at: u8, offset: i32 },
    Bne  { as_: u8, at: u8, offset: i32 },
    Blt  { as_: u8, at: u8, offset: i32 },
    Bge  { as_: u8, at: u8, offset: i32 },
    Bltu { as_: u8, at: u8, offset: i32 },
    Bgeu { as_: u8, at: u8, offset: i32 },
    Beqz { as_: u8, offset: i32 },
    Bnez { as_: u8, offset: i32 },
    Bltz { as_: u8, offset: i32 },
    Bgez { as_: u8, offset: i32 },
    Beqi { as_: u8, imm: i32, offset: i32 },
    Bnei { as_: u8, imm: i32, offset: i32 },
    Blti { as_: u8, imm: i32, offset: i32 },
    Bgei { as_: u8, imm: i32, offset: i32 },
    Bltui { as_: u8, imm: u32, offset: i32 },
    Bgeui { as_: u8, imm: u32, offset: i32 },
    Bany { as_: u8, at: u8, offset: i32 },
    Ball { as_: u8, at: u8, offset: i32 },
    Bnone { as_: u8, at: u8, offset: i32 },
    Bnall { as_: u8, at: u8, offset: i32 },
    Bbc  { as_: u8, at: u8, offset: i32 },
    Bbs  { as_: u8, at: u8, offset: i32 },
    Bbci { as_: u8, bit: u8, offset: i32 },
    Bbsi { as_: u8, bit: u8, offset: i32 },
    // -- Jumps and calls --
    J { offset: i32 },
    Jx { as_: u8 },
    Call0 { offset: i32 },
    Callx0 { as_: u8 },
    Call4 { offset: i32 }, Callx4 { as_: u8 },
    Call8 { offset: i32 }, Callx8 { as_: u8 },
    Call12 { offset: i32 }, Callx12 { as_: u8 },
    Ret,
    Retw,
    // -- Windowed-only --
    Entry { as_: u8, imm: u32 },
    Movsp { at: u8, as_: u8 },
    Rotw { n: i8 },
    S32e { at: u8, as_: u8, imm: u32 },
    L32e { at: u8, as_: u8, imm: u32 },
    Rfwo, Rfwu,
    // -- Exception/interrupt return --
    Rfe, Rfde,
    Rfi { level: u8 },
    // -- Atomic / memory-order --
    S32c1i { at: u8, as_: u8, imm: u32 },
    L32ai  { at: u8, as_: u8, imm: u32 },
    S32ri  { at: u8, as_: u8, imm: u32 },
    // -- MUL / DIV --
    Mull { ar: u8, as_: u8, at: u8 },
    Muluh { ar: u8, as_: u8, at: u8 },
    Mulsh { ar: u8, as_: u8, at: u8 },
    Quos { ar: u8, as_: u8, at: u8 },
    Quou { ar: u8, as_: u8, at: u8 },
    Rems { ar: u8, as_: u8, at: u8 },
    Remu { ar: u8, as_: u8, at: u8 },
    Mul16s { ar: u8, as_: u8, at: u8 },
    Mul16u { ar: u8, as_: u8, at: u8 },
    // -- Bit-manip --
    Nsa { ar: u8, as_: u8 },
    Nsau { ar: u8, as_: u8 },
    Min { ar: u8, as_: u8, at: u8 },
    Max { ar: u8, as_: u8, at: u8 },
    Minu { ar: u8, as_: u8, at: u8 },
    Maxu { ar: u8, as_: u8, at: u8 },
    Sext { ar: u8, as_: u8, t: u8 },
    Clamps { ar: u8, as_: u8, t: u8 },
    Addx2 { ar: u8, as_: u8, at: u8 },
    Addx4 { ar: u8, as_: u8, at: u8 },
    Addx8 { ar: u8, as_: u8, at: u8 },
    Subx2 { ar: u8, as_: u8, at: u8 },
    Subx4 { ar: u8, as_: u8, at: u8 },
    Subx8 { ar: u8, as_: u8, at: u8 },
    // -- CSR / SR --
    Rsr { at: u8, sr: u16 },
    Wsr { at: u8, sr: u16 },
    Xsr { at: u8, sr: u16 },
    Rur { ar: u8, ur: u16 },
    Wur { at: u8, ur: u16 },
    // -- Loop (stubbed; decoded so SRs latch) --
    Loop { as_: u8, offset: i32 },
    Loopnez { as_: u8, offset: i32 },
    Loopgtz { as_: u8, offset: i32 },
    // -- Misc --
    Nop,
    Break { imm_s: u8, imm_t: u8 },
    Syscall,
    Ill,
    Memw, Extw, Isync, Rsync, Esync, Dsync,
    Unknown(u32),
}

/// Decode a 24-bit (wide) instruction. High byte of the 32-bit fetch word is
/// ignored; caller must use [`xtensa_length`] first to confirm wideness.
pub fn decode(word: u32) -> Instruction {
    let w = word & 0x00FF_FFFF;
    let op0 = (w & 0x0F) as u8;
    match op0 {
        0x0 => decode_qrst(w),
        0x1 => decode_l32r(w),
        0x2 => decode_lsai(w),
        0x3 => decode_lsci(w),
        0x4 => decode_mac16(w),
        0x5 => decode_calln(w),
        0x6 => decode_si(w),
        0x7 => decode_b(w),
        _ => Instruction::Unknown(w),
    }
}

// Each `decode_*` is stubbed to `Unknown(w)` in this task; filled in by
// subsequent tasks B3..B8.
fn decode_qrst(w: u32) -> Instruction { Instruction::Unknown(w) }
fn decode_l32r(w: u32) -> Instruction { Instruction::Unknown(w) }
fn decode_lsai(w: u32) -> Instruction { Instruction::Unknown(w) }
fn decode_lsci(w: u32) -> Instruction { Instruction::Unknown(w) }
fn decode_mac16(w: u32) -> Instruction { Instruction::Unknown(w) }
fn decode_calln(w: u32) -> Instruction { Instruction::Unknown(w) }
fn decode_si(w: u32) -> Instruction { Instruction::Unknown(w) }
fn decode_b(w: u32) -> Instruction { Instruction::Unknown(w) }

impl fmt::Display for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self) // adequate for Plan 1; disassembly format later
    }
}
```

Create `crates/core/src/decoder/xtensa_narrow.rs`:
```rust
//! Xtensa Code Density (16-bit) decoder.
//!
//! Expands narrow encodings into the same `Instruction` enum from
//! `xtensa::Instruction` where semantics are identical, or uses a narrow-only
//! variant where they diverge.

use super::xtensa::Instruction;

/// Decode a 16-bit narrow instruction. Caller must have confirmed narrowness
/// via `xtensa_length::instruction_length(byte0) == 2`.
pub fn decode_narrow(halfword: u16) -> Instruction {
    let op0 = (halfword & 0x0F) as u8;
    match op0 {
        0x8 => Instruction::Unknown(halfword as u32), // L32I.N — filled in Task E
        0x9 => Instruction::Unknown(halfword as u32), // S32I.N
        0xA => Instruction::Unknown(halfword as u32), // ADD.N / ADDI.N
        0xD => Instruction::Unknown(halfword as u32), // MOV.N / MOVI.N / etc
        _ => Instruction::Unknown(halfword as u32),
    }
}
```

Register both in `decoder/mod.rs`:
```rust
pub mod xtensa;
pub mod xtensa_narrow;
```

- [ ] **Step 4: Run, expect PASS**

Run: `cargo test -p labwired-core --test xtensa_decode`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/core
git commit -m "feat(xtensa): Instruction enum skeleton and decode dispatch tree"
```

---

### Task B3: Decode RRR ALU ops (`op0 == 0`, QRST)

**Context:** `op0 == 0` routes into the QRST opcode group which subdivides by `op1` (bits [19:16]) and `op2` (bits [23:20]). This task covers the RRR ALU family under `op1 = 0x0..0x2` (ST0/ST1 — boolean, reg-reg arith, reg-reg logical). Xtensa ISA RM §8, "Instruction Reference / RRR."

Format RRR: `bits[23:20]=op2, bits[19:16]=op1, bits[15:12]=r, bits[11:8]=s, bits[7:4]=t, bits[3:0]=op0`.

- [ ] **Step 1: Write failing tests**

Append to `crates/core/tests/xtensa_decode.rs`:

```rust
fn rrr(op2: u32, op1: u32, r: u32, s: u32, t: u32) -> u32 {
    (op2 << 20) | (op1 << 16) | (r << 12) | (s << 8) | (t << 4) | 0x0
}

#[test]
fn decode_add() {
    // ADD ar, as_, at  →  op2=0x8, op1=0x0
    let w = rrr(0x8, 0x0, 3, 4, 5);
    assert_eq!(decode(w), Instruction::Add { ar: 3, as_: 4, at: 5 });
}

#[test]
fn decode_sub() {
    // SUB ar, as_, at  →  op2=0xC, op1=0x0
    let w = rrr(0xC, 0x0, 1, 2, 3);
    assert_eq!(decode(w), Instruction::Sub { ar: 1, as_: 2, at: 3 });
}

#[test]
fn decode_and_or_xor() {
    // AND: op2=0x1, op1=0x0
    assert_eq!(decode(rrr(0x1, 0x0, 7, 8, 9)), Instruction::And { ar: 7, as_: 8, at: 9 });
    // OR : op2=0x2, op1=0x0
    assert_eq!(decode(rrr(0x2, 0x0, 1, 1, 1)), Instruction::Or { ar: 1, as_: 1, at: 1 });
    // XOR: op2=0x3, op1=0x0
    assert_eq!(decode(rrr(0x3, 0x0, 1, 2, 3)), Instruction::Xor { ar: 1, as_: 2, at: 3 });
}

#[test]
fn decode_neg_abs() {
    // NEG ar, at — op2=0x6, op1=0x0, s == 0, t = at, r = ar
    assert_eq!(decode(rrr(0x6, 0x0, 5, 0, 4)), Instruction::Neg { ar: 5, at: 4 });
    // ABS — op2=0x6, op1=0x0, s == 1
    assert_eq!(decode(rrr(0x6, 0x0, 5, 1, 4)), Instruction::Abs { ar: 5, at: 4 });
}

#[test]
fn decode_addx_subx() {
    // ADDX2: op2=0x9, op1=0x0;  ADDX4: op2=0xA;  ADDX8: op2=0xB
    assert_eq!(decode(rrr(0x9, 0x0, 1, 2, 3)), Instruction::Addx2 { ar: 1, as_: 2, at: 3 });
    assert_eq!(decode(rrr(0xA, 0x0, 1, 2, 3)), Instruction::Addx4 { ar: 1, as_: 2, at: 3 });
    assert_eq!(decode(rrr(0xB, 0x0, 1, 2, 3)), Instruction::Addx8 { ar: 1, as_: 2, at: 3 });
    // SUBX2: op2=0xD; SUBX4: 0xE; SUBX8: 0xF
    assert_eq!(decode(rrr(0xD, 0x0, 1, 2, 3)), Instruction::Subx2 { ar: 1, as_: 2, at: 3 });
    assert_eq!(decode(rrr(0xE, 0x0, 1, 2, 3)), Instruction::Subx4 { ar: 1, as_: 2, at: 3 });
    assert_eq!(decode(rrr(0xF, 0x0, 1, 2, 3)), Instruction::Subx8 { ar: 1, as_: 2, at: 3 });
}
```

- [ ] **Step 2: Run, expect FAIL**

Run: `cargo test -p labwired-core --test xtensa_decode`
Expected: FAIL — RRR decode returns `Unknown`.

- [ ] **Step 3: Implement `decode_qrst`**

Replace the stub in `crates/core/src/decoder/xtensa.rs`:

```rust
fn decode_qrst(w: u32) -> Instruction {
    let op1 = ((w >> 16) & 0xF) as u8;
    let op2 = ((w >> 20) & 0xF) as u8;
    let r   = ((w >> 12) & 0xF) as u8;
    let s   = ((w >> 8)  & 0xF) as u8;
    let t   = ((w >> 4)  & 0xF) as u8;

    match op1 {
        0x0 => match op2 {
            0x0 => decode_st0(w, r, s, t),
            0x1 => Instruction::And { ar: r, as_: s, at: t },
            0x2 => Instruction::Or  { ar: r, as_: s, at: t },
            0x3 => Instruction::Xor { ar: r, as_: s, at: t },
            0x6 => match s {
                0x0 => Instruction::Neg { ar: r, at: t },
                0x1 => Instruction::Abs { ar: r, at: t },
                _ => Instruction::Unknown(w),
            },
            0x8 => Instruction::Add { ar: r, as_: s, at: t },
            0x9 => Instruction::Addx2 { ar: r, as_: s, at: t },
            0xA => Instruction::Addx4 { ar: r, as_: s, at: t },
            0xB => Instruction::Addx8 { ar: r, as_: s, at: t },
            0xC => Instruction::Sub  { ar: r, as_: s, at: t },
            0xD => Instruction::Subx2 { ar: r, as_: s, at: t },
            0xE => Instruction::Subx4 { ar: r, as_: s, at: t },
            0xF => Instruction::Subx8 { ar: r, as_: s, at: t },
            _ => Instruction::Unknown(w),
        },
        // op1 = 0x1, 0x2, 0x3 (shifts) — fill in Task B4.
        // op1 = 0x4..=0xF — fill in later tasks.
        _ => Instruction::Unknown(w),
    }
}

/// ST0 group — miscellaneous single-operand / zero-operand instructions.
fn decode_st0(w: u32, r: u8, s: u8, t: u8) -> Instruction {
    // Covers RET, RETW, JX, CALLX*, NOP, ISYNC/RSYNC/ESYNC/DSYNC, MEMW/EXTW,
    // RSR/WSR/XSR (no — those are ST1), RFE/RFDE/RFI, BREAK, SYSCALL.
    // This task implements only what's tested above plus NOP / BREAK;
    // the rest are stubbed as Unknown and filled in later tasks.
    match r {
        0x0 => match s {
            0x0 => match t {
                0x0 => Instruction::Isync,
                0x1 => Instruction::Rsync,
                0x2 => Instruction::Esync,
                0x3 => Instruction::Dsync,
                0xC => Instruction::Memw,
                0xD => Instruction::Extw,
                0xF => Instruction::Nop,
                _ => Instruction::Unknown(w),
            },
            _ => Instruction::Unknown(w),
        },
        0x4 => Instruction::Break { imm_s: s, imm_t: t },
        _ => Instruction::Unknown(w),
    }
}
```

- [ ] **Step 4: Run tests, expect PASS**

Run: `cargo test -p labwired-core --test xtensa_decode`
Expected: PASS (all ALU-RRR tests pass).

- [ ] **Step 5: Commit**

```bash
git add crates/core
git commit -m "feat(xtensa): decode RRR ALU + ADDX/SUBX + NOP/BREAK/sync"
```

---

### Task B4: Decode shifts (RRR, `op1 = 0x1..=0x3`)

Shifts: `SLL (op2=0xA, op1=0x1)`, `SRL (op2=0x9, op1=0x1)`, `SRA (op2=0xB, op1=0x1)`, `SRC (op2=0x8, op1=0x1)`, `SLLI (op2=0x0..=0x1, op1=0x1 — sa field split)`, `SRLI (op2=0x4, op1=0x1)`, `SRAI (op2=0x2..=0x3, op1=0x1)`, `SSL/SSR/SSA8L/SSA8B (op2=0x4, op1=0x0 w/ specific t encoding)`, `SSAI (op2=0x4, op1=0x0, t=0x4)`.

Rather than retranscribe semantics, refer to Xtensa ISA RM §7.1 table of ST3 (shift) opcodes. Decode only what the MVP needs.

Follow the same TDD pattern: write tests for `Sll`, `Sra`, `Src`, `Slli (shamt=5)`, `Srli (shamt=7)`, `Srai (shamt=3)`, `Ssl {as_: 3}`, `Ssai {shamt: 9}`. Then extend `decode_qrst` with:

- [ ] **Step 1:** Add shift tests (one per instruction variant listed) to `xtensa_decode.rs`.
- [ ] **Step 2:** Run — FAIL.
- [ ] **Step 3:** Implement decode branches for `op1 = 0x1` covering the above opcodes. For `SLLI`, note that the 5-bit shift amount is split: `shamt = ((op2 & 0x1) << 4) | t` (op2 values 0x0 and 0x1 both map to SLLI, with op2[0] being the high bit of shamt).
- [ ] **Step 4:** PASS.
- [ ] **Step 5:** Commit `feat(xtensa): decode shift instructions (SLL/SRL/SRA/SRC/SLLI/SRLI/SRAI/SSL/SSR/SSAI)`.

Exact shamt-splitting table (write into the decoder):
```
SLLI: op2 ∈ {0x0, 0x1}, shamt = (((op2 & 1) << 4) | t) XOR-not-required: treat as 32 - raw when raw >= 16 per ISA RM
SRAI: op2 ∈ {0x2, 0x3}, shamt = ((op2 & 1) << 4) | t
SRLI: op2 = 0x4, shamt = t  (only low 4 bits; ISA limits SRLI to 0..15)
SRC : op2 = 0x8, r=ar, s=as_, t=at
SRL : op2 = 0x9, r=ar, s=0, t=at
SLL : op2 = 0xA, r=ar, s=as_, t=0
SRA : op2 = 0xB, r=ar, s=0, t=at
```

(Do NOT skip this table — transcribe it exactly into the decoder and a test case per row.)

---

### Task B5: Decode `L32R` (`op0 = 0x1`)

`L32R at, label` — PC-relative literal load, bits [23:8] form a 16-bit negative offset (word-aligned): `offset = -(4 + (((word >> 8) & 0xFFFF) ^ 0xFFFF)*4)`; simplified: `addr = ((pc + 3) & ~3) + (sign_extend_16(word[23:8] as signed) << 2)`.

Authoritative semantics in Xtensa ISA RM §8 "L32R."

- [ ] **Step 1: Test**

```rust
#[test]
fn decode_l32r() {
    // at=3, imm16 = 0xFFFE => offset = -2*4 = -8 bytes
    let w = 0x0001u32 | (3u32 << 4) | (0xFFFEu32 << 8);
    match decode(w) {
        Instruction::L32r { at, pc_rel_byte_offset } => {
            assert_eq!(at, 3);
            // Offset is computed relative to PC; decoder stores the raw sign-extended
            // word-offset in bytes for the exec phase to apply.
            assert_eq!(pc_rel_byte_offset, -8);
        }
        other => panic!("expected L32R, got {:?}", other),
    }
}
```

- [ ] **Step 2:** Run, FAIL.
- [ ] **Step 3:** Fill `decode_l32r`:
```rust
fn decode_l32r(w: u32) -> Instruction {
    let at = ((w >> 4) & 0xF) as u8;
    let imm16 = (w >> 8) & 0xFFFF;
    // Sign-extend 16-bit value (2's complement), treat as word-offset (×4).
    let sext = ((imm16 ^ 0x8000).wrapping_sub(0x8000)) as i32;
    let pc_rel_byte_offset = sext * 4;
    Instruction::L32r { at, pc_rel_byte_offset }
}
```
- [ ] **Step 4:** PASS.
- [ ] **Step 5:** Commit.

---

### Task B6: Decode RRI8 loads/stores (`op0 = 0x2`, LSAI)

RRI8 format: `bits[23:16]=imm8, bits[15:12]=r (sub-opcode), bits[11:8]=s (as_), bits[7:4]=t (at), bits[3:0]=op0`.

LSAI sub-opcodes by `r`:
```
r = 0x0  L8UI   — imm shift 0
r = 0x1  L16UI  — imm shift 1 (word) nope: actually imm shift 1 means ×2
r = 0x2  L32I   — imm shift 2
r = 0x4  S8I    — imm shift 0
r = 0x5  S16I   — imm shift 1
r = 0x6  S32I   — imm shift 2
r = 0x9  L16SI  — imm shift 1
r = 0x7  CACHEATTR-like ops — skip in MVP
r = 0xB  L32AI  — imm shift 2 (atomic load)
r = 0xE  S32C1I — imm shift 2 (compare-and-swap)
r = 0xF  S32RI  — imm shift 2 (release-sync store)
```

Effective address: `as_ + (imm8 << shift)`.

- [ ] **Step 1:** Write tests for L8UI, L32I, S32I, L16UI, L16SI, S8I, S16I, L32AI, S32C1I, S32RI. Use representative `imm8` values.

Example:
```rust
fn rri8(r: u32, s: u32, t: u32, imm8: u32) -> u32 {
    0x2 | (t << 4) | (s << 8) | (r << 12) | ((imm8 & 0xFF) << 16)
}
#[test]
fn decode_l32i() {
    let w = rri8(0x2, 4, 5, 0x10); // L32I at=5, as=4, imm = 0x10 << 2 = 0x40
    assert_eq!(decode(w), Instruction::L32i { at: 5, as_: 4, imm: 0x40 });
}
```

- [ ] **Step 2:** Run, FAIL.
- [ ] **Step 3:** Implement:
```rust
fn decode_lsai(w: u32) -> Instruction {
    let imm8 = ((w >> 16) & 0xFF) as u32;
    let r    = ((w >> 12) & 0xF) as u8;
    let s    = ((w >> 8)  & 0xF) as u8;
    let t    = ((w >> 4)  & 0xF) as u8;
    let shift = |k| imm8 << k;
    match r {
        0x0 => Instruction::L8ui  { at: t, as_: s, imm: shift(0) },
        0x1 => Instruction::L16ui { at: t, as_: s, imm: shift(1) },
        0x2 => Instruction::L32i  { at: t, as_: s, imm: shift(2) },
        0x4 => Instruction::S8i   { at: t, as_: s, imm: shift(0) },
        0x5 => Instruction::S16i  { at: t, as_: s, imm: shift(1) },
        0x6 => Instruction::S32i  { at: t, as_: s, imm: shift(2) },
        0x9 => Instruction::L16si { at: t, as_: s, imm: shift(1) },
        0xB => Instruction::L32ai { at: t, as_: s, imm: shift(2) },
        0xE => Instruction::S32c1i { at: t, as_: s, imm: shift(2) },
        0xF => Instruction::S32ri  { at: t, as_: s, imm: shift(2) },
        0xC => { // ADDI family: sub-encoded with op2
            // ADDI is under LSAI when imm shift is 0; uses RRI8 similarly.
            // Per ISA RM, ADDI uses op0=0x2, r=0xC, imm8 sign-extended.
            let imm = ((imm8 ^ 0x80).wrapping_sub(0x80)) as i32;
            Instruction::Addi { at: t, as_: s, imm8: imm }
        }
        0xD => { // ADDMI
            let imm = (((imm8 ^ 0x80).wrapping_sub(0x80)) as i32) << 8;
            Instruction::Addmi { at: t, as_: s, imm }
        }
        _ => Instruction::Unknown(w),
    }
}
```

- [ ] **Step 4:** PASS.
- [ ] **Step 5:** Commit.

---

### Task B7: Decode branch family (`op0 = 0x7`, BRI8/BRI12; and `op0 = 0x6`, SI)

Branch formats are extensive. Refer to Xtensa ISA RM §8, "Branch Format." For the MVP we must decode:

- BR (R-type, op0=0x7): BEQ, BNE, BLT, BGE, BLTU, BGEU, BANY, BALL, BNONE, BNALL, BBC, BBS.
- BRI8 (op0=0x6): BEQI, BNEI, BLTI, BGEI, BLTUI, BGEUI, BEQZ, BNEZ, BLTZ, BGEZ; plus BBCI/BBSI (sub-form).
- BRI12 (op0=0x6, special): BEQZ, BNEZ, BLTZ, BGEZ with 12-bit signed offset.
- SI / J: JX under ST0 already; J (jump) under op0=0x6 special.

Rather than cram this into one code block, follow the TDD flow tighter: write one test, one decode arm, one pass, repeat, then commit once the whole family is covered. Build the decoder in `crates/core/src/decoder/xtensa.rs`:

```rust
fn decode_b(w: u32) -> Instruction {
    let op2 = ((w >> 20) & 0xF) as u8; // sub-opcode
    let r   = ((w >> 12) & 0xF) as u8;
    let s   = ((w >> 8)  & 0xF) as u8;
    let t   = ((w >> 4)  & 0xF) as u8;
    let imm8 = ((w >> 16) & 0xFF) as u32;
    let offset8 = ((imm8 ^ 0x80).wrapping_sub(0x80)) as i32;

    // op2 selects comparison. Table from ISA RM §8:
    match op2 {
        0x0 => Instruction::Bnone { as_: s, at: t, offset: offset8 + 4 },
        0x1 => Instruction::Beq   { as_: s, at: t, offset: offset8 + 4 },
        0x2 => Instruction::Blt   { as_: s, at: t, offset: offset8 + 4 },
        0x3 => Instruction::Bltu  { as_: s, at: t, offset: offset8 + 4 },
        0x4 => Instruction::Ball  { as_: s, at: t, offset: offset8 + 4 },
        0x5 => Instruction::Bbc   { as_: s, at: t, offset: offset8 + 4 },
        0x6 | 0x7 => Instruction::Bbci { as_: s, bit: (r & 0xF) | ((op2 & 0x1) << 4), offset: offset8 + 4 },
        0x8 => Instruction::Bany  { as_: s, at: t, offset: offset8 + 4 },
        0x9 => Instruction::Bne   { as_: s, at: t, offset: offset8 + 4 },
        0xA => Instruction::Bge   { as_: s, at: t, offset: offset8 + 4 },
        0xB => Instruction::Bgeu  { as_: s, at: t, offset: offset8 + 4 },
        0xC => Instruction::Bnall { as_: s, at: t, offset: offset8 + 4 },
        0xD => Instruction::Bbs   { as_: s, at: t, offset: offset8 + 4 },
        0xE | 0xF => Instruction::Bbsi { as_: s, bit: (r & 0xF) | ((op2 & 0x1) << 4), offset: offset8 + 4 },
        _ => Instruction::Unknown(w),
    }
}

fn decode_si(w: u32) -> Instruction {
    // op0 = 0x6 covers J, BZ (BEQZ/BNEZ/BLTZ/BGEZ with 12-bit imm), BI (BEQI etc with 8-bit imm).
    // Bits [7:6] select n (0..3), [5:4] select m (0..3), [3:0] = op0 = 0x6.
    let n = ((w >> 4) & 0x3) as u8;
    let m = ((w >> 6) & 0x3) as u8;
    let s = ((w >> 8) & 0xF) as u8;
    let imm12 = (w >> 12) & 0xFFF;
    let offset12 = ((imm12 ^ 0x800).wrapping_sub(0x800)) as i32;
    let imm8 = ((w >> 16) & 0xFF) as u32;
    let offset8 = ((imm8 ^ 0x80).wrapping_sub(0x80)) as i32;

    match m {
        0 => match n {
            0 => {
                // J: imm18 = bits [23:6]; 18-bit signed offset.
                let imm18 = (w >> 6) & 0x3_FFFF;
                let off = ((imm18 ^ 0x2_0000).wrapping_sub(0x2_0000)) as i32;
                Instruction::J { offset: off + 4 }
            }
            1 => Instruction::Unknown(w), // reserved for extensions
            2 => match s {
                0x0 => Instruction::Beqz { as_: s, offset: offset12 + 4 },
                0x1 => Instruction::Bnez { as_: s, offset: offset12 + 4 },
                _ => Instruction::Unknown(w),
            },
            3 => match s {
                0x0 => Instruction::Bltz { as_: s, offset: offset12 + 4 },
                0x1 => Instruction::Bgez { as_: s, offset: offset12 + 4 },
                _ => Instruction::Unknown(w),
            },
            _ => Instruction::Unknown(w),
        },
        1 => match n {
            // BI: BEQI/BNEI/BLTI/BGEI with imm8; imm range special-encoded via b4const table.
            // This is spec-heavy: implement with a helper `b4const(r)` and `b4constu(r)`.
            _ => decode_bi(w, n, s, imm8 as i32, offset8),
        },
        2 => match n {
            _ => decode_bi_u(w, n, s, imm8 as u32, offset8),
        },
        _ => Instruction::Unknown(w),
    }
}

// Per Xtensa ISA RM Appendix "B4CONST" table:
fn b4const(r: u8) -> i32 {
    match r {
        0 => -1, 1 => 1, 2 => 2, 3 => 3, 4 => 4, 5 => 5, 6 => 6, 7 => 7,
        8 => 8, 9 => 10, 10 => 12, 11 => 16, 12 => 32, 13 => 64, 14 => 128, 15 => 256,
        _ => unreachable!(),
    }
}
fn b4constu(r: u8) -> u32 {
    match r {
        0 => 32768, 1 => 65536, 2 => 2, 3 => 3, 4 => 4, 5 => 5, 6 => 6, 7 => 7,
        8 => 8, 9 => 10, 10 => 12, 11 => 16, 12 => 32, 13 => 64, 14 => 128, 15 => 256,
        _ => unreachable!(),
    }
}

fn decode_bi(w: u32, n: u8, s: u8, _imm8: i32, offset: i32) -> Instruction {
    let r = ((w >> 12) & 0xF) as u8;
    match n {
        0 => Instruction::Beqi { as_: s, imm: b4const(r), offset: offset + 4 },
        1 => Instruction::Bnei { as_: s, imm: b4const(r), offset: offset + 4 },
        2 => Instruction::Blti { as_: s, imm: b4const(r), offset: offset + 4 },
        3 => Instruction::Bgei { as_: s, imm: b4const(r), offset: offset + 4 },
        _ => Instruction::Unknown(w),
    }
}
fn decode_bi_u(w: u32, n: u8, s: u8, _imm8: u32, offset: i32) -> Instruction {
    let r = ((w >> 12) & 0xF) as u8;
    match n {
        0 => Instruction::Bltui { as_: s, imm: b4constu(r), offset: offset + 4 },
        1 => Instruction::Bgeui { as_: s, imm: b4constu(r), offset: offset + 4 },
        _ => Instruction::Unknown(w),
    }
}
```

- [ ] **Step 1:** Write tests for each branch family member listed above (at least 18 tests).
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Implement as shown.
- [ ] **Step 4:** PASS.
- [ ] **Step 5:** Commit `feat(xtensa): decode branch family + B4CONST/B4CONSTU tables + J`.

---

### Task B8: Decode CALL / CALLX / RET / windowed variants

Format: `CALLn` uses `op0 = 0x5` with `n` = `bits[5:4]`. Offset is 18 bits (`bits[23:6]`), sign-extended, shifted left 2, PC-relative to word-aligned (PC & ~3).

`CALLX0/4/8/12` under ST0 group (`op0 = 0x0, op1 = 0x0, op2 = 0x0, r = 0x0`), with `m` field in `bits[7:6]` selecting n.

`RET` = `CALLX0 a0` (no operand). `RETW` = windowed return.

- [ ] **Step 1:** Write tests for CALL0, CALL4, CALL8, CALL12, CALLX0, CALLX4, CALLX8, CALLX12, RET, RETW.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Extend `decode_calln`:
```rust
fn decode_calln(w: u32) -> Instruction {
    let n = ((w >> 4) & 0x3) as u8;
    let imm18 = (w >> 6) & 0x3_FFFF;
    let off = ((imm18 ^ 0x2_0000).wrapping_sub(0x2_0000)) as i32;
    match n {
        0 => Instruction::Call0  { offset: off * 4 },
        1 => Instruction::Call4  { offset: off * 4 },
        2 => Instruction::Call8  { offset: off * 4 },
        3 => Instruction::Call12 { offset: off * 4 },
        _ => unreachable!(),
    }
}
```
And extend `decode_st0` to handle CALLX/RET under `r=0x0, s=0x0, t` encoding: a `CALLX0` is `op0=0, op1=0, op2=0, r=0, s=<as>, t=0`; RET is `CALLX0 a0` with `s=0`.

Exact ST0 encoding table (transcribe):
```
op2  op1  r  s  t      instruction
0    0    0  0  0..3  ret/retw/jx per t
0    0    0  s  0     callx0 as=s (when s>0)
0    0    0  s  1     callx4 as=s
0    0    0  s  2     callx8 as=s
0    0    0  s  3     callx12 as=s
```
Actually the Xtensa ST0 encoding nests by `op1`. Refer to the ISA RM §8 opcode table and encode exactly per the table. Do NOT improvise.

- [ ] **Step 4:** PASS.
- [ ] **Step 5:** Commit.

---

## PHASE C — CPU state

Goal: data structures for register file, PS, SRs, and fetch loop. No exec yet.

### Task C1: AR register file with windowing

**File:** `crates/core/src/cpu/xtensa_regs.rs`

- [ ] **Step 1: Write failing tests**

Create `crates/core/tests/xtensa_regs.rs`:
```rust
use labwired_core::cpu::xtensa_regs::{ArFile, Ps};

#[test]
fn logical_a0_maps_to_physical_0_when_windowbase_zero() {
    let mut f = ArFile::new();
    f.set_windowbase(0);
    f.write_logical(0, 0xDEAD_BEEF);
    assert_eq!(f.read_logical(0), 0xDEAD_BEEF);
    assert_eq!(f.physical(0), 0xDEAD_BEEF);
}

#[test]
fn windowbase_rotation_shifts_by_four() {
    let mut f = ArFile::new();
    f.set_windowbase(1); // logical a0 → physical 4
    f.write_logical(0, 0x1111_2222);
    assert_eq!(f.physical(4), 0x1111_2222);
    f.set_windowbase(0);
    assert_eq!(f.read_logical(4), 0x1111_2222);
}

#[test]
fn logical_index_15_is_valid() {
    let mut f = ArFile::new();
    f.set_windowbase(5);
    f.write_logical(15, 0xAAAA);
    assert_eq!(f.physical((5 * 4 + 15) % 64), 0xAAAA);
}

#[test]
fn windowstart_bit_tracks_allocated_frames() {
    let mut f = ArFile::new();
    f.set_windowstart(0);
    f.set_windowstart_bit(3, true);
    assert!(f.windowstart_bit(3));
    f.set_windowstart_bit(3, false);
    assert!(!f.windowstart_bit(3));
}

#[test]
fn ps_fielded_readback() {
    let mut ps = Ps::from_raw(0);
    ps.set_intlevel(5);
    ps.set_excm(true);
    ps.set_woe(true);
    assert_eq!(ps.intlevel(), 5);
    assert!(ps.excm());
    assert!(ps.woe());
    let raw = ps.as_raw();
    let ps2 = Ps::from_raw(raw);
    assert_eq!(ps2.intlevel(), 5);
    assert!(ps2.excm());
    assert!(ps2.woe());
}
```

- [ ] **Step 2:** FAIL — module not present.
- [ ] **Step 3:** Implement `crates/core/src/cpu/xtensa_regs.rs`:

```rust
//! Xtensa AR register file with Windowed Registers Option + PS struct.

/// 64-entry physical AR file indexed via WindowBase. Logical registers
/// a0..a15 map to physical[(WindowBase*4 + idx) mod 64].
#[derive(Debug, Clone)]
pub struct ArFile {
    phys: [u32; 64],
    window_base: u8,   // 0..15
    window_start: u16, // 16 bits
}

impl Default for ArFile { fn default() -> Self { Self::new() } }

impl ArFile {
    pub fn new() -> Self {
        let mut ws = 0u16;
        ws |= 1; // bit 0 set at reset — a0..a3 frame exists
        Self { phys: [0; 64], window_base: 0, window_start: ws }
    }

    pub fn windowbase(&self) -> u8 { self.window_base }
    pub fn set_windowbase(&mut self, v: u8) { self.window_base = v & 0x0F; }

    pub fn windowstart(&self) -> u16 { self.window_start }
    pub fn set_windowstart(&mut self, v: u16) { self.window_start = v; }
    pub fn windowstart_bit(&self, idx: u8) -> bool { (self.window_start >> (idx & 0xF)) & 1 == 1 }
    pub fn set_windowstart_bit(&mut self, idx: u8, v: bool) {
        let b = idx & 0xF;
        if v { self.window_start |= 1 << b; } else { self.window_start &= !(1 << b); }
    }

    pub fn physical(&self, phys_idx: usize) -> u32 { self.phys[phys_idx & 63] }
    pub fn set_physical(&mut self, phys_idx: usize, v: u32) { self.phys[phys_idx & 63] = v; }

    #[inline]
    fn logical_to_physical(&self, logical: u8) -> usize {
        ((self.window_base as usize * 4) + logical as usize) & 63
    }

    pub fn read_logical(&self, logical: u8) -> u32 {
        self.phys[self.logical_to_physical(logical & 0xF)]
    }
    pub fn write_logical(&mut self, logical: u8, v: u32) {
        let p = self.logical_to_physical(logical & 0xF);
        self.phys[p] = v;
    }
}

/// Processor State (PS) fielded.
#[derive(Debug, Clone, Copy)]
pub struct Ps(u32);

impl Ps {
    pub fn from_raw(raw: u32) -> Self { Self(raw) }
    pub fn as_raw(self) -> u32 { self.0 }

    #[inline] pub fn intlevel(self) -> u8 { (self.0 & 0xF) as u8 }
    #[inline] pub fn set_intlevel(&mut self, v: u8) { self.0 = (self.0 & !0xF) | (v as u32 & 0xF); }

    #[inline] pub fn excm(self) -> bool { (self.0 >> 4) & 1 == 1 }
    #[inline] pub fn set_excm(&mut self, v: bool) { if v { self.0 |= 1 << 4 } else { self.0 &= !(1 << 4) } }

    #[inline] pub fn ring(self) -> u8 { ((self.0 >> 6) & 0x3) as u8 }
    #[inline] pub fn set_ring(&mut self, v: u8) { self.0 = (self.0 & !(0x3 << 6)) | ((v as u32 & 0x3) << 6); }

    #[inline] pub fn owb(self) -> u8 { ((self.0 >> 8) & 0xF) as u8 }
    #[inline] pub fn set_owb(&mut self, v: u8) { self.0 = (self.0 & !(0xF << 8)) | ((v as u32 & 0xF) << 8); }

    #[inline] pub fn callinc(self) -> u8 { ((self.0 >> 16) & 0x3) as u8 }
    #[inline] pub fn set_callinc(&mut self, v: u8) { self.0 = (self.0 & !(0x3 << 16)) | ((v as u32 & 0x3) << 16); }

    #[inline] pub fn woe(self) -> bool { (self.0 >> 18) & 1 == 1 }
    #[inline] pub fn set_woe(&mut self, v: bool) { if v { self.0 |= 1 << 18 } else { self.0 &= !(1 << 18) } }
}
```

- [ ] **Step 4:** PASS.
- [ ] **Step 5:** Commit `feat(xtensa): AR register file with windowing + PS fielded struct`.

---

### Task C2: Special Register table

Per spec §5.3 SR list. Implement `XtensaSrFile` with `read(sr_id)`, `write(sr_id, v)`, `swap(sr_id, v)` (XSR).

- [ ] **Step 1:** Tests in `crates/core/tests/xtensa_regs.rs` for each SR:
  - EPC1..6, EPS2..6, EXCSAVE1..6, EXCCAUSE, EXCVADDR, DEPC, INTERRUPT (read-only latch), INTENABLE, INTCLEAR (write-clears bits in INTERRUPT), SAR, LBEG, LEND, LCOUNT, VECBASE, CPENABLE, THREADPTR, CCOUNT, CCOMPARE0/1/2, SCOMPARE1, LITBASE, M0..M3/ACCLO/ACCHI (stubs — any value survives roundtrip), FCR, FSR.

Target count: ~35 SR cases. Write one test per SR.

- [ ] **Step 2..5:** Standard TDD cycle. Implement `crates/core/src/cpu/xtensa_sr.rs`.

Key behavior details (must be encoded in tests + impl):
- `INTCLEAR` is write-only; writing a 1 bit clears the corresponding bit in `INTERRUPT`.
- `INTERRUPT` is read-only from software; set by hardware when peripherals raise IRQs.
- `LITBASE` writes succeed but exec ignores (ESP32-S3 hardwires LITBASE=0).
- `CCOUNT` monotonic: software writes accepted but engine tick overwrites.

Commit: `feat(xtensa): Special Register table with RSR/WSR/XSR dispatch`.

---

### Task C3: Integrate `XtensaLx7` into `Cpu` trait

**File:** `crates/core/src/cpu/xtensa_lx7.rs`

Create the `XtensaLx7` struct holding `ArFile`, `Ps`, `SrFile`, `pc: u32`, and provide a stub `Cpu` trait impl where `step` is unimplemented (just returns `Ok(())` after incrementing PC by 3 to keep forward progress during unit tests). Full implementation comes in Phase D+.

- [ ] **Step 1:** Test skeleton — verify `XtensaLx7::reset` sets sane initial state.
```rust
#[test]
fn xtensa_lx7_reset_initial_state() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    assert_eq!(cpu.get_pc(), 0x4000_0400);
    assert_eq!(cpu.ps.intlevel(), 0);
    assert!(cpu.ps.excm());
    assert_eq!(cpu.regs.windowbase(), 0);
    assert_eq!(cpu.regs.windowstart(), 0x1);
    assert_eq!(cpu.sr.read(VECBASE_SR_ID), 0x4000_0000);
}
```
- [ ] **Step 2..5:** Standard TDD. Commit: `feat(xtensa): XtensaLx7 struct, Cpu trait stub, reset state`.

---

### Task C4: Fetch loop skeleton

Fetch from PC, length-predecode, dispatch to wide or narrow decode, advance PC. Execute returns `NotImplemented` (stub). Hook into `Cpu::step`.

- [ ] **Step 1:** Test that `step` with a dummy ADD in memory decodes + advances PC by 3:
```rust
#[test]
fn step_decodes_and_advances_pc() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    // Place ADD a3, a4, a5 at PC
    let add = 0x00_00_85_30u32; // op2=8, op1=0, r=3, s=4, t=5, op0=0
    bus.write_u32(0x4000_0400, add).unwrap();
    cpu.reset(&mut bus).unwrap();
    // Step raises NotImplementedExec error after decoding (exec stubbed).
    let err = cpu.step(&mut bus, &[]).unwrap_err();
    assert!(matches!(err, SimulationError::NotImplemented(_)));
    // PC should NOT advance when exec fails. If this is the wrong policy, revise.
}
```

- [ ] **Step 2..5:** Implement, test, commit. Exec dispatch body is:
```rust
fn step(&mut self, bus: &mut dyn Bus, _observers: &[Arc<dyn SimulationObserver>]) -> SimResult<()> {
    let pc = self.pc;
    let b0 = bus.read_u8(pc as u64)?;
    let len = xtensa_length::instruction_length(b0);
    let ins = if len == 2 {
        let hw = bus.read_u16(pc as u64)?;
        xtensa_narrow::decode_narrow(hw)
    } else {
        let w = bus.read_u32(pc as u64)?;
        xtensa::decode(w)
    };
    self.execute(ins, bus, len)
}
```
Commit: `feat(xtensa): fetch loop skeleton with length predecode + decode dispatch`.

---

## PHASE D — Base integer exec

Goal: execute every RRR ALU, shift, load/store, L32R, branch, jump, and narrow op. Each task writes exec for one semantic family, tests with a hand-crafted program, commits.

### Task D1: ALU reg-reg exec (ADD/SUB/AND/OR/XOR/NEG/ABS/ADDX*/SUBX*)

- [ ] **Step 1:** Test — place `MOVI a2, 5; MOVI a3, 7; ADD a4, a2, a3; BREAK 1,15` in memory; run until BREAK; assert `a4 == 12`.
- [ ] **Step 2:** FAIL (exec not implemented).
- [ ] **Step 3:** Implement `execute(Instruction::Add {..})` and peers in `cpu/xtensa_lx7.rs`:
```rust
fn execute(&mut self, ins: Instruction, bus: &mut dyn Bus, len: u32) -> SimResult<()> {
    use Instruction::*;
    match ins {
        Add { ar, as_, at } => {
            let v = self.regs.read_logical(as_).wrapping_add(self.regs.read_logical(at));
            self.regs.write_logical(ar, v);
            self.pc = self.pc.wrapping_add(len);
        }
        Sub { ar, as_, at } => {
            let v = self.regs.read_logical(as_).wrapping_sub(self.regs.read_logical(at));
            self.regs.write_logical(ar, v);
            self.pc = self.pc.wrapping_add(len);
        }
        And { ar, as_, at } => { let v = self.regs.read_logical(as_) & self.regs.read_logical(at); self.regs.write_logical(ar, v); self.pc = self.pc.wrapping_add(len); }
        Or  { ar, as_, at } => { let v = self.regs.read_logical(as_) | self.regs.read_logical(at); self.regs.write_logical(ar, v); self.pc = self.pc.wrapping_add(len); }
        Xor { ar, as_, at } => { let v = self.regs.read_logical(as_) ^ self.regs.read_logical(at); self.regs.write_logical(ar, v); self.pc = self.pc.wrapping_add(len); }
        Neg { ar, at } => { let v = 0u32.wrapping_sub(self.regs.read_logical(at)); self.regs.write_logical(ar, v); self.pc = self.pc.wrapping_add(len); }
        Abs { ar, at } => {
            let x = self.regs.read_logical(at) as i32;
            self.regs.write_logical(ar, x.unsigned_abs());
            self.pc = self.pc.wrapping_add(len);
        }
        Addx2 { ar, as_, at } => { let v = (self.regs.read_logical(as_) << 1).wrapping_add(self.regs.read_logical(at)); self.regs.write_logical(ar, v); self.pc = self.pc.wrapping_add(len); }
        Addx4 { ar, as_, at } => { let v = (self.regs.read_logical(as_) << 2).wrapping_add(self.regs.read_logical(at)); self.regs.write_logical(ar, v); self.pc = self.pc.wrapping_add(len); }
        Addx8 { ar, as_, at } => { let v = (self.regs.read_logical(as_) << 3).wrapping_add(self.regs.read_logical(at)); self.regs.write_logical(ar, v); self.pc = self.pc.wrapping_add(len); }
        Subx2 { ar, as_, at } => { let v = (self.regs.read_logical(as_) << 1).wrapping_sub(self.regs.read_logical(at)); self.regs.write_logical(ar, v); self.pc = self.pc.wrapping_add(len); }
        Subx4 { ar, as_, at } => { let v = (self.regs.read_logical(as_) << 2).wrapping_sub(self.regs.read_logical(at)); self.regs.write_logical(ar, v); self.pc = self.pc.wrapping_add(len); }
        Subx8 { ar, as_, at } => { let v = (self.regs.read_logical(as_) << 3).wrapping_sub(self.regs.read_logical(at)); self.regs.write_logical(ar, v); self.pc = self.pc.wrapping_add(len); }
        Movi { at, imm } => { self.regs.write_logical(at, imm as u32); self.pc = self.pc.wrapping_add(len); }
        Break { .. } => {
            // Raise breakpoint exception (exec halts here).
            return Err(SimulationError::BreakpointHit(self.pc));
        }
        Nop | Memw | Extw | Isync | Rsync | Esync | Dsync => { self.pc = self.pc.wrapping_add(len); }
        _ => return Err(SimulationError::NotImplemented(format!("exec: {:?}", ins))),
    }
    Ok(())
}
```
(Extend `SimulationError` with `BreakpointHit(u32)` if absent.)

- [ ] **Step 4:** PASS.
- [ ] **Step 5:** Commit `feat(xtensa): exec for ALU reg-reg + ADDX/SUBX + NOP + BREAK`.

---

### Task D2: Shift exec

Implement SLL, SRL, SRA, SRC, SLLI, SRLI, SRAI, SSL, SSR, SSA8L, SSA8B, SSAI. SAR is a 6-bit SR set by SSL/SSR/SSAI.

Same TDD pattern:
- [ ] Tests for each shift variant (at minimum 1 per).
- [ ] FAIL → implement → PASS → commit `feat(xtensa): exec shifts + SAR handling`.

Key semantics:
- `SLL ar, as` : `ar = as_ << (32 - SAR)`.
- `SRL ar, at` : `ar = at >> SAR`.
- `SRA ar, at` : `ar = (at as i32) >> SAR`.
- `SRC ar, as_, at` : concatenate `(as_ : at)` into 64-bit, shift right by SAR, take low 32 — used for bit-string extract.
- `SSL as_` : `SAR = 32 - (as_ & 0x1F)`.
- `SSR as_` : `SAR = as_ & 0x1F`.
- `SSAI shamt` : `SAR = shamt & 0x1F`.
- `SSA8L/SSA8B as_` : `SAR = (as_ & 3) * 8` with endian flip for 8B.

---

### Task D3: ADDI / ADDMI / MOVI immediate exec

- [ ] Tests (3–6). Commit `feat(xtensa): exec ADDI/ADDMI/MOVI with sign-extension`.

---

### Task D4: Load exec (L8UI, L16UI, L16SI, L32I, L32R)

- [ ] Tests: place a word at RAM, load with each variant, verify sign-extension.
- [ ] Exec: compute EA = `as_ + imm`; call `bus.read_u8/u16/u32`; sign-extend L16SI.
- [ ] For L32R, compute `addr = ((pc + 3) & !3) + pc_rel_byte_offset`, then `bus.read_u32`.
- [ ] PASS → Commit `feat(xtensa): exec loads L8UI/L16UI/L16SI/L32I/L32R`.

---

### Task D5: Store exec (S8I, S16I, S32I)

- [ ] Tests: write reg to mem, read back, verify.
- [ ] Exec: EA = `as_ + imm`; `bus.write_u8/u16/u32`.
- [ ] PASS → Commit.

---

### Task D6: Branch exec

Each branch: compute condition, if true `pc = pc + offset` (note offsets are stored pre-advance in decoder; `pc + offset + 4` semantics already baked in decoder via `+ 4` addition — verify in Task B7 decoder tests). If false, `pc += len`.

- [ ] Tests: taken + not-taken for each of BEQ, BNE, BLT, BGE, BLTU, BGEU, BANY, BALL, BNONE, BNALL, BBC, BBS, BBCI, BBSI, BEQZ, BNEZ, BLTZ, BGEZ, BEQI, BNEI, BLTI, BGEI, BLTUI, BGEUI. ~24 tests minimum.
- [ ] Exec: implement each arm.
- [ ] Commit `feat(xtensa): exec branch family`.

---

### Task D7: Jump / CALL / RET / CALLX / RETW (non-windowed semantics only)

Plan 1 wires CALL4/8/12 to update WindowStart bit but defers OF/UF exceptions to Task G3. For Tasks D7, implement:
- `J offset`: `pc = pc + offset`.
- `Jx as_`: `pc = a[as_]`.
- `Call0 offset`: `a0 = pc + 3` (return addr), `pc = ((pc + 3) & !3) + offset`.
- `Callx0 as_`: save, jump.
- `Call4/8/12 offset`: in addition to J, write PS.CALLINC = n/4 and rotate window on ENTRY (Task G2).
- `Ret`: `pc = a0`.
- `Retw`: unimplemented → Task G2.

- [ ] Tests: hand-asm "call a subroutine that returns a constant in a2."
- [ ] Implement.
- [ ] Commit.

---

### Task D8: Code-Density narrow exec (ADD.N, ADDI.N, MOV.N, MOVI.N, L32I.N, S32I.N, BEQZ.N, BNEZ.N, NOP.N, RET.N, BREAK.N, ILL.N, RETW.N)

Narrow decoder in `xtensa_narrow.rs` must produce either an existing `Instruction::*` where semantics are identical, or new narrow-variant enum members. For Plan 1 simplicity, reuse wide variants (decoder decodes `ADD.N` as `Instruction::Add`) — the only difference is the length, which the fetch loop already tracks via `len`.

- [ ] Fill in narrow decoder:
```rust
pub fn decode_narrow(hw: u16) -> Instruction {
    let op0 = (hw & 0xF) as u8;
    let s = ((hw >> 4) & 0xF) as u8;
    let t = ((hw >> 8) & 0xF) as u8;
    let r = ((hw >> 12) & 0xF) as u8;
    match op0 {
        0x8 => Instruction::L32i { at: t, as_: s, imm: (r as u32) << 2 },   // L32I.N
        0x9 => Instruction::S32i { at: t, as_: s, imm: (r as u32) << 2 },   // S32I.N
        0xA => Instruction::Add { ar: r, as_: s, at: t },                   // ADD.N
        0xB => { let imm = sext4_nonzero(r); Instruction::Addi { at: t, as_: s, imm8: imm } } // ADDI.N — imm field special: 0 encodes -1
        0xD => decode_narrow_d(hw, r, s, t),
        _ => Instruction::Unknown(hw as u32),
    }
}
fn sext4_nonzero(r: u8) -> i32 { if r == 0 { -1 } else { r as i32 } }
fn decode_narrow_d(hw: u16, r: u8, s: u8, t: u8) -> Instruction {
    // Xtensa ISA RM §3.3.7 narrow-D sub-opcodes, selected by `r`:
    //   r = 0        MOV.N    at <- as_                 → reuse wide MOV (OR at, as_, as_)
    //   r = 2, 3     MOVI.N   at, imm7                  → reuse wide MOVI
    //                            imm7 = sign_extend_7( ((r & 0x1) << 7) | (s << 4) | t ) with the
    //                            spec-specific wrap: if top bit set, value = raw - 128; else raw.
    //   r = 15 (0xF) zero-operand narrow instructions; sub-sub by s and t:
    //                  s=0  t=0  RET.N     → Ret
    //                  s=0  t=1  RETW.N    → Retw
    //                  s=0  t=2  BREAK.N   → Break { imm_s: 0, imm_t: 0 }   (narrow has no operands)
    //                  s=0  t=3 NOP.N      → Nop
    //                  s=0  t=6  ILL.N     → Ill
    //   any other r   unknown / reserved
    match r {
        0x0 => Instruction::Or { ar: t, as_: s, at: s }, // MOV.N = OR at, as_, as_
        0x2 | 0x3 => {
            let raw = (((r & 0x1) as u32) << 7) | ((s as u32) << 4) | t as u32;
            let imm = ((raw ^ 0x80).wrapping_sub(0x80)) as i32; // sign-extend 8 bits
            Instruction::Movi { at: t, imm }
        }
        0xF => match (s, t) {
            (0x0, 0x0) => Instruction::Ret,
            (0x0, 0x1) => Instruction::Retw,
            (0x0, 0x2) => Instruction::Break { imm_s: 0, imm_t: 0 },
            (0x0, 0x3) => Instruction::Nop,
            (0x0, 0x6) => Instruction::Ill,
            _ => Instruction::Unknown(hw as u32),
        },
        _ => Instruction::Unknown(hw as u32),
    }
}
```
Exact narrow-D sub-opcode table is in Xtensa ISA RM §3.3.7. Transcribe carefully.

- [ ] Tests for each narrow form.
- [ ] Commit `feat(xtensa): Code Density narrow decode + exec via wide variants`.

---

## PHASE E — MUL + bit-manip + atomics

Goal: integer multiply, divide, bit-manip, atomic ops all executing.

### Task E1: MUL family

MULL, MULUH, MULSH, MUL16S, MUL16U. Decode is RRR with specific op2 values:
- MULL  op2=0x8, op1=0x2
- MULUH op2=0xA, op1=0x2
- MULSH op2=0xB, op1=0x2
- MUL16U op2=0x6, op1=0x0 with s,t form per ISA RM §8 "MUL16U"

Tests, exec, commit. Rust `u32::wrapping_mul` for low 32, `((a as u64) * (b as u64) >> 32) as u32` for upper, signed variant cast-through-`i32`.

---

### Task E2: DIV family (QUOS, QUOU, REMS, REMU)

Decode + exec. Handle division by zero: Xtensa ISA RM specifies "integer divide-by-zero exception" (EXCCAUSE=6). Tests must cover both nonzero and zero-divisor cases.

Commit.

---

### Task E3: Bit-manip

NSA, NSAU, MIN, MAX, MINU, MAXU, SEXT, CLAMPS.

Semantics:
- `NSA ar, as_`: count sign bits minus 1. `ar = clz(if as_>=0 then as_ else !as_) - 1`.
- `NSAU ar, as_`: `ar = clz(as_)` for unsigned.
- `MIN/MAX/MINU/MAXU`: obvious.
- `SEXT ar, as_, t`: sign-extend `as_` from bit position `t+7` downward.
- `CLAMPS ar, as_, t`: saturate signed `as_` into `(t+7)+1`-bit range.

Tests, exec, commit.

---

### Task E4: Atomic — S32C1I + SCOMPARE1 + L32AI + S32RI

`S32C1I at, as_, imm`:
- `EA = as_ + imm`
- `mem32 = bus.read_u32(EA)`
- if `mem32 == SCOMPARE1`: `bus.write_u32(EA, at)` else: no-write
- `at = mem32` (old value always returned)

`L32AI at, as_, imm`: like L32I but with an implicit acquire barrier (on our single-quantum bus, a no-op ordering-wise in Plan 1; SMP effects land in Plan 4).

`S32RI at, as_, imm`: like S32I with release barrier.

Tests:
- Uncontended CAS success
- Uncontended CAS failure (SCOMPARE1 mismatch)
- Tests must verify SCOMPARE1 SR is read through the SR dispatcher.

Exec + commit.

---

## PHASE F — Windowed machinery

### Task F1: ENTRY + RETW without OF/UF

`ENTRY as_, imm`: allocate a new frame — compute `as_new = as_ - ((imm - 4) & ~0x3)` (wait: actually `a[as_] = a[as_] - imm` per ISA RM; window doesn't rotate on ENTRY itself, it rotates on preceding CALL*). Check Xtensa ISA RM §8 "ENTRY" carefully.

Real ENTRY semantics:
- Rotate window-base by CALLINC (set by previous CALLn).
- Adjust WindowStart: clear bit `(WB_old - CALLINC)` cleared? — set the bit corresponding to `WB_new`.
- Check for window-overflow: if WindowStart bit `(WB_new + 1) mod 16` is set, raise exception.
- Adjust `a[as_] -= imm` to allocate stack.

Tests cover the non-OF path first. Then Task F2/F3 adds OF/UF handling.

Commit.

---

### Task F2: CALL4/8/12 / CALLX4/8/12 WindowStart updates

Implement the `callinc = n/4` set-up and PC jump. WindowStart bit manipulation actually happens on ENTRY (see Xtensa ISA RM); CALL* only sets PS.CALLINC.

- [ ] Tests: CALL4 + ENTRY + RETW round-trip for a single stack frame.
- [ ] Commit.

---

### Task F3: Window overflow exception

When ENTRY is executed and the destination frame is marked in-use (its WindowStart bit set), raise WindowOverflow*4/8/12 exception (EXCCAUSE = 5/6/7 depending on call size). The handler is at a 64-byte slot from VECBASE:
- OF4:  VECBASE + 0x00
- OF8:  VECBASE + 0x40
- OF12: VECBASE + 0x80
- UF4:  VECBASE + 0x40 (same as OF8 no — check table)
- UF8:  VECBASE + 0xC0
- UF12: VECBASE + 0x100

Refer to Xtensa ISA RM for exact vector table layout. Transcribe in a constant.

- [ ] Tests: exec ENTRY where target frame is in use; expect PC = VECBASE + OF vector offset; EPC1 = original PC; PS.EXCM=1.
- [ ] Commit.

---

### Task F4: Window underflow exception + RETW return

On RETW: rotate WB back. If the destination frame's WindowStart bit is clear, raise UnderflowException. Handler loads physical regs back from stack.

- [ ] Tests: deep call, RETW that would trigger UF; verify vector entry.
- [ ] Commit.

---

### Task F5: S32E / L32E (context-only opcodes)

`S32E/L32E at, as_, imm`: same EA calculation as `S32I/L32I` BUT only valid when `PS.EXCM == 1`. Outside that context they raise `IllegalInstruction` (EXCCAUSE = 0).

- [ ] Tests: S32E inside vector (PS.EXCM=1) works; S32E outside (PS.EXCM=0) raises IllegalInstruction.
- [ ] Commit.

---

### Task F6: MOVSP + ROTW

`MOVSP at, as_`: conditional spill/reload if `WindowStart bit (WB+1) mod 16` is set. Can trigger OF/UF exceptions.

`ROTW n`: rotate window base by `n` (signed). Normally used only in vector code.

- [ ] Tests + exec + commit.

---

## PHASE G — Exception / interrupt dispatch

### Task G1: EPC/EPS/EXCSAVE shadow stacks

Already built in SR table (Task C2). Now wire them into exec:
- On exception entry at level `n`: `EPC[n] = PC; EPS[n] = PS; EXCSAVE[n] = a0 (if vector does save-a0-first); PS.EXCM = 1; PS.INTLEVEL = n; PC = VECBASE + offset(n)`.

For EXCCAUSE-based exceptions (not interrupts), set EXCCAUSE and EXCVADDR.

- [ ] Tests: fire illegal instruction; verify EPC1 = bad PC, EXCCAUSE = 0, PS.EXCM = 1, PC = VECBASE + 0x340 (kernel vector offset for the ESP32-S3 LX7 config).
- [ ] Commit.

---

### Task G2: RFE / RFI / RFWO / RFWU / RFDE / RFDO return paths

- `RFE` (Return From Exception): `PS = EPS1? wait — RFE returns from kernel exception: PS = PS with EXCM=0, INTLEVEL=0, PC = EPC1`. Refer to ISA RM.
- `RFI n`: `PS = EPS[n]; PC = EPC[n]`.
- `RFWO/RFWU`: window return paths.

- [ ] Tests for each.
- [ ] Commit.

---

### Task G3: INTERRUPT / INTENABLE / INTCLEAR dispatch

A pending interrupt fires when `INTERRUPT & INTENABLE != 0` AND `PS.INTLEVEL < highest_level_of_pending_bit` AND `PS.EXCM == 0`.

- [ ] Tests: set INTENABLE to enable one level; raise a virtual IRQ (set INTERRUPT bit via test hook); step CPU; verify EPC[level] and PS transitions.
- [ ] Commit.

---

### Task G4: BREAK handling + panic trap

`BREAK` raises a debug exception (EXCCAUSE = ???, actually Debug exception is DEBUGCAUSE-based). For Plan 1 simplicity: emit `SimulationError::BreakpointHit(pc)` from exec, which the test harness uses to halt and collect state.

- [ ] Already covered in Task D1 partially; add explicit test: `BREAK 1,15` halts execution with PC = break address in error.
- [ ] Commit `feat(xtensa): BREAK halt plumbing for oracle test harness`.

---

## PHASE H — HW-oracle harness

### Task H1: OpenOCD subprocess wrapper

**File:** `crates/hw-oracle/src/openocd.rs`

Wraps `openocd` subprocess with TCL command interface. Start it in daemon mode listening on TCP port `6666` (default). Send commands like `reset halt`, `mdw 0x40370000 4` (read 4 words), `reg a0`, `resume`.

- [ ] **Step 1: Failing test**
```rust
#[test]
#[ignore] // gated — run with --ignored when HW connected
fn openocd_halts_and_reads_reg() {
    let mut oc = OpenOcd::spawn_default().unwrap();
    oc.reset_halt().unwrap();
    let a0 = oc.read_register("a0").unwrap();
    // Value is unpredictable but the call must succeed.
    let _ = a0;
    oc.shutdown().unwrap();
}
```

- [ ] **Step 2..5:** Implement. Use `std::process::Command` to spawn openocd with `-f target/esp32s3.cfg`. Open TCP socket to port 6666, send TCL, parse text responses. Watch out for openocd's TCL line endings (`\x1a` EOF byte).

Reference: `openocd` manual, "TCL Server" section.

Commit `feat(hw-oracle): OpenOCD TCL wrapper: reset/halt/resume/step/read-reg/read-mem/write-reg/write-mem`.

---

### Task H2: Flash + reset + halt primitive

**File:** `crates/hw-oracle/src/flash.rs`

Use `espflash` library to flash a tiny ELF. Then `reset_halt` via OpenOCD. Confirm PC is at the ELF entry.

- [ ] Implement `TargetBoard::detect()` in `crates/hw-oracle/src/flash.rs`:

```rust
use anyhow::{anyhow, Result};

pub struct TargetBoard {
    /// USB bus device path, e.g. "/dev/ttyACM0".
    pub serial_port: String,
    /// USB VID:PID.
    pub usb_id: (u16, u16),
}

impl TargetBoard {
    pub fn detect() -> Result<Self> {
        // Enumerate via serialport crate; match VID:PID 303a:1001 (or $LABWIRED_BOARD_USB).
        let wanted = std::env::var("LABWIRED_BOARD_USB")
            .ok()
            .and_then(|s| parse_vid_pid(&s))
            .unwrap_or((0x303a, 0x1001));
        for p in serialport::available_ports()? {
            if let serialport::SerialPortType::UsbPort(info) = &p.port_type {
                if (info.vid, info.pid) == wanted {
                    return Ok(TargetBoard { serial_port: p.port_name, usb_id: wanted });
                }
            }
        }
        Err(anyhow!("no board with USB id {:04x}:{:04x} found", wanted.0, wanted.1))
    }

    /// Flash an ELF file using the `espflash` library.
    pub fn flash(&self, elf_bytes: &[u8]) -> Result<()> {
        // Wrap espflash's Flasher::load_elf (see espflash crate docs).
        // Placeholder flashing path: rely on espflash's default config for ESP32-S3.
        unimplemented!("wire up espflash::Flasher::load_elf here");
    }
}

fn parse_vid_pid(s: &str) -> Option<(u16, u16)> {
    let (v, p) = s.split_once(':')?;
    Some((u16::from_str_radix(v, 16).ok()?, u16::from_str_radix(p, 16).ok()?))
}
```

- [ ] Test (`#[ignore]` — needs physical HW):
```rust
#[test]
#[ignore]
fn flash_and_halt_minimal_elf() {
    let elf = std::fs::read("fixtures/xtensa-asm/nop-at-entry.elf").unwrap();
    let board = TargetBoard::detect().unwrap();
    board.flash(&elf).unwrap();
    let mut oc = OpenOcd::spawn_for(&board).unwrap();
    oc.reset_halt().unwrap();
    let pc = oc.read_register("pc").unwrap();
    let entry = elf_entry_point_from_bytes(&elf);
    assert_eq!(pc, entry);
}
```

- [ ] Implement `elf_entry_point_from_bytes` using `goblin::elf::Elf::parse(&bytes)?.entry as u32`. Commit.

---

### Task H3: `#[hw_oracle_test]` macro expansion

**File:** `crates/hw-oracle-macros/src/lib.rs`

Expand
```rust
#[hw_oracle_test]
fn add_oracle() -> OracleCase {
    OracleCase::asm(".word 0x00008530") // ADD a3, a4, a5
        .setup(|st| { st.write_reg("a4", 0x11); st.write_reg("a5", 0x22); })
        .expect(|st| { st.assert_reg("a3", 0x33); })
}
```
into
```rust
#[test]
fn add_oracle_sim() { labwired_hw_oracle::run_sim(add_oracle_inner()); }
#[test]
#[cfg(feature = "hw-oracle")]
fn add_oracle_hw() { labwired_hw_oracle::run_hw(add_oracle_inner()); }
#[test]
#[cfg(feature = "hw-oracle")]
fn add_oracle_diff() { labwired_hw_oracle::run_diff(add_oracle_inner()); }
fn add_oracle_inner() -> OracleCase { /* original body */ }
```

- [ ] **Step 1..5**: macro expansion test with `trybuild` to snapshot output; commit.

Commit `feat(hw-oracle): #[hw_oracle_test] macro producing sim/hw/diff triplets`.

---

### Task H4: `OracleCase` runtime + first oracle test (ADD)

**File:** `crates/hw-oracle/src/lib.rs`

Define:
```rust
use std::collections::HashMap;

pub struct OracleCase {
    pub program: Program,
    pub setup: Box<dyn Fn(&mut OracleState) + Send + Sync>,
    pub expect: Box<dyn Fn(&OracleState) + Send + Sync>,
    pub tolerance: Tolerance,
}

pub enum Program {
    /// Raw encoded instruction words, relocated to IRAM on flash/sim-reset.
    Asm(Vec<u8>),
    /// ELF path loaded via espflash on HW; parsed directly on sim.
    Elf(std::path::PathBuf),
}

#[derive(Default)]
pub struct OracleState {
    pub regs: HashMap<String, u32>,
    pub mem: HashMap<u32, u32>,
}

impl OracleState {
    pub fn write_reg(&mut self, name: &str, v: u32) { self.regs.insert(name.into(), v); }
    pub fn read_reg(&self, name: &str) -> u32 { *self.regs.get(name).unwrap_or(&0) }
    pub fn assert_reg(&self, name: &str, expected: u32) {
        let actual = self.read_reg(name);
        assert_eq!(actual, expected, "reg {name}: expected 0x{expected:08X}, got 0x{actual:08X}");
    }
}

#[derive(Debug, Clone)]
pub struct Tolerance {
    pub ccount_cycles: u32,  // ±cycles tolerated for CCOUNT
    pub timestamp_ps:  u64,  // ±picoseconds tolerated for event timestamps (Plan 2+)
}

impl Tolerance {
    pub fn exact() -> Self { Self { ccount_cycles: 0, timestamp_ps: 0 } }
    pub fn lenient() -> Self { Self { ccount_cycles: 2, timestamp_ps: 1_000 } }
}

/// Builder-style constructors for OracleCase.
impl OracleCase {
    pub fn asm(hex_word: &str) -> Self { /* parse ".word 0xNN...", one or more */ unimplemented!() }
    pub fn elf(path: &str) -> Self { /* store ELF path */ unimplemented!() }
    pub fn setup<F: 'static + Fn(&mut OracleState) + Send + Sync>(mut self, f: F) -> Self { self.setup = Box::new(f); self }
    pub fn expect<F: 'static + Fn(&OracleState) + Send + Sync>(mut self, f: F) -> Self { self.expect = Box::new(f); self }
    pub fn tolerance(mut self, t: Tolerance) -> Self { self.tolerance = t; self }
}

pub fn run_sim(case: OracleCase) { /* instantiate XtensaLx7, inject setup, step until BREAK, run expect */ }
pub fn run_hw(case: OracleCase) { /* flash via espflash, setup via openocd write_reg, resume, wait for halt on BREAK, pull state, run expect */ }
pub fn run_diff(case: OracleCase) { /* run sim + hw; diff OracleState bitwise within tolerance; assert_eq! */ }
```

- [ ] First oracle: ADD. Confirm `add_oracle_sim` passes; `add_oracle_hw` passes (ignored without feature); `add_oracle_diff` passes end-to-end.
- [ ] Commit `feat(hw-oracle): OracleCase runtime + first ADD oracle test`.

---

### Task H5: Oracle test bank — ALU + shift group (15 tests)

Write one `#[hw_oracle_test]` per: ADD, SUB, AND, OR, XOR, NEG, ABS, ADDX2, ADDX4, ADDX8, SUBX2, SUBX4, SUBX8, SLL, SRA.

Use the same state-setup pattern. Commit after all 15 pass on both sim and HW.

Commit `feat(hw-oracle): ALU + shift oracle bank (15 tests)`.

---

### Task H6: Oracle bank — loads / stores / L32R / branches (15 tests)

L8UI, L16UI, L16SI, L32I, S8I, S16I, S32I, L32R, plus BEQ (taken/not-taken), BNE (taken/not-taken), BEQZ, BLTUI, J, CALL0.

Commit.

---

### Task H7: Oracle bank — windowing (8 tests)

CALL4 + ENTRY + RETW (no OF/UF), single-level with OF, nested 2-level, UF on deep return, S32E inside vector, S32E outside vector (expect IllegalInstr on both sim and HW), ROTW, MOVSP.

Commit.

---

### Task H8: Oracle bank — exception / interrupt (6 tests)

IllegalInstruction fires + EPC/EPS/EXCCAUSE readback; RFE returns correctly; INTENABLE+INTERRUPT → interrupt vector dispatch; RFI returns; VECBASE relocation (write new VECBASE, raise exception, verify new vector hit).

Commit.

---

## PHASE I — Final milestone: Fibonacci end-to-end

### Task I1: Hand-asm Fibonacci fixture

**Files:**
- Create: `fixtures/xtensa-asm/fibonacci.s`
- Create: `fixtures/xtensa-asm/linker.ld`
- Create: `fixtures/xtensa-asm/Makefile`

Assembly:
```
    .section .text.entry, "ax"
    .align 4
    .global _start
_start:
    entry   a1, 32
    movi    a2, 10              /* N = 10 */
    movi    a3, 0               /* fib(n-2) */
    movi    a4, 1               /* fib(n-1) */
    beqz    a2, done
loop:
    add     a5, a3, a4
    mov     a3, a4
    mov     a4, a5
    addi    a2, a2, -1
    bnez    a2, loop
done:
    mov     a2, a3              /* result in a2 */
    break   1, 15
```

Expected: a2 = fib(10) = 55.

Makefile uses `xtensa-esp32s3-elf-as` + `ld` + `objcopy` to build both `.elf` (for espflash) and `.bin` (for sim). Linker script places `.text.entry` at `0x4037_0000` (IRAM start).

- [ ] **Step 1: Write test**
```rust
#[hw_oracle_test]
fn fibonacci_10() -> OracleCase {
    OracleCase::elf("fixtures/xtensa-asm/fibonacci.elf")
        .expect(|st| st.assert_reg("a2", 55))
        .tolerance(Tolerance::exact())
}
```
- [ ] **Step 2:** FAIL (no fixture).
- [ ] **Step 3..4:** Build fixture; both `fibonacci_sim` and `fibonacci_hw` pass; `fibonacci_diff` passes.
- [ ] **Step 5:** Commit `feat(fixtures): fibonacci.s + oracle-verified end-to-end on sim and HW`.

---

### Task I2: CI workflow — hw-oracle gated

**File:** `.github/workflows/hw-oracle.yml`

Self-hosted runner with label `esp32s3-zero`. Runs:
```yaml
- cargo test -p labwired-core --features hw-oracle
- cargo test -p labwired-hw-oracle --features hw-oracle
```

Gate on PR label `hw-test` or push to `main`. Provide a sim-only fallback workflow (`ci.yml`) that always runs unit/integration/decode tests on `ubuntu-latest`.

- [ ] Commit `ci: gated hw-oracle workflow on self-hosted runner with S3-Zero`.

---

### Task I3: Documentation

**File:** `docs/case_study_esp32s3_plan1.md`

Summarize:
- What Plan 1 delivered (tie back to M1 + M2 milestones in the spec).
- Oracle test pass rate on sim-only and on HW.
- Known gaps or behaviors that surfaced during bringup but aren't yet scoped.
- Invitation for Plan 2 (boot + core peripherals).

- [ ] Commit `docs: Plan 1 case study + M1/M2 milestone closeout`.

---

## Plan 1 exit criteria

Plan 1 is complete when ALL of the following are true:

1. `cargo test --workspace` passes on `ubuntu-latest` with zero failures, zero warnings under `-D warnings`.
2. `cargo test -p labwired-core --features hw-oracle` passes on the self-hosted runner with the physical S3-Zero plugged in.
3. At least **44 oracle tests** pass on both sim and HW, across the 4 oracle banks (ALU 15 + Mem/Branch 15 + Windowing 8 + Exception 6 + Fibonacci end-to-end = 45).
4. `fibonacci_diff` test passes bit-exact — sim and HW register state at `BREAK` are identical.
5. The word-granular bus write path is merged and declarative peripheral tests still pass.
6. `docs/case_study_esp32s3_plan1.md` exists and describes the outcome.
7. `hw-oracle.yml` workflow has had at least one successful PR gate and one nightly run.

**When exit criteria are met, invoke writing-plans again for Plan 2 (Boot path + core peripherals).**
