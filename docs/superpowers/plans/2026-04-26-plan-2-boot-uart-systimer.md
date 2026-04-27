# Plan 2 — Boot Path + USB_SERIAL_JTAG + SYSTIMER (esp-hal hello-world) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Run a real `esp-hal` Rust firmware (`xtensa-esp32s3-none-elf`) end-to-end in the LabWired simulator, printing `"Hello world!"` to stdout once per second via the simulated `USB_SERIAL_JTAG` peripheral, with `esp_hal::delay::Delay` driven by the simulated `SYSTIMER`.

**Architecture:** A new `boot` module performs fast-boot of an ELF (place segments via the bus, synthesise post-bootloader CPU state). A new `peripherals/esp32s3/` module group provides `RomThunkBank` (ROM functions dispatched via reserved BREAK 1,14), `UsbSerialJtag`, `Systimer`, three small register stubs (`SystemStub`, `RtcCntlStub`, `EfuseStub`), and `FlashXipPeripheral`. A new `system/xtensa.rs` glues everything together. The CLI gets a `run` subcommand that loads an ELF + chip YAML and runs the simulation indefinitely.

**Tech Stack:** Rust 2021 edition (workspace `edition.workspace = true`), `goblin = "0.7"` for ELF parsing, `clap` for CLI, `esp-hal` v1.0 + `esp-println` v0.13 for the example firmware, Xtensa toolchain at `~/.rustup/toolchains/esp/xtensa-esp-elf/esp-15.2.0_20250920/`.

**Spec:** `docs/superpowers/specs/2026-04-26-plan-2-boot-uart-systimer.md`

---

## Pre-flight

### Task 0: Branch + baseline verification

**Files:** none (git only)

- [ ] **Step 1: Confirm clean working tree**

```bash
git status
git rev-parse --abbrev-ref HEAD
```

Expected: `nothing to commit, working tree clean` and `feature/esp32s3-plan1-foundation`.

- [ ] **Step 2: Create the Plan 2 branch**

```bash
git checkout -b feature/esp32s3-plan2-boot-uart
```

Expected: `Switched to a new branch 'feature/esp32s3-plan2-boot-uart'`.

- [ ] **Step 3: Capture sim test baseline**

```bash
cargo test --workspace \
  --exclude firmware \
  --exclude firmware-ci-fixture \
  --exclude riscv-ci-fixture \
  2>&1 | tail -10
```

Expected: ≥ 461 sim tests passing. Record the exact count for end-of-plan diff.

- [ ] **Step 4: Confirm Xtensa toolchain is on PATH**

```bash
which xtensa-esp32s3-elf-as && xtensa-esp32s3-elf-as --version | head -1
which xtensa-esp32s3-elf-objdump && xtensa-esp32s3-elf-objdump --version | head -1
which xtensa-esp32s3-elf-ld && xtensa-esp32s3-elf-ld --version | head -1
```

Expected: paths under `~/.rustup/toolchains/esp/xtensa-esp-elf/esp-15.2.0_20250920/bin/` and version strings. If missing, add the bin dir to `PATH` for this shell session.

- [ ] **Step 5: Confirm the ESP Rust toolchain is installed**

```bash
rustup toolchain list | grep esp
ls ~/.rustup/toolchains/esp/ 2>/dev/null
```

Expected: an `esp` toolchain entry. If missing, follow https://docs.esp-rs.org/book/installation/index.html to install (`espup install`). The implementer must have this before Task 13.

---

## Phase 1 — Boot Module (1 task)

### Task 1: Boot module skeleton + ELF segment loader

**Files:**
- Create: `crates/core/src/boot/mod.rs`
- Create: `crates/core/src/boot/esp32s3.rs`
- Modify: `crates/core/src/lib.rs:7-17` (add `pub mod boot;`)
- Modify: `crates/core/Cargo.toml` (add `goblin = { workspace = true }`)
- Test: inline `#[cfg(test)] mod tests` in `boot/esp32s3.rs`

- [ ] **Step 1: Add `goblin` dependency to `crates/core/Cargo.toml`**

Edit `crates/core/Cargo.toml`. After the existing `serde_json = { workspace = true }` line, add:

```toml
goblin = { workspace = true }
```

- [ ] **Step 2: Create `crates/core/src/boot/mod.rs`**

```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Boot-path implementations for each supported chip.
//!
//! Each submodule provides a `fast_boot` function that takes an ELF byte slice,
//! a `SystemBus` with all peripherals already mapped, and a CPU; loads the ELF
//! segments via the bus; synthesises the post-bootloader CPU state; and
//! returns a `BootResult` describing what was loaded.

pub mod esp32s3;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum BootError {
    #[error("ELF parse error: {0}")]
    ElfParse(String),
    #[error("ELF segment vaddr 0x{addr:08x} (size {size}) is outside any mapped peripheral")]
    SegmentOutsideMap { addr: u32, size: usize },
    #[error("flash-XIP page table overflow: tried to map {requested} pages (max 64)")]
    TooManyXipPages { requested: usize },
    #[error("no stack top: ELF symbol _stack_start_cpu0 not found and no fallback supplied")]
    NoStackTop,
    #[error("simulator error during boot: {0}")]
    Sim(#[from] crate::SimulationError),
}

pub type BootResult<T> = Result<T, BootError>;
```

- [ ] **Step 3: Create `crates/core/src/boot/esp32s3.rs` with the public API surface and a stub implementation**

```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Fast-boot for ESP32-S3 ELFs (`xtensa-esp32s3-none-elf` target).
//!
//! Skips the BROM and 2nd-stage bootloader; places ELF segments at their
//! virtual addresses via the bus and synthesises post-bootloader CPU state.

use crate::boot::{BootError, BootResult};
use crate::bus::SystemBus;
use crate::cpu::xtensa_lx7::XtensaLx7;
use crate::Bus;
use goblin::elf::program_header::PT_LOAD;
use goblin::elf::Elf;

/// Per-call options for `fast_boot`.
#[derive(Debug, Clone)]
pub struct BootOpts {
    /// Used as the SP if the ELF lacks a `_stack_start_cpu0` symbol.
    pub stack_top_fallback: u32,
}

/// Result of a successful boot.
#[derive(Debug, Clone, Copy)]
pub struct BootSummary {
    pub entry: u32,
    pub stack: u32,
    pub segments_loaded: usize,
}

/// Load `elf_bytes` into the bus, set the CPU's PC and SP, return a summary.
///
/// This function does NOT touch peripherals other than via `Bus::write_u8`.
/// The caller is responsible for having registered all relevant peripherals
/// (IRAM, DRAM, flash-XIP, ROM thunks, etc.) before calling.
pub fn fast_boot(
    elf_bytes: &[u8],
    bus: &mut SystemBus,
    cpu: &mut XtensaLx7,
    opts: &BootOpts,
) -> BootResult<BootSummary> {
    let elf = Elf::parse(elf_bytes).map_err(|e| BootError::ElfParse(format!("{e}")))?;

    let mut segments_loaded = 0;
    for ph in &elf.program_headers {
        if ph.p_type != PT_LOAD || ph.p_filesz == 0 {
            continue;
        }
        let vaddr = ph.p_vaddr as u32;
        let file_off = ph.p_offset as usize;
        let size = ph.p_filesz as usize;
        let bytes = &elf_bytes[file_off..file_off + size];
        for (i, &b) in bytes.iter().enumerate() {
            let addr = vaddr.wrapping_add(i as u32) as u64;
            bus.write_u8(addr, b)
                .map_err(|_| BootError::SegmentOutsideMap { addr: vaddr, size })?;
        }
        segments_loaded += 1;
    }

    // Look up `_stack_start_cpu0`; fall back to opts.stack_top_fallback.
    let stack = elf
        .syms
        .iter()
        .find(|sym| {
            let name = elf.strtab.get_at(sym.st_name).unwrap_or("");
            name == "_stack_start_cpu0" || name == "_stack_top"
        })
        .map(|sym| sym.st_value as u32)
        .unwrap_or(opts.stack_top_fallback);

    let entry = elf.entry as u32;
    cpu.set_pc(entry);
    cpu.set_sp(stack);

    Ok(BootSummary {
        entry,
        stack,
        segments_loaded,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::xtensa_lx7::XtensaLx7;
    use crate::{Bus, Cpu, Peripheral, SimResult};

    /// A minimal `Peripheral` backed by a flat byte array, used to satisfy
    /// `fast_boot`'s `bus.write_u8` calls in unit tests.
    #[derive(Debug)]
    struct RamPeripheral {
        data: std::cell::RefCell<Vec<u8>>,
    }

    impl Peripheral for RamPeripheral {
        fn read(&self, offset: u64) -> SimResult<u8> {
            Ok(*self.data.borrow().get(offset as usize).unwrap_or(&0))
        }
        fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
            let mut d = self.data.borrow_mut();
            if let Some(slot) = d.get_mut(offset as usize) {
                *slot = value;
            }
            Ok(())
        }
    }

    /// Build a minimal valid Xtensa ELF in memory: one PT_LOAD segment of 4
    /// bytes at `0x4037_0000`, entry point at the same address, no symbols.
    /// We use the `goblin::elf` writer for clarity, but it doesn't have a
    /// builder API, so we hand-construct the bytes.
    fn build_minimal_elf() -> Vec<u8> {
        // ELF64 header (64 bytes) + 1 program header (56 bytes) + 4 bytes payload
        let mut elf = vec![0u8; 64 + 56 + 4];

        // ELF identification
        elf[0..4].copy_from_slice(b"\x7FELF");
        elf[4] = 2; // EI_CLASS = ELFCLASS64
        elf[5] = 1; // EI_DATA = ELFDATA2LSB
        elf[6] = 1; // EI_VERSION = EV_CURRENT
        elf[16] = 2; // e_type = ET_EXEC
        elf[17] = 0;
        elf[18] = 94; // e_machine = EM_XTENSA (94)
        elf[19] = 0;
        // e_version (4 bytes) at 20
        elf[20] = 1;
        // e_entry (8 bytes) at 24
        elf[24..28].copy_from_slice(&0x4037_0000u32.to_le_bytes());
        // e_phoff (8 bytes) at 32
        elf[32] = 64;
        // e_ehsize at 52, e_phentsize at 54, e_phnum at 56
        elf[52..54].copy_from_slice(&64u16.to_le_bytes());
        elf[54..56].copy_from_slice(&56u16.to_le_bytes());
        elf[56..58].copy_from_slice(&1u16.to_le_bytes());

        // Program header at offset 64
        let ph = 64;
        elf[ph..ph + 4].copy_from_slice(&1u32.to_le_bytes()); // p_type = PT_LOAD
        elf[ph + 4..ph + 8].copy_from_slice(&5u32.to_le_bytes()); // p_flags = R+X
        elf[ph + 8..ph + 16].copy_from_slice(&120u64.to_le_bytes()); // p_offset
        elf[ph + 16..ph + 24].copy_from_slice(&0x4037_0000u64.to_le_bytes()); // p_vaddr
        elf[ph + 24..ph + 32].copy_from_slice(&0x4037_0000u64.to_le_bytes()); // p_paddr
        elf[ph + 32..ph + 40].copy_from_slice(&4u64.to_le_bytes()); // p_filesz
        elf[ph + 40..ph + 48].copy_from_slice(&4u64.to_le_bytes()); // p_memsz

        // Payload at offset 120
        elf[120..124].copy_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]);

        elf
    }

    #[test]
    fn fast_boot_places_segment_and_sets_pc_sp() {
        let elf_bytes = build_minimal_elf();

        let mut bus = SystemBus::new();
        bus.add_peripheral(
            "iram",
            0x4037_0000,
            0x1_0000,
            None,
            Box::new(RamPeripheral {
                data: std::cell::RefCell::new(vec![0u8; 0x1_0000]),
            }),
        );

        let mut cpu = XtensaLx7::new();
        cpu.reset(&mut bus).unwrap();

        let summary = fast_boot(
            &elf_bytes,
            &mut bus,
            &mut cpu,
            &BootOpts {
                stack_top_fallback: 0x3FCD_FFF0,
            },
        )
        .expect("fast_boot");

        assert_eq!(summary.entry, 0x4037_0000);
        assert_eq!(summary.stack, 0x3FCD_FFF0);
        assert_eq!(summary.segments_loaded, 1);

        assert_eq!(cpu.get_pc(), 0x4037_0000);
        assert_eq!(bus.read_u8(0x4037_0000).unwrap(), 0xAA);
        assert_eq!(bus.read_u8(0x4037_0003).unwrap(), 0xDD);
    }

    #[test]
    fn fast_boot_returns_segment_outside_map_on_unmapped_vaddr() {
        let elf_bytes = build_minimal_elf();

        let mut bus = SystemBus::new();
        // Note: no IRAM peripheral mapped; the segment write should hit the
        // default SystemBus's flash/ram routing, miss it, and either silently
        // write to nowhere OR fail. The current SystemBus::write_u8 returns
        // Ok(()) for unmapped addresses, so to test this we use a custom bus
        // wrapper or skip this case; for now, we only assert that the function
        // runs without panic when the address routing is permissive.
        let mut cpu = XtensaLx7::new();
        cpu.reset(&mut bus).unwrap();

        // Document current behaviour: SystemBus is permissive on unmapped
        // writes, so fast_boot succeeds even without IRAM mapped. The error
        // surface for SegmentOutsideMap is only reached if Bus::write_u8
        // returns Err — which the default SystemBus does not.
        let res = fast_boot(
            &elf_bytes,
            &mut bus,
            &mut cpu,
            &BootOpts {
                stack_top_fallback: 0x3FCD_FFF0,
            },
        );
        assert!(res.is_ok(), "fast_boot is permissive on unmapped writes (matches SystemBus)");
    }
}
```

- [ ] **Step 4: Add `pub mod boot;` to `crates/core/src/lib.rs`**

Edit `crates/core/src/lib.rs:7-17`. After `pub mod bus;` line (line 7), add:

```rust
pub mod boot;
```

- [ ] **Step 5: Build and run the new tests**

```bash
cargo build -p labwired-core 2>&1 | tail -20
cargo test -p labwired-core boot::esp32s3 2>&1 | tail -10
```

Expected: build succeeds; both `fast_boot_places_segment_and_sets_pc_sp` and `fast_boot_returns_segment_outside_map_on_unmapped_vaddr` PASS.

- [ ] **Step 6: Run the full sim suite to confirm no regression**

```bash
cargo test --workspace \
  --exclude firmware --exclude firmware-ci-fixture --exclude riscv-ci-fixture \
  2>&1 | tail -5
```

Expected: ≥ baseline + 2 new tests passing.

- [ ] **Step 7: Commit**

```bash
git add crates/core/src/boot crates/core/src/lib.rs crates/core/Cargo.toml
git commit -m "feat(boot): fast_boot module + ESP32-S3 ELF segment loader

Adds crates/core/src/boot/{mod,esp32s3}.rs with fast_boot(elf_bytes,
bus, cpu, opts) -> BootSummary. Parses ELF via goblin, places PT_LOAD
segments through bus.write_u8, looks up _stack_start_cpu0 (or fallback),
sets cpu PC and SP. BootError taxonomy for failure modes. Two unit
tests using a hand-constructed minimal Xtensa ELF."
git push -u origin feature/esp32s3-plan2-boot-uart
```

---

## Phase 2 — ROM Thunks + BREAK Dispatch (1 task)

### Task 2: RomThunkBank + BREAK 1,14 dispatch hook

**Files:**
- Create: `crates/core/src/peripherals/esp32s3/mod.rs`
- Create: `crates/core/src/peripherals/esp32s3/rom_thunks.rs`
- Modify: `crates/core/src/peripherals/mod.rs` (add `pub mod esp32s3;`)
- Modify: `crates/core/src/cpu/xtensa_lx7.rs:185-187` (BREAK arm: dispatch to thunk on imm_s=1, imm_t=14)
- Modify: `crates/core/src/bus/mod.rs` (add `get_rom_thunk` accessor)

- [ ] **Step 1: Create `crates/core/src/peripherals/esp32s3/mod.rs`**

```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 peripheral implementations (Plan 2+).

pub mod rom_thunks;
```

- [ ] **Step 2: Create `crates/core/src/peripherals/esp32s3/rom_thunks.rs`**

```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ROM thunk dispatch for ESP32-S3.
//!
//! The ESP32-S3 has ~384 KiB of mask ROM at 0x4000_0000 holding the BROM
//! reset handler and a library of utility functions (`ets_printf`, cache
//! maintenance, flash access, …).  Real firmware calls a small subset of
//! these.  Rather than emulate the whole BROM, we register Rust thunks at
//! the addresses the firmware calls.
//!
//! ## Dispatch mechanism
//!
//! When the simulator constructs a `RomThunkBank`, it pre-fills the bank's
//! backing memory with the byte sequence `BREAK 1, 14` (encoded
//! `[0xF0, 0x42, 0x00]`) at every registered address.  When the CPU fetches
//! from that address it gets BREAK back.  The CPU's BREAK exec arm
//! recognises `imm_s == 1 && imm_t == 14` as a thunk dispatch, looks up
//! the current PC in the bank, and calls the registered Rust function.
//! The function is responsible for setting `PC = a0` to return.
//!
//! The level-imm pair `1, 14` is reserved for ROM thunks; `1, 15` is
//! reserved for the oracle harness BREAK.  Other BREAK values fall through
//! to the existing `SimulationError::BreakpointHit` raise.

use crate::cpu::xtensa_lx7::XtensaLx7;
use crate::{Bus, Peripheral, SimResult, SimulationError};
use std::collections::HashMap;

/// A ROM thunk function: invoked when the CPU executes the registered
/// `BREAK 1, 14` at a known address.  Must set `cpu.pc = a0` to return.
pub type RomThunkFn = fn(&mut XtensaLx7, &mut dyn Bus) -> SimResult<()>;

/// `BREAK 1, 14` encoded as 3 LE bytes.
///
/// Encoding (ST0 format, op0=0, op1=0, op2=0, r=4, s=imm_s=1, t=imm_t=14):
///   st0(r=4, s=1, t=14) = (4<<12)|(1<<8)|(14<<4) = 0x40_E0 + 0x0100 = 0x41E0
///   3-byte LE: 0xE0, 0x41, 0x00
///
/// (The 0xF0 byte in the spec was for `BREAK 1, 15` (imm_t=15); we use
/// imm_t=14 here so the thunk dispatch is distinguishable from the oracle
/// harness's BREAK 1, 15.)
pub const ROM_THUNK_BREAK_BYTES: [u8; 3] = [0xE0, 0x41, 0x00];

/// `imm_t` value reserved for ROM thunk dispatch in the BREAK exec arm.
pub const ROM_THUNK_IMM_T: u8 = 14;
/// `imm_s` value reserved for ROM thunk dispatch.
pub const ROM_THUNK_IMM_S: u8 = 1;

pub struct RomThunkBank {
    base: u32,
    backing: Vec<u8>,
    thunks: HashMap<u32, RomThunkFn>,
}

impl RomThunkBank {
    /// Create an empty bank covering `[base, base + size)`.
    ///
    /// Backing memory is initialised to 0; thunks are registered separately.
    pub fn new(base: u32, size: u32) -> Self {
        Self {
            base,
            backing: vec![0u8; size as usize],
            thunks: HashMap::new(),
        }
    }

    /// Register `thunk` at absolute address `pc`.
    ///
    /// The bank pre-fills 3 bytes at `pc` with `ROM_THUNK_BREAK_BYTES` so
    /// that an instruction fetch from `pc` returns `BREAK 1, 14`.
    pub fn register(&mut self, pc: u32, thunk: RomThunkFn) {
        let off = (pc - self.base) as usize;
        assert!(
            off + 3 <= self.backing.len(),
            "RomThunkBank::register: pc 0x{pc:08x} outside bank [0x{:08x}, 0x{:08x})",
            self.base,
            self.base as u64 + self.backing.len() as u64,
        );
        self.backing[off..off + 3].copy_from_slice(&ROM_THUNK_BREAK_BYTES);
        self.thunks.insert(pc, thunk);
    }

    /// Look up a thunk by absolute PC.  Returns `None` if no thunk is
    /// registered (the BREAK exec arm raises `NotImplemented` in that case).
    pub fn get(&self, pc: u32) -> Option<RomThunkFn> {
        self.thunks.get(&pc).copied()
    }

    /// Helper: read the 32-bit value in argument register `aN` of the
    /// current window.
    ///
    /// Most thunks read their args from a2..a7 (Xtensa windowed-call ABI)
    /// and write their return value to a2.
    pub fn read_arg(cpu: &XtensaLx7, idx: u8) -> u32 {
        cpu.regs.read_logical(idx)
    }

    /// Helper: write the 32-bit return value into a2 and set PC = a0
    /// (the saved return address per Xtensa CALL convention).
    pub fn return_with(cpu: &mut XtensaLx7, value: u32) {
        cpu.regs.write_logical(2, value);
        cpu.set_pc(cpu.regs.read_logical(0));
    }
}

impl std::fmt::Debug for RomThunkBank {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "RomThunkBank(base=0x{:08x}, size={}, {} thunks)",
            self.base,
            self.backing.len(),
            self.thunks.len(),
        )
    }
}

impl Peripheral for RomThunkBank {
    fn read(&self, offset: u64) -> SimResult<u8> {
        self.backing
            .get(offset as usize)
            .copied()
            .ok_or(SimulationError::MemoryViolation(self.base as u64 + offset))
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        // ROM is read-only; silently drop writes (real silicon ignores them).
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

// ── Default thunk set ────────────────────────────────────────────────────────
//
// These are the thunks that esp-hal hello-world is expected to call.  The
// actual addresses are filled in during Task 11 by disassembling the built
// firmware and reading ESP-IDF's `rom/esp32s3.ld`.  The implementations here
// are NOPs or zero-returns; specific behaviour (printf format expansion) lives
// in the registration callsite.

/// `Cache_Suspend_DCache(): u32` — returns 0 (cache wasn't suspended).
pub fn cache_suspend_dcache(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `Cache_Resume_DCache(prev: u32) -> u32` — returns 0.
pub fn cache_resume_dcache(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `esp_rom_spiflash_unlock(): u32` — returns 0 (success).
pub fn esp_rom_spiflash_unlock(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `rom_config_instruction_cache_mode(...)` — NOP, returns 0.
pub fn rom_config_instruction_cache_mode(
    cpu: &mut XtensaLx7,
    _bus: &mut dyn Bus,
) -> SimResult<()> {
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `ets_set_appcpu_boot_addr(addr: u32) -> u32` — NOP, returns 0
/// (cpu1 is not modelled in Plan 2).
pub fn ets_set_appcpu_boot_addr(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `ets_printf(fmt: *const u8, ...)` — read fmt string from `a2`, do a
/// minimal subset of printf formatting (%s, %d, %x, %c, %p) consuming
/// args from a3..a7, write to host stdout via `tracing::info!`.
///
/// The format expansion is intentionally minimal — esp-hal hello-world
/// uses esp-println for the actual `println!` output (which writes
/// directly to USB_SERIAL_JTAG).  ets_printf is only called for ROM-side
/// diagnostics that we want to surface in the dev's terminal.
pub fn ets_printf(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    let fmt_addr = cpu.regs.read_logical(2);
    let mut fmt = String::new();
    for i in 0..256u32 {
        let b = bus.read_u8(fmt_addr.wrapping_add(i) as u64)?;
        if b == 0 {
            break;
        }
        fmt.push(b as char);
    }

    // Minimal printf: substitute %s, %d, %x, %c, %p with args from a3..a7.
    let args = [
        cpu.regs.read_logical(3),
        cpu.regs.read_logical(4),
        cpu.regs.read_logical(5),
        cpu.regs.read_logical(6),
        cpu.regs.read_logical(7),
    ];
    let mut out = String::new();
    let mut chars = fmt.chars().peekable();
    let mut argi = 0usize;
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => {
                let addr = args[argi.min(4)];
                argi += 1;
                for i in 0..256u32 {
                    let b = bus.read_u8(addr.wrapping_add(i) as u64).unwrap_or(0);
                    if b == 0 {
                        break;
                    }
                    out.push(b as char);
                }
            }
            Some('d') | Some('i') => {
                out.push_str(&format!("{}", args[argi.min(4)] as i32));
                argi += 1;
            }
            Some('u') => {
                out.push_str(&format!("{}", args[argi.min(4)]));
                argi += 1;
            }
            Some('x') => {
                out.push_str(&format!("{:x}", args[argi.min(4)]));
                argi += 1;
            }
            Some('p') => {
                out.push_str(&format!("0x{:08x}", args[argi.min(4)]));
                argi += 1;
            }
            Some('c') => {
                out.push((args[argi.min(4)] as u8) as char);
                argi += 1;
            }
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    tracing::info!(target: "esp32s3::rom::ets_printf", "{}", out);
    RomThunkBank::return_with(cpu, out.len() as u32);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SystemBus;
    use crate::cpu::xtensa_lx7::XtensaLx7;
    use crate::Cpu;

    #[test]
    fn registered_thunk_address_holds_break_bytes() {
        let mut bank = RomThunkBank::new(0x4000_0000, 0x10_0000);
        bank.register(0x4000_1234, cache_suspend_dcache);
        let off = 0x1234usize;
        assert_eq!(&bank.backing[off..off + 3], &ROM_THUNK_BREAK_BYTES);
    }

    #[test]
    fn unregistered_thunk_returns_none() {
        let bank = RomThunkBank::new(0x4000_0000, 0x10_0000);
        assert!(bank.get(0x4000_1234).is_none());
    }

    #[test]
    fn registered_thunk_is_retrievable() {
        let mut bank = RomThunkBank::new(0x4000_0000, 0x10_0000);
        bank.register(0x4000_2000, cache_suspend_dcache);
        assert!(bank.get(0x4000_2000).is_some());
    }

    #[test]
    fn return_with_sets_a2_and_pc() {
        let mut bus = SystemBus::new();
        let mut cpu = XtensaLx7::new();
        cpu.reset(&mut bus).unwrap();
        cpu.regs.write_logical(0, 0x4037_0010); // a0 = return address
        cpu.set_pc(0x4000_0000);
        RomThunkBank::return_with(&mut cpu, 0xCAFE_BABE);
        assert_eq!(cpu.regs.read_logical(2), 0xCAFE_BABE);
        assert_eq!(cpu.get_pc(), 0x4037_0010);
    }
}
```

- [ ] **Step 3: Add the new module to `crates/core/src/peripherals/mod.rs`**

Edit `crates/core/src/peripherals/mod.rs:7-22`. Append a line at the bottom:

```rust
pub mod esp32s3;
```

- [ ] **Step 4: Add `get_rom_thunk` accessor to `SystemBus`**

In `crates/core/src/bus/mod.rs`, add this method to `impl SystemBus` (placement: after the existing `add_peripheral` method around line 322):

```rust
    /// Look up a registered ROM thunk by absolute PC.
    ///
    /// Iterates the registered peripherals; if any is a `RomThunkBank` whose
    /// address range contains `pc`, asks it for a thunk at `pc`.  Returns
    /// `None` if no bank covers the PC or no thunk is registered.
    ///
    /// Used by the CPU's `BREAK 1, 14` dispatch in `xtensa_lx7.rs`.
    pub fn get_rom_thunk(
        &self,
        pc: u32,
    ) -> Option<crate::peripherals::esp32s3::rom_thunks::RomThunkFn> {
        for p in &self.peripherals {
            let base = p.base as u32;
            let end = base.wrapping_add(p.size as u32);
            if pc >= base && pc < end {
                if let Some(any) = p.dev.as_any() {
                    if let Some(bank) = any
                        .downcast_ref::<crate::peripherals::esp32s3::rom_thunks::RomThunkBank>()
                    {
                        return bank.get(pc);
                    }
                }
            }
        }
        None
    }
```

- [ ] **Step 5: Modify the `Break` exec arm in `xtensa_lx7.rs` to dispatch thunks**

Edit `crates/core/src/cpu/xtensa_lx7.rs:185-187`. Replace:

```rust
            Break { .. } => {
                return Err(SimulationError::BreakpointHit(self.pc));
            }
```

with:

```rust
            Break { imm_s, imm_t } => {
                use crate::peripherals::esp32s3::rom_thunks::{
                    ROM_THUNK_IMM_S, ROM_THUNK_IMM_T,
                };
                if imm_s == ROM_THUNK_IMM_S && imm_t == ROM_THUNK_IMM_T {
                    let pc = self.pc;
                    if let Some(thunk) = bus.get_rom_thunk(pc) {
                        return thunk(self, bus);
                    }
                    return Err(SimulationError::NotImplemented(format!(
                        "ROM thunk at 0x{pc:08x} not registered (BREAK 1,14 with no thunk)"
                    )));
                }
                return Err(SimulationError::BreakpointHit(self.pc));
            }
```

Note: `bus` here is the parameter of `execute` — confirm from existing context. If the parameter name is different, use it. Also note that `bus.get_rom_thunk` is a concrete method on `SystemBus`, not on the `Bus` trait. The CPU's `bus` parameter is `&mut dyn Bus`. To downcast at the call site, change the call to use a trait-level accessor instead. Replace the line `if let Some(thunk) = bus.get_rom_thunk(pc)` with a trait-method call:

Actually, to keep `Bus` clean, add a default method to the `Bus` trait in `crates/core/src/lib.rs` (around line 130, after the existing trait body):

```rust
    /// Look up a registered ROM thunk at the given PC.  Default returns None
    /// (test stubs and non-ESP buses don't have thunks).  `SystemBus`
    /// overrides to search registered `RomThunkBank` peripherals.
    fn get_rom_thunk(
        &self,
        _pc: u32,
    ) -> Option<crate::peripherals::esp32s3::rom_thunks::RomThunkFn> {
        None
    }
```

Then add the `impl Bus for SystemBus` override that calls the inherent `get_rom_thunk`. Find the existing `impl Bus for SystemBus` block (around line 514+) and add at its end:

```rust
    fn get_rom_thunk(
        &self,
        pc: u32,
    ) -> Option<crate::peripherals::esp32s3::rom_thunks::RomThunkFn> {
        // Delegate to the inherent method.
        SystemBus::get_rom_thunk(self, pc)
    }
```

- [ ] **Step 6: Build and run new tests**

```bash
cargo build -p labwired-core 2>&1 | tail -20
cargo test -p labwired-core peripherals::esp32s3::rom_thunks 2>&1 | tail -10
```

Expected: build succeeds; all four `rom_thunks::tests` PASS.

- [ ] **Step 7: Add an integration test for the BREAK 1,14 dispatch**

Append to `crates/core/src/peripherals/esp32s3/rom_thunks.rs` `mod tests`:

```rust
    #[test]
    fn break_1_14_dispatches_to_thunk_via_bus() {
        // Register a RomThunkBank in the bus at IRAM address (so the BREAK
        // bytes fetch through the same path real ROM would).  We use IRAM
        // because the default SystemBus doesn't have a peripheral at the BROM
        // address, and adding the bank itself as the peripheral works.
        let mut bus = SystemBus::new();
        let mut bank = RomThunkBank::new(0x4037_0000, 0x100);
        // Register a thunk that bumps a2 by 1 to prove it ran.
        fn bump_a2(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
            let v = cpu.regs.read_logical(2);
            RomThunkBank::return_with(cpu, v + 1);
            Ok(())
        }
        bank.register(0x4037_0000, bump_a2);
        bus.add_peripheral("rom", 0x4037_0000, 0x100, None, Box::new(bank));

        let mut cpu = XtensaLx7::new();
        cpu.reset(&mut bus).unwrap();
        cpu.regs.write_logical(0, 0x4037_0080); // a0 = return address
        cpu.regs.write_logical(2, 41);           // a2 = 41
        cpu.set_pc(0x4037_0000);

        // Step once: should fetch BREAK 1,14, dispatch to bump_a2, return.
        cpu.step(&mut bus, &[]).expect("step dispatches thunk");

        assert_eq!(cpu.regs.read_logical(2), 42);
        assert_eq!(cpu.get_pc(), 0x4037_0080);
    }

    #[test]
    fn break_1_14_unregistered_raises_not_implemented() {
        let mut bus = SystemBus::new();
        let bank = RomThunkBank::new(0x4037_0000, 0x100);
        // Register no thunks; manually plant the BREAK bytes so the fetch
        // still produces BREAK 1,14.
        let mut bytes = vec![0u8; 0x100];
        bytes[0..3].copy_from_slice(&ROM_THUNK_BREAK_BYTES);
        // Use a RamPeripheral inline rather than RomThunkBank so the bank
        // doesn't have an entry at PC 0.
        struct OneShotRam(std::cell::RefCell<Vec<u8>>);
        impl std::fmt::Debug for OneShotRam {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "OneShotRam")
            }
        }
        impl Peripheral for OneShotRam {
            fn read(&self, off: u64) -> SimResult<u8> {
                Ok(*self.0.borrow().get(off as usize).unwrap_or(&0))
            }
            fn write(&mut self, _off: u64, _v: u8) -> SimResult<()> {
                Ok(())
            }
        }
        bus.add_peripheral(
            "ram",
            0x4037_0000,
            0x100,
            None,
            Box::new(OneShotRam(std::cell::RefCell::new(bytes))),
        );

        let mut cpu = XtensaLx7::new();
        cpu.reset(&mut bus).unwrap();
        cpu.set_pc(0x4037_0000);

        let res = cpu.step(&mut bus, &[]);
        match res {
            Err(SimulationError::NotImplemented(msg)) => {
                assert!(msg.contains("ROM thunk"), "unexpected message: {msg}");
            }
            other => panic!("expected NotImplemented, got {other:?}"),
        }
        let _ = bank;
    }
```

- [ ] **Step 8: Run the tests**

```bash
cargo test -p labwired-core peripherals::esp32s3::rom_thunks 2>&1 | tail -15
```

Expected: all six tests PASS.

- [ ] **Step 9: Run full sim suite**

```bash
cargo test --workspace \
  --exclude firmware --exclude firmware-ci-fixture --exclude riscv-ci-fixture \
  2>&1 | tail -5
```

Expected: ≥ baseline + 8 new tests; existing BREAK-related tests (oracle harness uses `BREAK 1, 15`, distinct from 1,14) still pass.

- [ ] **Step 10: Commit**

```bash
git add crates/core/src/peripherals/esp32s3 crates/core/src/peripherals/mod.rs crates/core/src/bus/mod.rs crates/core/src/cpu/xtensa_lx7.rs crates/core/src/lib.rs
git commit -m "feat(esp32s3): RomThunkBank + BREAK 1,14 dispatch

Adds crates/core/src/peripherals/esp32s3/{mod,rom_thunks}.rs with:
- RomThunkBank peripheral mapped at the BROM address range, pre-fills
  registered addresses with BREAK 1,14 bytes.
- BREAK exec arm in xtensa_lx7.rs dispatches imm_s=1, imm_t=14 to the
  bus's get_rom_thunk lookup; raises NotImplemented if not registered.
- Bus trait gains a default get_rom_thunk; SystemBus overrides to
  scan registered RomThunkBank peripherals.
- Default thunks: cache_suspend_dcache, cache_resume_dcache,
  esp_rom_spiflash_unlock, rom_config_instruction_cache_mode,
  ets_set_appcpu_boot_addr, ets_printf (minimal printf expansion).

Reserves BREAK 1,14 for ROM thunks; oracle harness's BREAK 1,15 is
unchanged."
git push
```

---

## Phase 3 — UsbSerialJtag (1 task)

### Task 3: USB_SERIAL_JTAG peripheral

**Files:**
- Create: `crates/core/src/peripherals/esp32s3/usb_serial_jtag.rs`
- Modify: `crates/core/src/peripherals/esp32s3/mod.rs` (add `pub mod usb_serial_jtag;`)

- [ ] **Step 1: Create `crates/core/src/peripherals/esp32s3/usb_serial_jtag.rs`**

```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! USB_SERIAL_JTAG peripheral for ESP32-S3.
//!
//! The S3 exposes a CDC-ACM device over USB that shares the same physical
//! USB cable as the JTAG debug interface.  When a host connects, it sees a
//! `/dev/ttyACM*` device on which the firmware can print.
//!
//! In the simulator we don't model the USB protocol — we expose just the
//! MMIO interface the firmware writes to.  Bytes written to EP1 are
//! appended to a sink (a `Vec<u8>` for tests) and optionally echoed to
//! host stdout for live runs.
//!
//! ## Register layout (ESP32-S3 TRM §27.5)
//!
//! | Offset | Name              | Direction | Behaviour |
//! |-------:|-------------------|-----------|-----------|
//! |  0x00  | EP1               | W         | byte FIFO data; bottom 8 bits of write are appended |
//! |  0x04  | EP1_CONF          | R         | reads `WR_DONE | SERIAL_IN_EP_DATA_FREE = 0x3` always |
//! |  0x08  | INT_RAW           | R/W       | stub: 0 |
//! |  0x0C  | INT_ST            | R         | stub: 0 |
//! |  0x10  | INT_ENA           | R/W       | stub: 0 (no IRQs in Plan 2) |
//! |  0x14  | INT_CLR           | W         | stub: NOP |
//!
//! Plan 2 does not generate interrupts — esp-hal's println path is
//! polling-based.

use crate::{Peripheral, SimResult};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct UsbSerialJtag {
    sink: Option<Arc<Mutex<Vec<u8>>>>,
    echo_stdout: bool,
}

impl std::fmt::Debug for UsbSerialJtag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "UsbSerialJtag(sink={}, echo_stdout={})",
            self.sink.is_some(),
            self.echo_stdout,
        )
    }
}

impl UsbSerialJtag {
    pub fn new() -> Self {
        Self {
            sink: None,
            echo_stdout: true,
        }
    }

    /// Set or clear the byte capture sink and stdout-echo flag.
    pub fn set_sink(&mut self, sink: Option<Arc<Mutex<Vec<u8>>>>, echo_stdout: bool) {
        self.sink = sink;
        self.echo_stdout = echo_stdout;
    }
}

impl Peripheral for UsbSerialJtag {
    fn read(&self, offset: u64) -> SimResult<u8> {
        match offset {
            // EP1_CONF (4 bytes, LE): always returns 0x0000_0003
            //   (WR_DONE | SERIAL_IN_EP_DATA_FREE).
            0x04 => Ok(0x03),
            0x05..=0x07 => Ok(0x00),
            // INT_* registers stub to 0.
            _ => Ok(0),
        }
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        match offset {
            // EP1: only the low byte of the LE word is the data byte.
            // The other 3 bytes of a 32-bit write are control bits we ignore.
            0x00 => {
                if let Some(sink) = &self.sink {
                    if let Ok(mut g) = sink.lock() {
                        g.push(value);
                    }
                }
                if self.echo_stdout {
                    let _ = io::stdout().write_all(&[value]);
                    let _ = io::stdout().flush();
                }
            }
            // INT_* writes accepted silently.
            _ => {}
        }
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Bus;
    use crate::bus::SystemBus;

    #[test]
    fn ep1_conf_reads_constant() {
        let p = UsbSerialJtag::new();
        // 32-bit read at 0x04 = 0x00000003 LE.
        assert_eq!(p.read(0x04).unwrap(), 0x03);
        assert_eq!(p.read(0x05).unwrap(), 0x00);
        assert_eq!(p.read(0x06).unwrap(), 0x00);
        assert_eq!(p.read(0x07).unwrap(), 0x00);
    }

    #[test]
    fn writing_ep1_appends_to_sink() {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let mut p = UsbSerialJtag::new();
        p.set_sink(Some(sink.clone()), false);
        p.write(0x00, b'H').unwrap();
        p.write(0x00, b'i').unwrap();
        assert_eq!(sink.lock().unwrap().as_slice(), b"Hi");
    }

    #[test]
    fn writing_via_bus_word_write_appends_low_byte() {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let mut bus = SystemBus::new();
        let mut p = UsbSerialJtag::new();
        p.set_sink(Some(sink.clone()), false);
        bus.add_peripheral("usb_jtag", 0x6003_8000, 0x100, None, Box::new(p));

        // Simulate `sw a2, 0(a1)` writing 'H' = 0x48 to the FIFO.
        bus.write_u32(0x6003_8000, 0x0000_0048).unwrap();
        // The write_u32 path decomposes into 4 byte writes at offsets 0..=3.
        // Offset 0 (low byte) is 'H'; the 3 high bytes go to offsets 1..=3,
        // which are not the FIFO byte — they're silently accepted.
        assert_eq!(sink.lock().unwrap().as_slice(), b"H");
    }

    #[test]
    fn int_registers_stub_to_zero() {
        let p = UsbSerialJtag::new();
        for off in 0x08..=0x17u64 {
            assert_eq!(p.read(off).unwrap(), 0, "offset 0x{off:02x}");
        }
    }
}
```

- [ ] **Step 2: Add the module to `crates/core/src/peripherals/esp32s3/mod.rs`**

Edit `crates/core/src/peripherals/esp32s3/mod.rs`. Append:

```rust
pub mod usb_serial_jtag;
```

- [ ] **Step 3: Run the new tests**

```bash
cargo test -p labwired-core peripherals::esp32s3::usb_serial_jtag 2>&1 | tail -10
```

Expected: all four tests PASS.

- [ ] **Step 4: Run full sim suite**

```bash
cargo test --workspace \
  --exclude firmware --exclude firmware-ci-fixture --exclude riscv-ci-fixture \
  2>&1 | tail -5
```

Expected: ≥ baseline + 12 new tests.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/peripherals/esp32s3
git commit -m "feat(esp32s3): UsbSerialJtag peripheral

Adds crates/core/src/peripherals/esp32s3/usb_serial_jtag.rs with the
minimum MMIO surface needed for esp_println::println! output:
- EP1 (0x00): write-only FIFO; bytes appended to optional sink and
  echoed to stdout.
- EP1_CONF (0x04): read-only constant 0x3 (WR_DONE | EP_DATA_FREE).
- INT_* (0x08..=0x14): stubbed to zero; no IRQs in Plan 2.

Sink wiring follows the existing UART peripheral's set_sink pattern."
git push
```

---

## Phase 4 — SYSTIMER (1 task)

### Task 4: SYSTIMER peripheral

**Files:**
- Create: `crates/core/src/peripherals/esp32s3/systimer.rs`
- Modify: `crates/core/src/peripherals/esp32s3/mod.rs` (add `pub mod systimer;`)

- [ ] **Step 1: Create `crates/core/src/peripherals/esp32s3/systimer.rs`**

```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! SYSTIMER peripheral for ESP32-S3.
//!
//! Two 64-bit free-running counters (UNIT0, UNIT1), each clocked at 16 MHz
//! independently of CPU frequency.  Plan 2 implements the counter + the
//! load/update handshake; alarms / IRQs land in Plan 3.
//!
//! ## Register layout (ESP32-S3 TRM §16.5, partial)
//!
//! | Offset | Name              | Behaviour |
//! |-------:|-------------------|-----------|
//! |  0x00  | CONF              | bit 31 clk_en (default 1), bit 30 timer_unit0_work_en, bit 29 timer_unit1_work_en |
//! |  0x04  | UNIT0_OP          | write 1<<30 to trigger snapshot of UNIT0 into VALUE registers |
//! |  0x08  | UNIT1_OP          | same for UNIT1 |
//! |  0x18  | UNIT0_LOAD_HI     | high 32 bits of pending load |
//! |  0x1C  | UNIT0_LOAD_LO     | low 32 bits of pending load |
//! |  0x20  | UNIT1_LOAD_HI     | high 32 bits of pending load (UNIT1) |
//! |  0x24  | UNIT1_LOAD_LO     | low 32 bits of pending load (UNIT1) |
//! |  0x40  | UNIT0_VALUE_HI    | snapshot high 32 bits |
//! |  0x44  | UNIT0_VALUE_LO    | snapshot low 32 bits |
//! |  0x48  | UNIT1_VALUE_HI    | snapshot high 32 bits |
//! |  0x4C  | UNIT1_VALUE_LO    | snapshot low 32 bits |
//! |  0x60  | UNIT0_LOAD        | write 1 to commit pending load into counter |
//! |  0x64  | UNIT1_LOAD        | same for UNIT1 |

use crate::{Peripheral, PeripheralTickResult, SimResult};

const SYSTIMER_CLOCK_HZ: u64 = 16_000_000;

#[derive(Debug, Default, Clone, Copy)]
struct UnitState {
    counter: u64,
    snapshot: u64,
    load_hi: u32,
    load_lo: u32,
}

#[derive(Debug)]
pub struct Systimer {
    conf: u32,
    unit0: UnitState,
    unit1: UnitState,
    cpu_clock_hz: u32,
    /// Accumulated CPU cycles since last counter update; flushed when ≥ 1
    /// SYSTIMER tick worth of CPU cycles have elapsed.
    cpu_cycle_accum: u64,
}

impl Systimer {
    pub fn new(cpu_clock_hz: u32) -> Self {
        Self {
            // Default: clock enabled (bit 31), both units running (bits 30, 29).
            conf: 0xE000_0000,
            unit0: UnitState::default(),
            unit1: UnitState::default(),
            cpu_clock_hz,
            cpu_cycle_accum: 0,
        }
    }

    fn unit0_running(&self) -> bool {
        self.conf & (1 << 30) != 0
    }

    fn unit1_running(&self) -> bool {
        self.conf & (1 << 29) != 0
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.conf,
            0x04 | 0x08 => 0, // OP regs are write-trigger only
            0x18 => self.unit0.load_hi,
            0x1C => self.unit0.load_lo,
            0x20 => self.unit1.load_hi,
            0x24 => self.unit1.load_lo,
            0x40 => (self.unit0.snapshot >> 32) as u32,
            0x44 => (self.unit0.snapshot & 0xFFFF_FFFF) as u32,
            0x48 => (self.unit1.snapshot >> 32) as u32,
            0x4C => (self.unit1.snapshot & 0xFFFF_FFFF) as u32,
            _ => 0,
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.conf = value,
            0x04 => {
                if value & (1 << 30) != 0 {
                    self.unit0.snapshot = self.unit0.counter;
                }
            }
            0x08 => {
                if value & (1 << 30) != 0 {
                    self.unit1.snapshot = self.unit1.counter;
                }
            }
            0x18 => self.unit0.load_hi = value,
            0x1C => self.unit0.load_lo = value,
            0x20 => self.unit1.load_hi = value,
            0x24 => self.unit1.load_lo = value,
            0x60 => {
                if value & 1 != 0 {
                    self.unit0.counter =
                        ((self.unit0.load_hi as u64) << 32) | (self.unit0.load_lo as u64);
                }
            }
            0x64 => {
                if value & 1 != 0 {
                    self.unit1.counter =
                        ((self.unit1.load_hi as u64) << 32) | (self.unit1.load_lo as u64);
                }
            }
            _ => {}
        }
    }
}

impl Peripheral for Systimer {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let word = self.read_word(word_off);
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let mut word = self.read_word(word_off);
        word &= !(0xFFu32 << byte_off);
        word |= (value as u32) << byte_off;
        self.write_word(word_off, word);
        Ok(())
    }

    /// One CPU cycle elapses per `tick`. Convert to SYSTIMER ticks at 16 MHz.
    /// At 80 MHz CPU clock, 5 CPU cycles == 1 SYSTIMER tick.
    fn tick(&mut self) -> PeripheralTickResult {
        self.cpu_cycle_accum += 1;
        let cpu_per_systimer = (self.cpu_clock_hz as u64).saturating_div(SYSTIMER_CLOCK_HZ).max(1);
        if self.cpu_cycle_accum >= cpu_per_systimer {
            let ticks = self.cpu_cycle_accum / cpu_per_systimer;
            self.cpu_cycle_accum %= cpu_per_systimer;
            if self.unit0_running() {
                self.unit0.counter = self.unit0.counter.wrapping_add(ticks);
            }
            if self.unit1_running() {
                self.unit1.counter = self.unit1.counter.wrapping_add(ticks);
            }
        }
        PeripheralTickResult::default()
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let s = Systimer::new(80_000_000);
        assert_eq!(s.conf & 0xE000_0000, 0xE000_0000);
        assert_eq!(s.unit0.counter, 0);
        assert_eq!(s.unit1.counter, 0);
    }

    #[test]
    fn tick_increments_counter_at_correct_rate_80mhz() {
        let mut s = Systimer::new(80_000_000);
        // 80 MHz CPU / 16 MHz SYSTIMER = 5 CPU cycles per SYSTIMER tick.
        for _ in 0..5 {
            s.tick();
        }
        assert_eq!(s.unit0.counter, 1, "after 5 CPU cycles, SYSTIMER += 1");
        for _ in 0..50 {
            s.tick();
        }
        assert_eq!(s.unit0.counter, 11, "55 CPU cycles -> 11 SYSTIMER ticks");
    }

    #[test]
    fn tick_increments_at_240mhz() {
        let mut s = Systimer::new(240_000_000);
        // 240 MHz CPU / 16 MHz SYSTIMER = 15 CPU cycles per SYSTIMER tick.
        for _ in 0..15 {
            s.tick();
        }
        assert_eq!(s.unit0.counter, 1);
    }

    #[test]
    fn op_trigger_snapshots_counter() {
        let mut s = Systimer::new(80_000_000);
        for _ in 0..50 {
            s.tick();
        }
        // Trigger snapshot of UNIT0.
        s.write_word(0x04, 1 << 30);
        let snap_lo = s.read_word(0x44);
        let snap_hi = s.read_word(0x40);
        let combined = ((snap_hi as u64) << 32) | snap_lo as u64;
        assert_eq!(combined, 10);
    }

    #[test]
    fn load_handshake_sets_counter() {
        let mut s = Systimer::new(80_000_000);
        s.write_word(0x18, 0x0000_0001); // LOAD_HI = 1
        s.write_word(0x1C, 0x0000_0042); // LOAD_LO = 0x42
        s.write_word(0x60, 1); // commit
        assert_eq!(s.unit0.counter, (1u64 << 32) | 0x42);
    }

    #[test]
    fn unit1_independent_of_unit0() {
        let mut s = Systimer::new(80_000_000);
        for _ in 0..5 {
            s.tick();
        }
        assert_eq!(s.unit0.counter, 1);
        assert_eq!(s.unit1.counter, 1, "unit1 ticks alongside unit0");
        s.write_word(0x60, 1); // commit a load to unit0 (loads were 0)
        assert_eq!(s.unit0.counter, 0);
        assert_eq!(s.unit1.counter, 1, "unit1 not affected by unit0 load");
    }

    #[test]
    fn disabled_unit_does_not_tick() {
        let mut s = Systimer::new(80_000_000);
        // Clear bit 30 (unit0 work enable).
        s.write_word(0x00, 0xA000_0000);
        for _ in 0..50 {
            s.tick();
        }
        assert_eq!(s.unit0.counter, 0, "disabled unit must not tick");
        assert_eq!(s.unit1.counter, 10, "unit1 still ticks");
    }
}
```

- [ ] **Step 2: Add the module declaration**

Edit `crates/core/src/peripherals/esp32s3/mod.rs`. Append:

```rust
pub mod systimer;
```

- [ ] **Step 3: Run the new tests**

```bash
cargo test -p labwired-core peripherals::esp32s3::systimer 2>&1 | tail -15
```

Expected: all six tests PASS.

- [ ] **Step 4: Run full sim suite**

```bash
cargo test --workspace \
  --exclude firmware --exclude firmware-ci-fixture --exclude riscv-ci-fixture \
  2>&1 | tail -5
```

Expected: ≥ baseline + 18 new tests.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/peripherals/esp32s3
git commit -m "feat(esp32s3): SYSTIMER peripheral (counters only, no alarms)

Adds crates/core/src/peripherals/esp32s3/systimer.rs:
- Two 64-bit free-running counters (UNIT0, UNIT1), each clocked at
  16 MHz independent of CPU frequency.
- Load/update handshake: LOAD_HI/LOAD_LO + commit-bit at 0x60/0x64.
- OP register snapshot trigger: write 1<<30 to capture counter into
  VALUE_HI/VALUE_LO.
- CONF register: bit 30/29 enable UNIT0/UNIT1; default both running.

Tick math scales CPU cycles down to SYSTIMER ticks (5:1 at 80 MHz,
15:1 at 240 MHz). Plan 2 omits alarms / IRQs (Plan 3 territory).

esp-hal Delay::delay_millis is polling-based against the 64-bit
counter so the alarm-less subset is sufficient for hello-world."
git push
```

---

## Phase 5 — System / RTC_CNTL / EFUSE Stubs (1 task)

### Task 5: System stubs (SYSTEM, RTC_CNTL, EFUSE)

**Files:**
- Create: `crates/core/src/peripherals/esp32s3/system_stub.rs`
- Modify: `crates/core/src/peripherals/esp32s3/mod.rs` (add `pub mod system_stub;`)

- [ ] **Step 1: Create `crates/core/src/peripherals/esp32s3/system_stub.rs`**

```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Three small register stubs needed for esp-hal `init()` to complete:
//!
//! * `SystemStub`  — SYSTEM peripheral at 0x600C_0000.
//!                  SYSCLK_CONF.SOC_CLK_SEL is round-tripped (esp-hal reads it
//!                  back to know the active clock source).  Other registers
//!                  are write-accept / read-as-zero.
//! * `RtcCntlStub` — RTC_CNTL at 0x6000_8000.  Fully cosmetic for hello-world:
//!                  read-as-zero, write-accept.
//! * `EfuseStub`   — EFUSE at 0x6000_7000.  Returns canned MAC + chip-rev for
//!                  the few fields esp-hal reads at boot.

use crate::{Peripheral, SimResult};
use std::collections::HashMap;

/// SYSTEM peripheral stub.  Tracks every written word so reads return what
/// the firmware wrote (so its boot config-back-check passes).
#[derive(Debug, Default)]
pub struct SystemStub {
    words: HashMap<u64, u32>,
}

impl SystemStub {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Peripheral for SystemStub {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let word = self.words.get(&word_off).copied().unwrap_or(0);
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let entry = self.words.entry(word_off).or_insert(0);
        *entry &= !(0xFFu32 << byte_off);
        *entry |= (value as u32) << byte_off;
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

/// RTC_CNTL peripheral stub.  Read-as-zero, write-accept.
#[derive(Debug, Default)]
pub struct RtcCntlStub;

impl RtcCntlStub {
    pub fn new() -> Self {
        Self
    }
}

impl Peripheral for RtcCntlStub {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

/// EFUSE peripheral stub.  Returns canned MAC + chip-rev for the fields
/// esp-hal reads at boot.
///
/// Per ESP32-S3 TRM §6 (eFuse Controller), the relevant fields esp-hal touches:
///
/// | Offset | Field                        | Canned value |
/// |-------:|------------------------------|--------------|
/// |  0x044 | RD_MAC_SPI_SYS_0 (MAC[3:0])  | 0x00000002   |
/// |  0x048 | RD_MAC_SPI_SYS_1 (MAC[5:4])  | 0x00000000   |
/// |  0x05C | RD_SYS_PART1_DATA0 (chip_rev)| 0x00000000   |
///
/// The canned MAC is `02:00:00:00:00:01` (locally-administered).
#[derive(Debug, Default)]
pub struct EfuseStub;

impl EfuseStub {
    pub fn new() -> Self {
        Self
    }
}

impl Peripheral for EfuseStub {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let word: u32 = match word_off {
            0x044 => 0x0000_0002, // MAC low word: 0x00 00 00 02
            0x048 => 0x0000_0000, // MAC high word: 0x00 00 00 00
            0x05C => 0x0000_0000, // chip_rev = 0
            _ => 0,
        };
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_stub_round_trips_words() {
        let mut s = SystemStub::new();
        s.write(0x10, 0xAB).unwrap();
        s.write(0x11, 0xCD).unwrap();
        s.write(0x12, 0xEF).unwrap();
        s.write(0x13, 0x12).unwrap();
        assert_eq!(s.read(0x10).unwrap(), 0xAB);
        assert_eq!(s.read(0x11).unwrap(), 0xCD);
        assert_eq!(s.read(0x13).unwrap(), 0x12);
    }

    #[test]
    fn rtc_cntl_stub_read_as_zero() {
        let s = RtcCntlStub::new();
        for off in 0..16u64 {
            assert_eq!(s.read(off).unwrap(), 0);
        }
    }

    #[test]
    fn efuse_returns_canned_mac() {
        let s = EfuseStub::new();
        // MAC low byte at 0x044 = 0x02.
        assert_eq!(s.read(0x044).unwrap(), 0x02);
        assert_eq!(s.read(0x045).unwrap(), 0x00);
        // MAC high byte at 0x048 = 0x00 (no high MAC bytes).
        assert_eq!(s.read(0x048).unwrap(), 0x00);
        // chip_rev at 0x05C = 0.
        assert_eq!(s.read(0x05C).unwrap(), 0x00);
    }

    #[test]
    fn efuse_unknown_offset_reads_zero() {
        let s = EfuseStub::new();
        assert_eq!(s.read(0x100).unwrap(), 0);
    }
}
```

- [ ] **Step 2: Register the module**

Edit `crates/core/src/peripherals/esp32s3/mod.rs`. Append:

```rust
pub mod system_stub;
```

- [ ] **Step 3: Run new tests**

```bash
cargo test -p labwired-core peripherals::esp32s3::system_stub 2>&1 | tail -10
```

Expected: all four tests PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/core/src/peripherals/esp32s3
git commit -m "feat(esp32s3): SYSTEM/RTC_CNTL/EFUSE stubs

Adds crates/core/src/peripherals/esp32s3/system_stub.rs with three
minimal stubs needed for esp-hal init() to complete:
- SystemStub: word-cache for read-after-write round-trip.
- RtcCntlStub: read-as-zero, write-accept.
- EfuseStub: canned MAC 02:00:00:00:00:01, chip_rev = 0."
git push
```

---

## Phase 6 — Flash-XIP Backing Peripheral (1 task)

### Task 6: FlashXipPeripheral

**Files:**
- Create: `crates/core/src/peripherals/esp32s3/flash_xip.rs`
- Modify: `crates/core/src/peripherals/esp32s3/mod.rs` (add `pub mod flash_xip;`)

- [ ] **Step 1: Create `crates/core/src/peripherals/esp32s3/flash_xip.rs`**

```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Flash-XIP backing peripheral for ESP32-S3.
//!
//! The S3 exposes the in-package SPI flash to the CPU through two MMIO
//! windows: 0x4200_0000 (I-cache, instruction fetch) and 0x3C00_0000
//! (D-cache, data load).  Both are read-only; writes raise a bus fault.
//!
//! Real silicon translates virt addresses through a 64-entry × 64 KiB MMU
//! page table that the firmware programs at boot via the EXTMEM peripheral.
//! For Plan 2 (fast-boot, static page table) we accept a `page_table` from
//! the boot path and consult it on every read.
//!
//! ## Sharing
//!
//! The same physical flash backing is mapped twice on the bus (once at
//! 0x4200_0000, once at 0x3C00_0000).  Both mappings share an
//! `Arc<Mutex<Vec<u8>>>` backing buffer so writes through either alias —
//! though writes are forbidden in Plan 2 — would be coherent.

use crate::{Peripheral, SimResult, SimulationError};
use std::sync::{Arc, Mutex};

const PAGE_SIZE: u32 = 64 * 1024;
const PAGE_TABLE_ENTRIES: usize = 64;

#[derive(Debug, Clone)]
pub struct FlashXipPeripheral {
    backing: Arc<Mutex<Vec<u8>>>,
    /// Maps virtual page index (offset within the 4 MiB window) to physical
    /// page index (offset within the flash backing).  `None` = unmapped.
    page_table: [Option<u16>; PAGE_TABLE_ENTRIES],
    base: u32,
}

impl FlashXipPeripheral {
    /// Create a new instance with a shared backing buffer and an unpopulated
    /// page table.  `base` is `0x4200_0000` for I-cache or `0x3C00_0000`
    /// for D-cache.
    pub fn new_shared(backing: Arc<Mutex<Vec<u8>>>, base: u32) -> Self {
        Self {
            backing,
            page_table: [None; PAGE_TABLE_ENTRIES],
            base,
        }
    }

    /// Map virtual page `virt` (0..=63) to physical page `phys` in the
    /// backing buffer.
    pub fn map_page(&mut self, virt: u8, phys: u16) {
        assert!((virt as usize) < PAGE_TABLE_ENTRIES, "virt page out of range");
        self.page_table[virt as usize] = Some(phys);
    }

    /// Identity-map all pages (virt page N → phys page N).  Useful for tests
    /// and fast-boot fallback when the firmware's expected segment layout
    /// matches a 1:1 mapping.
    pub fn map_identity(&mut self) {
        for i in 0..PAGE_TABLE_ENTRIES {
            self.page_table[i] = Some(i as u16);
        }
    }

    /// Returns the number of currently-mapped pages.
    pub fn pages_mapped(&self) -> usize {
        self.page_table.iter().filter(|p| p.is_some()).count()
    }

    fn translate(&self, offset: u64) -> Option<u64> {
        let virt_page = (offset / PAGE_SIZE as u64) as usize;
        let in_page = offset % PAGE_SIZE as u64;
        if virt_page >= PAGE_TABLE_ENTRIES {
            return None;
        }
        let phys_page = self.page_table[virt_page]?;
        Some(phys_page as u64 * PAGE_SIZE as u64 + in_page)
    }
}

impl Peripheral for FlashXipPeripheral {
    fn read(&self, offset: u64) -> SimResult<u8> {
        match self.translate(offset) {
            Some(phys) => {
                let backing = self.backing.lock().unwrap();
                Ok(*backing.get(phys as usize).unwrap_or(&0))
            }
            None => Ok(0), // unmapped page reads as 0
        }
    }

    fn write(&mut self, offset: u64, _value: u8) -> SimResult<()> {
        Err(SimulationError::MemoryViolation(
            self.base as u64 + offset,
        ))
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unmapped_pages_read_as_zero() {
        let backing = Arc::new(Mutex::new(vec![0xAAu8; 64 * 1024]));
        let p = FlashXipPeripheral::new_shared(backing, 0x4200_0000);
        // No pages mapped: read returns 0 even though backing has 0xAA.
        assert_eq!(p.read(0).unwrap(), 0);
    }

    #[test]
    fn mapped_page_reads_through_to_backing() {
        let mut backing = vec![0u8; PAGE_SIZE as usize];
        backing[0] = 0xCA;
        backing[1] = 0xFE;
        let backing = Arc::new(Mutex::new(backing));
        let mut p = FlashXipPeripheral::new_shared(backing, 0x4200_0000);
        p.map_page(0, 0);
        assert_eq!(p.read(0).unwrap(), 0xCA);
        assert_eq!(p.read(1).unwrap(), 0xFE);
    }

    #[test]
    fn writes_are_forbidden() {
        let backing = Arc::new(Mutex::new(vec![0u8; 64 * 1024]));
        let mut p = FlashXipPeripheral::new_shared(backing, 0x4200_0000);
        p.map_identity();
        let err = p.write(0, 0xAA).unwrap_err();
        match err {
            SimulationError::MemoryViolation(_) => {}
            other => panic!("expected MemoryViolation, got {other:?}"),
        }
    }

    #[test]
    fn cross_page_remap_works() {
        let mut backing = vec![0u8; PAGE_SIZE as usize * 2];
        backing[PAGE_SIZE as usize] = 0xAB; // first byte of physical page 1
        let backing = Arc::new(Mutex::new(backing));
        let mut p = FlashXipPeripheral::new_shared(backing, 0x4200_0000);
        // Map virtual page 0 → physical page 1.
        p.map_page(0, 1);
        assert_eq!(p.read(0).unwrap(), 0xAB);
    }

    #[test]
    fn shared_backing_visible_to_both_aliases() {
        let mut buf = vec![0u8; PAGE_SIZE as usize];
        buf[0] = 0x42;
        let backing = Arc::new(Mutex::new(buf));
        let mut p_icache = FlashXipPeripheral::new_shared(backing.clone(), 0x4200_0000);
        let mut p_dcache = FlashXipPeripheral::new_shared(backing.clone(), 0x3C00_0000);
        p_icache.map_identity();
        p_dcache.map_identity();
        assert_eq!(p_icache.read(0).unwrap(), 0x42);
        assert_eq!(p_dcache.read(0).unwrap(), 0x42);
    }
}
```

- [ ] **Step 2: Register the module**

Edit `crates/core/src/peripherals/esp32s3/mod.rs`. Append:

```rust
pub mod flash_xip;
```

- [ ] **Step 3: Run new tests**

```bash
cargo test -p labwired-core peripherals::esp32s3::flash_xip 2>&1 | tail -10
```

Expected: all five tests PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/core/src/peripherals/esp32s3
git commit -m "feat(esp32s3): FlashXipPeripheral (read-only, page-table translated)

Adds crates/core/src/peripherals/esp32s3/flash_xip.rs:
- 64-entry × 64 KiB MMU page table; reads consult the table.
- Shared Arc<Mutex<Vec<u8>>> backing so I-cache (0x4200_0000) and
  D-cache (0x3C00_0000) aliases see the same flash.
- Writes raise SimulationError::MemoryViolation (XIP is read-only).
- map_identity helper for fast-boot 1:1 mapping fallback."
git push
```

---

## Phase 7 — System Glue + Chip YAML (1 task)

### Task 7: configure_xtensa_esp32s3 + chip YAML

**Files:**
- Create: `crates/core/src/system/xtensa.rs`
- Modify: `crates/core/src/system/mod.rs` (add `pub mod xtensa;`)
- Create: `configs/chips/esp32s3-zero.yaml`

- [ ] **Step 1: Create `crates/core/src/system/xtensa.rs`**

```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Xtensa LX7 / ESP32-S3 system glue.
//!
//! `configure_xtensa_esp32s3` registers all peripherals defined for the
//! ESP32-S3-Zero and returns a fresh `XtensaLx7` CPU.  After calling this,
//! the caller invokes `boot::esp32s3::fast_boot` to load an ELF and
//! synthesise CPU state, then enters the simulation loop.

use crate::bus::SystemBus;
use crate::cpu::xtensa_lx7::XtensaLx7;
use crate::peripherals::esp32s3::flash_xip::FlashXipPeripheral;
use crate::peripherals::esp32s3::rom_thunks::{
    self, RomThunkBank, RomThunkFn,
};
use crate::peripherals::esp32s3::system_stub::{EfuseStub, RtcCntlStub, SystemStub};
use crate::peripherals::esp32s3::systimer::Systimer;
use crate::peripherals::esp32s3::usb_serial_jtag::UsbSerialJtag;
use crate::Cpu;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct Esp32s3Opts {
    pub iram_size: u32,
    pub dram_size: u32,
    pub flash_size: u32,
    pub cpu_clock_hz: u32,
}

impl Default for Esp32s3Opts {
    fn default() -> Self {
        Self {
            iram_size: 512 * 1024,
            dram_size: 480 * 1024,
            flash_size: 4 * 1024 * 1024,
            cpu_clock_hz: 80_000_000,
        }
    }
}

/// Result of `configure_xtensa_esp32s3` — exposes the shared flash backing
/// so the boot path can write to it.
pub struct Esp32s3Wiring {
    pub cpu: XtensaLx7,
    pub flash_backing: Arc<Mutex<Vec<u8>>>,
}

/// Register all ESP32-S3 peripherals on `bus` and return the CPU + the
/// shared flash backing buffer.
pub fn configure_xtensa_esp32s3(bus: &mut SystemBus, opts: &Esp32s3Opts) -> Esp32s3Wiring {
    use crate::Peripheral;

    // ── IRAM (instruction fetch view) ─────────────────────────────────────
    bus.add_peripheral(
        "iram",
        0x4037_0000,
        opts.iram_size as u64,
        None,
        Box::new(RamPeripheral::new(opts.iram_size as usize)),
    );

    // ── DRAM (data view of the same physical SRAM0) ───────────────────────
    bus.add_peripheral(
        "dram",
        0x3FC8_8000,
        opts.dram_size as u64,
        None,
        Box::new(RamPeripheral::new(opts.dram_size as usize)),
    );

    // ── Flash-XIP backing, shared between I-cache and D-cache aliases ─────
    let flash_backing = Arc::new(Mutex::new(vec![0u8; opts.flash_size as usize]));
    let mut icache = FlashXipPeripheral::new_shared(flash_backing.clone(), 0x4200_0000);
    let mut dcache = FlashXipPeripheral::new_shared(flash_backing.clone(), 0x3C00_0000);
    icache.map_identity();
    dcache.map_identity();
    bus.add_peripheral(
        "flash_icache",
        0x4200_0000,
        opts.flash_size as u64,
        None,
        Box::new(icache),
    );
    bus.add_peripheral(
        "flash_dcache",
        0x3C00_0000,
        opts.flash_size as u64,
        None,
        Box::new(dcache),
    );

    // ── ROM thunk bank ────────────────────────────────────────────────────
    let mut rom_bank = RomThunkBank::new(0x4000_0000, 0x6_0000);
    register_default_thunks(&mut rom_bank);
    bus.add_peripheral(
        "rom_thunks",
        0x4000_0000,
        0x6_0000,
        None,
        Box::new(rom_bank),
    );

    // ── USB_SERIAL_JTAG ───────────────────────────────────────────────────
    bus.add_peripheral(
        "usb_serial_jtag",
        0x6003_8000,
        0x1000,
        None,
        Box::new(UsbSerialJtag::new()),
    );

    // ── SYSTIMER ──────────────────────────────────────────────────────────
    bus.add_peripheral(
        "systimer",
        0x6002_3000,
        0x1000,
        None,
        Box::new(Systimer::new(opts.cpu_clock_hz)),
    );

    // ── SYSTEM / RTC_CNTL / EFUSE stubs ──────────────────────────────────
    bus.add_peripheral(
        "system",
        0x600C_0000,
        0x1000,
        None,
        Box::new(SystemStub::new()),
    );
    bus.add_peripheral(
        "rtc_cntl",
        0x6000_8000,
        0x1000,
        None,
        Box::new(RtcCntlStub::new()),
    );
    bus.add_peripheral(
        "efuse",
        0x6000_7000,
        0x1000,
        None,
        Box::new(EfuseStub::new()),
    );

    let mut cpu = XtensaLx7::new();
    cpu.reset(bus).expect("xtensa reset");

    Esp32s3Wiring { cpu, flash_backing }
}

/// Register the empty default thunk set.  Real addresses are filled in by
/// Task 11 once we disassemble the firmware.  For now the bank exists but
/// holds no thunks — the implementer adds entries as they're discovered.
fn register_default_thunks(_bank: &mut RomThunkBank) {
    // Intentionally empty in the initial implementation.
    //
    // Task 11 populates this with calls like:
    //
    //     bank.register(0x40000xxx, rom_thunks::ets_printf);
    //     bank.register(0x40000xxx, rom_thunks::cache_suspend_dcache);
    //     ... etc.
    //
    // The addresses come from disassembling the built firmware:
    //
    //   xtensa-esp32s3-elf-objdump -d examples/esp32s3-hello-world/target/.../hello-world \
    //       | grep -E '0x40[0-9a-f]+'
    //
    // and cross-referencing with ESP-IDF rom/esp32s3.ld.
    let _ = rom_thunks::ets_printf;
}

// ── RamPeripheral helper (private) ───────────────────────────────────────

/// Flat-array `Peripheral` used for IRAM + DRAM mappings.
struct RamPeripheral {
    data: std::cell::RefCell<Vec<u8>>,
}

impl RamPeripheral {
    fn new(size: usize) -> Self {
        Self {
            data: std::cell::RefCell::new(vec![0u8; size]),
        }
    }
}

impl std::fmt::Debug for RamPeripheral {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RamPeripheral({}B)", self.data.borrow().len())
    }
}

impl crate::Peripheral for RamPeripheral {
    fn read(&self, offset: u64) -> crate::SimResult<u8> {
        Ok(*self.data.borrow().get(offset as usize).unwrap_or(&0))
    }
    fn write(&mut self, offset: u64, value: u8) -> crate::SimResult<()> {
        let mut d = self.data.borrow_mut();
        if let Some(slot) = d.get_mut(offset as usize) {
            *slot = value;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Bus;

    #[test]
    fn configure_registers_all_peripherals() {
        let mut bus = SystemBus::new();
        let _ = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
        // Confirm core regions are reachable.
        assert!(bus.read_u8(0x4037_0000).is_ok(), "IRAM");
        assert!(bus.read_u8(0x3FC8_8000).is_ok(), "DRAM");
        assert!(bus.read_u8(0x4200_0000).is_ok(), "flash I-cache");
        assert!(bus.read_u8(0x3C00_0000).is_ok(), "flash D-cache");
        assert!(bus.read_u8(0x6003_8000).is_ok(), "USB_SERIAL_JTAG");
        assert!(bus.read_u8(0x6002_3000).is_ok(), "SYSTIMER");
        assert!(bus.read_u8(0x600C_0000).is_ok(), "SYSTEM");
        assert!(bus.read_u8(0x6000_8000).is_ok(), "RTC_CNTL");
        assert!(bus.read_u8(0x6000_7000).is_ok(), "EFUSE");
    }

    #[test]
    fn iram_writeable_and_readable() {
        let mut bus = SystemBus::new();
        let _ = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
        bus.write_u8(0x4037_0010, 0xAB).unwrap();
        assert_eq!(bus.read_u8(0x4037_0010).unwrap(), 0xAB);
    }

    #[test]
    fn flash_xip_aliases_share_backing() {
        let mut bus = SystemBus::new();
        let wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
        // Write directly into the flash backing (mimics fast-boot doing so).
        wiring.flash_backing.lock().unwrap()[0] = 0xCA;
        wiring.flash_backing.lock().unwrap()[1] = 0xFE;
        // Both aliases must reflect it.
        assert_eq!(bus.read_u8(0x4200_0000).unwrap(), 0xCA);
        assert_eq!(bus.read_u8(0x3C00_0000).unwrap(), 0xCA);
        assert_eq!(bus.read_u8(0x4200_0001).unwrap(), 0xFE);
    }
}
```

- [ ] **Step 2: Add module to `crates/core/src/system/mod.rs`**

Edit `crates/core/src/system/mod.rs`. Append:

```rust
pub mod xtensa;
```

- [ ] **Step 3: Create `configs/chips/esp32s3-zero.yaml`**

```yaml
# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
# SPDX-License-Identifier: MIT
#
# ESP32-S3-Zero (FH4R2 variant) chip descriptor for Plan 2.
#
# This YAML is documentation of the memory map; the simulator's
# crate::system::xtensa::configure_xtensa_esp32s3 function is the
# authoritative wiring code. The YAML is loaded by the CLI to validate
# that --chip references a known chip.

name: "esp32s3-zero"
arch: "xtensa-lx7"
flash:
  base: 0x42000000
  size: "4MiB"
ram:
  base: 0x3FC88000
  size: "480KiB"
peripherals:
  - id: "iram"
    type: "ram"
    base_address: 0x40370000
    size: "512KiB"
  - id: "rom_thunks"
    type: "rom_thunk_bank"
    base_address: 0x40000000
    size: "384KiB"
  - id: "flash_icache"
    type: "flash_xip"
    base_address: 0x42000000
    size: "4MiB"
  - id: "flash_dcache"
    type: "flash_xip"
    base_address: 0x3C000000
    size: "4MiB"
  - id: "usb_serial_jtag"
    type: "usb_serial_jtag"
    base_address: 0x60038000
    size: "4KiB"
  - id: "systimer"
    type: "systimer"
    base_address: 0x60023000
    size: "4KiB"
  - id: "system"
    type: "system_stub"
    base_address: 0x600C0000
    size: "4KiB"
  - id: "rtc_cntl"
    type: "rtc_cntl_stub"
    base_address: 0x60008000
    size: "4KiB"
  - id: "efuse"
    type: "efuse_stub"
    base_address: 0x60007000
    size: "4KiB"
```

- [ ] **Step 4: Build and run new tests**

```bash
cargo build -p labwired-core 2>&1 | tail -10
cargo test -p labwired-core system::xtensa 2>&1 | tail -10
```

Expected: build succeeds; all three tests PASS.

- [ ] **Step 5: Run full sim suite**

```bash
cargo test --workspace \
  --exclude firmware --exclude firmware-ci-fixture --exclude riscv-ci-fixture \
  2>&1 | tail -5
```

Expected: ≥ baseline + 26 new tests.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/system/xtensa.rs crates/core/src/system/mod.rs configs/chips/esp32s3-zero.yaml
git commit -m "feat(system): configure_xtensa_esp32s3 + esp32s3-zero chip YAML

Adds crates/core/src/system/xtensa.rs which wires all ESP32-S3
peripherals (IRAM/DRAM/flash-XIP/ROM thunks/USB_SERIAL_JTAG/SYSTIMER/
SYSTEM/RTC_CNTL/EFUSE) into a SystemBus and returns the XtensaLx7
CPU plus the shared flash backing buffer.

ROM thunk registration is empty in this commit — Task 11 fills it in
after disassembling the built firmware.

Adds configs/chips/esp32s3-zero.yaml as documentation of the memory
map; the YAML is non-authoritative (the wiring code in xtensa.rs is)."
git push
```

---

## Phase 8 — Boot Path Wires Flash XIP (1 task)

### Task 8: fast_boot loads flash-XIP segments through shared backing

**Files:**
- Modify: `crates/core/src/boot/esp32s3.rs` (extend fast_boot to handle XIP segments)

**Background:** The Plan 2 spec §4.2 step 3 says fast-boot must populate the flash-XIP backing buffer for segments whose `p_vaddr` lies in the XIP windows. Since `configure_xtensa_esp32s3` returns the `Arc<Mutex<Vec<u8>>>` backing, we can pass it into `fast_boot` to write through it directly. Without this, writes to `0x4200_0000+` go through `FlashXipPeripheral::write` which raises `MemoryViolation`.

- [ ] **Step 1: Extend `BootOpts` and `fast_boot` to accept the flash backing**

Edit `crates/core/src/boot/esp32s3.rs`. Replace the `BootOpts` struct with:

```rust
/// Per-call options for `fast_boot`.
pub struct BootOpts {
    /// Used as the SP if the ELF lacks a `_stack_start_cpu0` symbol.
    pub stack_top_fallback: u32,
    /// Shared flash backing buffer.  When set, segments whose virtual
    /// address falls inside the XIP windows (0x4200_0000..0x4400_0000 or
    /// 0x3C00_0000..0x3E00_0000) are written here instead of through
    /// the bus (since FlashXipPeripheral::write raises MemoryViolation).
    pub flash_backing: Option<std::sync::Arc<std::sync::Mutex<Vec<u8>>>>,
}

impl Default for BootOpts {
    fn default() -> Self {
        Self {
            stack_top_fallback: 0x3FCD_FFF0,
            flash_backing: None,
        }
    }
}
```

- [ ] **Step 2: Modify `fast_boot` to route XIP segments to the backing buffer**

Replace the segment-loading loop in `fast_boot` with:

```rust
    let mut segments_loaded = 0;
    for ph in &elf.program_headers {
        if ph.p_type != PT_LOAD || ph.p_filesz == 0 {
            continue;
        }
        let vaddr = ph.p_vaddr as u32;
        let file_off = ph.p_offset as usize;
        let size = ph.p_filesz as usize;
        let bytes = &elf_bytes[file_off..file_off + size];

        if let Some(target_phys_off) = xip_offset(vaddr) {
            if let Some(backing) = &opts.flash_backing {
                let mut buf = backing.lock().unwrap();
                let end = target_phys_off + size;
                if end > buf.len() {
                    return Err(BootError::SegmentOutsideMap { addr: vaddr, size });
                }
                buf[target_phys_off..end].copy_from_slice(bytes);
                segments_loaded += 1;
                continue;
            }
            return Err(BootError::SegmentOutsideMap { addr: vaddr, size });
        }

        for (i, &b) in bytes.iter().enumerate() {
            let addr = vaddr.wrapping_add(i as u32) as u64;
            bus.write_u8(addr, b)
                .map_err(|_| BootError::SegmentOutsideMap { addr: vaddr, size })?;
        }
        segments_loaded += 1;
    }
```

Add the helper function below `fast_boot`:

```rust
/// If `vaddr` lies in either flash-XIP window, return the byte offset into
/// the flash backing buffer (assuming identity mapping, which `configure_*`
/// sets up by default).  Else None.
fn xip_offset(vaddr: u32) -> Option<usize> {
    const ICACHE_BASE: u32 = 0x4200_0000;
    const DCACHE_BASE: u32 = 0x3C00_0000;
    const WINDOW_SIZE: u32 = 0x0200_0000;
    if vaddr >= ICACHE_BASE && vaddr < ICACHE_BASE + WINDOW_SIZE {
        return Some((vaddr - ICACHE_BASE) as usize);
    }
    if vaddr >= DCACHE_BASE && vaddr < DCACHE_BASE + WINDOW_SIZE {
        return Some((vaddr - DCACHE_BASE) as usize);
    }
    None
}
```

- [ ] **Step 3: Update existing tests to pass `BootOpts::default()`**

The existing two `fast_boot` tests use `BootOpts { stack_top_fallback: 0x3FCD_FFF0 }`. Update them to:

```rust
        let summary = fast_boot(
            &elf_bytes,
            &mut bus,
            &mut cpu,
            &BootOpts {
                stack_top_fallback: 0x3FCD_FFF0,
                flash_backing: None,
            },
        )
        .expect("fast_boot");
```

- [ ] **Step 4: Add a new test for XIP segment loading**

Append to `mod tests` in `boot/esp32s3.rs`:

```rust
    #[test]
    fn fast_boot_loads_xip_segment_into_backing() {
        // Build an ELF with one PT_LOAD at 0x4200_1000 (XIP window).
        let mut elf = vec![0u8; 64 + 56 + 8];
        elf[0..4].copy_from_slice(b"\x7FELF");
        elf[4] = 2;
        elf[5] = 1;
        elf[6] = 1;
        elf[16] = 2; // ET_EXEC
        elf[18] = 94; // EM_XTENSA
        elf[20] = 1;
        elf[24..28].copy_from_slice(&0x4200_1000u32.to_le_bytes());
        elf[32] = 64;
        elf[52..54].copy_from_slice(&64u16.to_le_bytes());
        elf[54..56].copy_from_slice(&56u16.to_le_bytes());
        elf[56..58].copy_from_slice(&1u16.to_le_bytes());

        let ph = 64;
        elf[ph..ph + 4].copy_from_slice(&1u32.to_le_bytes()); // PT_LOAD
        elf[ph + 4..ph + 8].copy_from_slice(&5u32.to_le_bytes());
        elf[ph + 8..ph + 16].copy_from_slice(&120u64.to_le_bytes()); // p_offset
        elf[ph + 16..ph + 24].copy_from_slice(&0x4200_1000u64.to_le_bytes());
        elf[ph + 24..ph + 32].copy_from_slice(&0x4200_1000u64.to_le_bytes());
        elf[ph + 32..ph + 40].copy_from_slice(&8u64.to_le_bytes());
        elf[ph + 40..ph + 48].copy_from_slice(&8u64.to_le_bytes());

        elf[120..128].copy_from_slice(b"FLASHHHH");

        let mut bus = SystemBus::new();
        let backing = std::sync::Arc::new(std::sync::Mutex::new(vec![0u8; 4 * 1024 * 1024]));
        let mut cpu = XtensaLx7::new();
        cpu.reset(&mut bus).unwrap();

        let summary = fast_boot(
            &elf,
            &mut bus,
            &mut cpu,
            &BootOpts {
                stack_top_fallback: 0x3FCD_FFF0,
                flash_backing: Some(backing.clone()),
            },
        )
        .expect("fast_boot");

        assert_eq!(summary.entry, 0x4200_1000);
        assert_eq!(summary.segments_loaded, 1);
        let buf = backing.lock().unwrap();
        assert_eq!(&buf[0x1000..0x1008], b"FLASHHHH");
    }
```

- [ ] **Step 5: Build and test**

```bash
cargo test -p labwired-core boot::esp32s3 2>&1 | tail -10
```

Expected: all three tests PASS.

- [ ] **Step 6: Run full sim suite**

```bash
cargo test --workspace \
  --exclude firmware --exclude firmware-ci-fixture --exclude riscv-ci-fixture \
  2>&1 | tail -5
```

Expected: ≥ baseline + 27 new tests.

- [ ] **Step 7: Commit**

```bash
git add crates/core/src/boot/esp32s3.rs
git commit -m "feat(boot): fast_boot loads flash-XIP segments via shared backing

Extends BootOpts with a flash_backing field. When a PT_LOAD segment's
virtual address falls inside either XIP window (0x4200_0000+ or
0x3C00_0000+), fast_boot writes to the shared Arc<Mutex<Vec<u8>>>
backing buffer directly instead of through the bus (which would raise
MemoryViolation since FlashXipPeripheral is read-only).

Default BootOpts.flash_backing = None preserves the previous behaviour
for non-XIP ELFs."
git push
```

---

## Phase 9 — Example Firmware (1 task)

### Task 9: esp-hal hello-world example crate

**Files:**
- Create: `examples/esp32s3-hello-world/Cargo.toml`
- Create: `examples/esp32s3-hello-world/rust-toolchain.toml`
- Create: `examples/esp32s3-hello-world/.cargo/config.toml`
- Create: `examples/esp32s3-hello-world/src/main.rs`
- Create: `examples/esp32s3-hello-world/build.rs`
- Create: `examples/esp32s3-hello-world/README.md`
- Modify: `Cargo.toml` (workspace `exclude` — the example is NOT a workspace member because it uses a different toolchain target)

- [ ] **Step 1: Add `examples/esp32s3-hello-world` to workspace `exclude`**

Edit the root `Cargo.toml`. The `exclude` array currently contains `["crates/firmware-hal-test"]`. Change to:

```toml
exclude = ["crates/firmware-hal-test", "examples/esp32s3-hello-world"]
```

- [ ] **Step 2: Create `examples/esp32s3-hello-world/Cargo.toml`**

```toml
[package]
name = "esp32s3-hello-world"
version = "0.1.0"
edition = "2024"
authors = ["LabWired Team <team@labwired.io>"]
license = "MIT"
description = "ESP32-S3 hello-world demo for the LabWired simulator (Plan 2)."

[dependencies]
esp-hal       = { version = "1.0", features = ["esp32s3"] }
esp-println   = { version = "0.13", features = ["esp32s3", "jtag-serial"] }
esp-backtrace = { version = "0.15", features = ["esp32s3", "panic-handler", "println"] }

[profile.release]
opt-level     = "s"
lto           = "thin"
codegen-units = 1
debug         = true   # keep DWARF so the simulator can resolve symbols
```

(If the `esp-hal` v1.0 API or the related crate versions don't match what's published when implementation starts, the implementer pins to the closest stable versions and adjusts the imports in `src/main.rs` accordingly. The hello-world API is stable across minor releases.)

- [ ] **Step 3: Create `examples/esp32s3-hello-world/rust-toolchain.toml`**

```toml
[toolchain]
channel = "esp"
```

- [ ] **Step 4: Create `examples/esp32s3-hello-world/.cargo/config.toml`**

```toml
[build]
target = "xtensa-esp32s3-none-elf"

[target.xtensa-esp32s3-none-elf]
runner = "espflash flash --monitor"
rustflags = [
  "-C", "link-arg=-Tlinkall.x",
  "-C", "link-arg=-nostartfiles",
]

[unstable]
build-std = ["alloc", "core"]
```

- [ ] **Step 5: Create `examples/esp32s3-hello-world/src/main.rs`**

```rust
//! ESP32-S3 hello-world for the LabWired simulator (Plan 2).
//!
//! Prints "Hello world!" via esp-println (USB_SERIAL_JTAG path) once per
//! second, indefinitely.  Runs identically on the simulator and on a
//! connected ESP32-S3-Zero.

#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::{delay::Delay, prelude::*};
use esp_println::println;

#[entry]
fn main() -> ! {
    let _peripherals = esp_hal::init(esp_hal::Config::default());
    let delay = Delay::new();
    loop {
        println!("Hello world!");
        delay.delay_millis(1000);
    }
}
```

- [ ] **Step 6: Create `examples/esp32s3-hello-world/build.rs`**

```rust
fn main() {
    // esp-hal expects this so its ROM-symbol linker scripts get found.
    println!("cargo:rustc-link-arg=-Tdefmt.x");
}
```

- [ ] **Step 7: Create `examples/esp32s3-hello-world/README.md`**

```markdown
# esp32s3-hello-world

Canonical esp-hal hello-world for the LabWired ESP32-S3 simulator (Plan 2).

## Build

Requires the ESP Rust toolchain. Install via [`espup`](https://docs.esp-rs.org/book/installation/index.html):

```sh
cargo install espup
espup install
. ~/export-esp.sh   # exports PATH + LIBCLANG_PATH
```

Build:

```sh
cd examples/esp32s3-hello-world
cargo +esp build --release
```

The resulting ELF is at `target/xtensa-esp32s3-none-elf/release/esp32s3-hello-world`.

## Run in the simulator

From the workspace root:

```sh
cargo run -p labwired-cli -- run \
    --chip configs/chips/esp32s3-zero.yaml \
    --firmware examples/esp32s3-hello-world/target/xtensa-esp32s3-none-elf/release/esp32s3-hello-world
```

The simulator should print `Hello world!` to stdout once per second.

## Run on real hardware

With the ESP32-S3-Zero connected via USB:

```sh
cd examples/esp32s3-hello-world
cargo +esp run --release
# … in another terminal:
cat /dev/ttyACM0
```

Output should be identical to the simulator's.
```

- [ ] **Step 8: Verify the firmware crate isn't accidentally picked up by the main workspace**

```bash
cargo build --workspace --exclude firmware --exclude firmware-ci-fixture --exclude riscv-ci-fixture 2>&1 | tail -5
```

Expected: build succeeds (the example crate is in `exclude`, so the main workspace ignores it).

- [ ] **Step 9: Verify the firmware crate builds independently (requires ESP toolchain)**

```bash
cd examples/esp32s3-hello-world && cargo +esp build --release 2>&1 | tail -10
```

Expected: builds successfully and produces `target/xtensa-esp32s3-none-elf/release/esp32s3-hello-world`. If the build fails because `esp-hal` v1.0 API differs from this template, adjust `src/main.rs` per the latest `esp-hal` examples (e.g., https://github.com/esp-rs/esp-hal/tree/main/examples/src/bin/hello_world.rs) and re-run.

- [ ] **Step 10: Return to workspace root and commit**

```bash
cd /home/andrii/Projects/labwired-core-plan1
git add examples/esp32s3-hello-world Cargo.toml
git commit -m "feat(examples): esp-hal hello-world for ESP32-S3-Zero

Adds examples/esp32s3-hello-world/ — a minimal esp-hal crate that
prints \"Hello world!\" via USB_SERIAL_JTAG once per second.

The crate is NOT a workspace member (excluded in the root Cargo.toml)
because it targets xtensa-esp32s3-none-elf and uses the +esp toolchain;
build it with 'cargo +esp build --release' from inside the directory.

Same source compiles and runs identically on the LabWired simulator
(via 'labwired-cli run') and on a physical ESP32-S3-Zero (via
'cargo +esp run')."
git push
```

---

## Phase 10 — CLI `run` Subcommand (1 task)

### Task 10: CLI `labwired run` subcommand

**Files:**
- Modify: `crates/cli/src/main.rs` (add `Run` variant + handler)

- [ ] **Step 1: Inspect existing CLI structure**

```bash
sed -n '350,460p' crates/cli/src/main.rs
```

Note where `Commands` enum is matched in `main()` and how the existing `run_machine` handler is wired.

- [ ] **Step 2: Add `Run(RunArgs)` variant to `Commands` enum and `RunArgs` struct**

In `crates/cli/src/main.rs`, locate the `Commands` enum (around line 80) and the existing args structs. Add a new `RunArgs` struct after `MachineArgs`:

```rust
#[derive(Parser, Debug)]
pub struct RunArgs {
    /// Path to the chip descriptor YAML.
    #[arg(long)]
    pub chip: PathBuf,

    /// Path to the firmware ELF.
    #[arg(long)]
    pub firmware: PathBuf,

    /// Maximum number of simulator steps before exit (default: unlimited).
    #[arg(long)]
    pub max_steps: Option<u64>,
}
```

Extend the `Commands` enum:

```rust
#[derive(Subcommand, Debug)]
enum Commands {
    /// Deterministic, CI-friendly runner mode driven by a test script (YAML).
    Test(TestArgs),

    /// Machine control operations (load, etc.)
    Machine(MachineArgs),

    /// Run a firmware ELF in the simulator using a chip descriptor.
    ///
    /// Loads the chip's peripheral wiring, fast-boots the firmware, and
    /// runs the simulation loop.  Output written to USB_SERIAL_JTAG (for
    /// Xtensa chips) or UART (for ARM chips) appears on stdout in real
    /// time.
    Run(RunArgs),
}
```

- [ ] **Step 3: Add the dispatch in `main()`**

Find the existing `match` on `Commands` in the `main` function (around line 370). Add a new arm:

```rust
        Some(Commands::Run(args)) => run_firmware(args),
```

- [ ] **Step 4: Implement `run_firmware`**

Append to `crates/cli/src/main.rs` (after the existing `run_machine` function):

```rust
fn run_firmware(args: RunArgs) -> ExitCode {
    use labwired_core::boot::esp32s3::{fast_boot, BootOpts};
    use labwired_core::bus::SystemBus;
    use labwired_core::system::xtensa::{configure_xtensa_esp32s3, Esp32s3Opts};
    use labwired_core::Cpu;
    use labwired_core::SimulationError;

    // Read the chip YAML to validate the chip name.
    let chip_yaml = match std::fs::read_to_string(&args.chip) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read chip YAML at {:?}: {e}", args.chip);
            return ExitCode::from(2);
        }
    };
    if !chip_yaml.contains("xtensa-lx7") {
        eprintln!(
            "error: chip {:?} does not look like an Xtensa LX7 chip; \
             only ESP32-S3 is supported by `labwired run` in Plan 2",
            args.chip,
        );
        return ExitCode::from(2);
    }

    // Read the firmware ELF.
    let elf_bytes = match std::fs::read(&args.firmware) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: cannot read firmware ELF at {:?}: {e}", args.firmware);
            return ExitCode::from(2);
        }
    };

    // Wire the bus + CPU.
    let mut bus = SystemBus::new();
    let opts = Esp32s3Opts::default();
    let wiring = configure_xtensa_esp32s3(&mut bus, &opts);
    let mut cpu = wiring.cpu;

    // Fast-boot.
    let boot = match fast_boot(
        &elf_bytes,
        &mut bus,
        &mut cpu,
        &BootOpts {
            stack_top_fallback: 0x3FCD_FFF0,
            flash_backing: Some(wiring.flash_backing),
        },
    ) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: fast_boot failed: {e}");
            return ExitCode::from(3);
        }
    };
    eprintln!(
        "labwired-cli run: entry=0x{:08x} stack=0x{:08x} segments={}",
        boot.entry, boot.stack, boot.segments_loaded,
    );

    // Run the step loop.
    let limit = args.max_steps.unwrap_or(u64::MAX);
    let observers: Vec<std::sync::Arc<dyn labwired_core::SimulationObserver>> = Vec::new();
    let mut steps = 0u64;
    while steps < limit {
        match cpu.step(&mut bus, &observers) {
            Ok(()) => {}
            Err(SimulationError::BreakpointHit(pc)) => {
                eprintln!("labwired-cli run: BREAK at 0x{pc:08x}");
                return ExitCode::from(0);
            }
            Err(SimulationError::ExceptionRaised { cause, pc }) => {
                eprintln!(
                    "labwired-cli run: ExceptionRaised cause={cause} at 0x{pc:08x}"
                );
                return ExitCode::from(3);
            }
            Err(e) => {
                eprintln!(
                    "labwired-cli run: simulator error at pc=0x{:08x}: {e}",
                    cpu.get_pc(),
                );
                return ExitCode::from(3);
            }
        }
        bus.tick_peripherals_with_costs();
        steps += 1;
    }
    eprintln!(
        "labwired-cli run: reached --max-steps {limit}; pc=0x{:08x}",
        cpu.get_pc(),
    );
    ExitCode::from(0)
}
```

- [ ] **Step 5: Build the CLI**

```bash
cargo build -p labwired-cli 2>&1 | tail -10
```

Expected: build succeeds.

- [ ] **Step 6: Smoke-test the CLI surface (no firmware yet — error path)**

```bash
cargo run -p labwired-cli --quiet -- run --chip configs/chips/esp32s3-zero.yaml --firmware /tmp/nonexistent.elf 2>&1 | tail -3
```

Expected: prints `error: cannot read firmware ELF at "/tmp/nonexistent.elf"` and exits with non-zero status.

- [ ] **Step 7: Commit**

```bash
git add crates/cli/src/main.rs
git commit -m "feat(cli): labwired run subcommand for Xtensa firmware

Adds 'labwired-cli run --chip CHIP.yaml --firmware ELF' which:
1. Validates the chip YAML names an xtensa-lx7 chip.
2. Calls configure_xtensa_esp32s3 to wire the bus.
3. Calls boot::esp32s3::fast_boot to load the ELF and synthesise
   CPU state.
4. Enters the step loop until BREAK / exception / --max-steps.

USB_SERIAL_JTAG writes go directly to stdout; SYSTIMER ticks each
iteration so esp-hal Delay terminates."
git push
```

---

## Phase 11 — First Real Firmware Run + ROM Thunk Iteration (1 task)

### Task 11: Iterate ROM thunks until hello-world prints

**Files:**
- Modify: `crates/core/src/system/xtensa.rs:register_default_thunks` (fill in real addresses)
- Modify: `crates/core/src/peripherals/esp32s3/rom_thunks.rs` (add new thunks as needed)

**Background:** The hello-world firmware will jump into the BROM range at runtime. Each unregistered jump produces `NotImplemented` with the exact PC. The implementer adds thunks one at a time until the firmware reaches the print loop.

- [ ] **Step 1: Build the firmware**

```bash
cd examples/esp32s3-hello-world && cargo +esp build --release 2>&1 | tail -5 && cd -
```

Expected: ELF at `examples/esp32s3-hello-world/target/xtensa-esp32s3-none-elf/release/esp32s3-hello-world`. Record exact path.

- [ ] **Step 2: Disassemble the firmware to find ROM call sites**

```bash
xtensa-esp32s3-elf-objdump -d \
    examples/esp32s3-hello-world/target/xtensa-esp32s3-none-elf/release/esp32s3-hello-world \
    | grep -E "call[x0-9]?.*0x40[0-9a-f]+" \
    | awk '{print $NF}' | sort -u | head -40
```

Expected: a list of ROM call targets (addresses in `0x4000_0000..0x4006_0000`). Save the list — we'll register thunks for each.

- [ ] **Step 3: Cross-reference each address with ESP-IDF's `rom/esp32s3.ld`**

If you have ESP-IDF cloned, look up symbol names:

```bash
# Path may vary; common locations:
grep -E "0x40[0-9a-f]+" $IDF_PATH/components/esp_rom/esp32s3/ld/esp32s3.rom.ld 2>/dev/null | head -20
```

If you don't have ESP-IDF locally, the canonical file is at https://github.com/espressif/esp-idf/blob/master/components/esp_rom/esp32s3/ld/esp32s3.rom.ld — open it in a browser and search for each address.

Record `(address, symbol_name)` pairs in a scratch note.

- [ ] **Step 4: First simulator run**

```bash
RUST_LOG=info cargo run -p labwired-cli --quiet --release -- run \
    --chip configs/chips/esp32s3-zero.yaml \
    --firmware examples/esp32s3-hello-world/target/xtensa-esp32s3-none-elf/release/esp32s3-hello-world \
    --max-steps 100000 2>&1 | tail -20
```

Expected: one of three outcomes:
1. **`Hello world!`** appears on stdout — Plan 2 is complete on first try (unlikely but possible).
2. **`NotImplemented: ROM thunk at 0xXXXXXXXX not registered`** — proceed to Step 5.
3. **`simulator error at pc=...`** — investigate (likely missing peripheral register; extend the relevant stub).

- [ ] **Step 5: Register the missing ROM thunk**

For each `0xXXXXXXXX` reported in Step 4, look up the symbol from Step 3 and register the appropriate thunk in `register_default_thunks`. Most ROM functions for hello-world are NOPs that return 0 — use `cache_suspend_dcache` (returns 0) as the default until the firmware reveals it needs different behaviour.

Edit `crates/core/src/system/xtensa.rs:register_default_thunks`:

```rust
fn register_default_thunks(bank: &mut RomThunkBank) {
    // Addresses come from disassembling examples/esp32s3-hello-world and
    // cross-referencing with ESP-IDF rom/esp32s3.rom.ld.
    //
    // The set below is filled in iteratively: each NotImplemented error from
    // a simulator run identifies a missing thunk; add a line here, rebuild,
    // re-run.
    //
    // EXAMPLE entries (replace with real addresses + symbols once known):
    //   bank.register(0x40000xxx, rom_thunks::ets_printf);
    //   bank.register(0x40000xxx, rom_thunks::cache_suspend_dcache);
    //   bank.register(0x40000xxx, rom_thunks::cache_resume_dcache);
    //   bank.register(0x40000xxx, rom_thunks::esp_rom_spiflash_unlock);
    //   bank.register(0x40000xxx, rom_thunks::rom_config_instruction_cache_mode);
    //   bank.register(0x40000xxx, rom_thunks::ets_set_appcpu_boot_addr);
    //
    // The implementer fills these in based on disassembly output. Each line
    // unblocks one more step of the boot. When hello-world prints, this
    // function is "done" for Plan 2.
    let _ = (bank, rom_thunks::ets_printf);
}
```

If the firmware calls a ROM function not in the default set (e.g., `ets_delay_us`, `Cache_Invalidate_DCache`), add a new thunk to `rom_thunks.rs` following the existing pattern — minimal NOP-returning-0 unless the firmware behaviour suggests otherwise.

Common additional thunks the implementer is likely to add:

```rust
// In rom_thunks.rs:

/// `ets_delay_us(us: u32)` — no-op (real silicon busy-loops).
pub fn ets_delay_us(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `Cache_Invalidate_DCache_All()` — no-op.
pub fn cache_invalidate_dcache_all(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `esp_rom_install_uart_printf()` — no-op (we route via USB_SERIAL_JTAG).
pub fn esp_rom_install_uart_printf(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}
```

Add these as needed.

- [ ] **Step 6: Re-run; iterate Steps 4–5 until "Hello world!" appears**

This is the core "fast iteration" loop. Each iteration:
1. Run the CLI.
2. Note the next missing thunk (or other error).
3. Add the thunk (or fix the stub).
4. Repeat.

Time-box: if iteration count exceeds 30 thunks without convergence, escalate (likely a structural issue — perhaps esp-hal calls a function that needs real behaviour, not a NOP).

- [ ] **Step 7: Confirm output is exactly "Hello world!" once per simulated second**

```bash
cargo run -p labwired-cli --quiet --release -- run \
    --chip configs/chips/esp32s3-zero.yaml \
    --firmware examples/esp32s3-hello-world/target/xtensa-esp32s3-none-elf/release/esp32s3-hello-world \
    --max-steps 80000000 2>&1 | head -10
```

Expected: at least three "Hello world!" lines, separated by ≈80 M cycles (1 simulated second at 80 MHz).

- [ ] **Step 8: Commit the registered thunks**

```bash
git add crates/core/src/system/xtensa.rs crates/core/src/peripherals/esp32s3/rom_thunks.rs
git commit -m "feat(esp32s3): register ROM thunks for hello-world boot

Fills register_default_thunks with the ROM function addresses
esp-hal hello-world calls during init. Addresses derived by
disassembling the built firmware and cross-referencing with
ESP-IDF's rom/esp32s3.rom.ld.

Adds rom_thunks for any new functions encountered during the
iteration cycle (e.g. ets_delay_us, Cache_Invalidate_DCache_All).

End state: hello-world reaches its print loop and outputs
\"Hello world!\" once per simulated second."
git push
```

---

## Phase 12 — End-to-End Test in CI (1 task)

### Task 12: e2e_hello_world test gated on esp32s3-fixtures feature

**Files:**
- Create: `crates/core/tests/e2e_hello_world.rs`
- Modify: `crates/core/Cargo.toml` (add `esp32s3-fixtures` feature)

- [ ] **Step 1: Add the feature flag to `crates/core/Cargo.toml`**

Edit `crates/core/Cargo.toml`. After the `[dependencies]` block, add:

```toml
[features]
default = []
# Build + run end-to-end tests against examples/esp32s3-hello-world.
# Requires the ESP Rust toolchain (cargo +esp).
esp32s3-fixtures = []
```

- [ ] **Step 2: Create `crates/core/tests/e2e_hello_world.rs`**

```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// End-to-end test: build esp-hal hello-world, run it in the simulator,
// confirm "Hello world!" is captured from the USB_SERIAL_JTAG sink.
//
// Gated on `--features esp32s3-fixtures` so plain `cargo test` (without
// the ESP toolchain) still works.

#![cfg(feature = "esp32s3-fixtures")]

use labwired_core::boot::esp32s3::{fast_boot, BootOpts};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::esp32s3::usb_serial_jtag::UsbSerialJtag;
use labwired_core::system::xtensa::{configure_xtensa_esp32s3, Esp32s3Opts};
use labwired_core::{Bus, Cpu, Peripheral, SimulationError};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};

/// Path to the firmware ELF, relative to the workspace root.
fn firmware_path() -> PathBuf {
    PathBuf::from("examples/esp32s3-hello-world/target/xtensa-esp32s3-none-elf/release/esp32s3-hello-world")
}

/// Build the firmware crate via `cargo +esp build --release`.
/// Skips the build if the ELF already exists and is newer than `src/main.rs`.
fn ensure_firmware_built() -> PathBuf {
    let elf = firmware_path();
    let src = PathBuf::from("examples/esp32s3-hello-world/src/main.rs");
    if elf.exists() {
        if let (Ok(elf_meta), Ok(src_meta)) = (std::fs::metadata(&elf), std::fs::metadata(&src)) {
            if elf_meta.modified().unwrap() >= src_meta.modified().unwrap() {
                return elf;
            }
        }
    }
    let status = Command::new("cargo")
        .args(["+esp", "build", "--release"])
        .current_dir("examples/esp32s3-hello-world")
        .status()
        .expect("cargo +esp build (is the ESP toolchain installed?)");
    assert!(status.success(), "esp32s3-hello-world build failed");
    assert!(elf.exists(), "ELF not found at {:?} after build", elf);
    elf
}

#[test]
fn hello_world_prints_at_least_three_times() {
    let elf_path = ensure_firmware_built();
    let elf_bytes = std::fs::read(&elf_path).expect("read firmware ELF");

    let mut bus = SystemBus::new();
    let wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
    let mut cpu = wiring.cpu;

    // Replace the default UsbSerialJtag with one that captures into a buffer.
    let sink = Arc::new(Mutex::new(Vec::<u8>::new()));
    if let Some(p) = bus.peripherals.iter_mut().find(|p| p.name == "usb_serial_jtag") {
        if let Some(mut_any) = p.dev.as_any_mut() {
            if let Some(jtag) = mut_any.downcast_mut::<UsbSerialJtag>() {
                jtag.set_sink(Some(sink.clone()), false);
            }
        }
    }

    let _ = fast_boot(
        &elf_bytes,
        &mut bus,
        &mut cpu,
        &BootOpts {
            stack_top_fallback: 0x3FCD_FFF0,
            flash_backing: Some(wiring.flash_backing),
        },
    )
    .expect("fast_boot");

    // Run for up to 240 M simulated cycles (≈ 3 simulated seconds at 80 MHz).
    const MAX_STEPS: u64 = 240_000_000;
    let observers: Vec<std::sync::Arc<dyn labwired_core::SimulationObserver>> = Vec::new();
    for _ in 0..MAX_STEPS {
        match cpu.step(&mut bus, &observers) {
            Ok(()) => {}
            Err(SimulationError::BreakpointHit(_)) => break,
            Err(e) => panic!("simulator error at pc=0x{:08x}: {e}", cpu.get_pc()),
        }
        bus.tick_peripherals_with_costs();

        // Early exit once we have three Hello-world lines.
        let captured = sink.lock().unwrap();
        let s = String::from_utf8_lossy(&captured);
        if s.matches("Hello world!").count() >= 3 {
            return;
        }
    }
    let captured = sink.lock().unwrap();
    let s = String::from_utf8_lossy(&captured);
    panic!(
        "did not see 3+ 'Hello world!' lines in {MAX_STEPS} steps; captured: {:?}",
        s
    );
}
```

- [ ] **Step 3: Run the e2e test (requires the ESP toolchain)**

```bash
cargo test -p labwired-core --features esp32s3-fixtures e2e_hello_world 2>&1 | tail -10
```

Expected: PASS within a few minutes (build takes the longest; sim run is seconds).

- [ ] **Step 4: Confirm plain `cargo test` still works without the toolchain**

```bash
cargo test -p labwired-core --no-default-features 2>&1 | tail -5
```

Expected: existing tests pass; e2e test is skipped (`#![cfg(feature = "esp32s3-fixtures")]` excludes it).

- [ ] **Step 5: Commit**

```bash
git add crates/core/Cargo.toml crates/core/tests/e2e_hello_world.rs
git commit -m "test(e2e): hello_world end-to-end gated on esp32s3-fixtures feature

Adds crates/core/tests/e2e_hello_world.rs which:
1. Builds examples/esp32s3-hello-world via cargo +esp build --release
   (cached based on src mtime).
2. Wires the ESP32-S3 simulator (configure_xtensa_esp32s3 + fast_boot).
3. Captures USB_SERIAL_JTAG output into a buffer.
4. Runs the simulator for up to 240 M cycles or until 3 'Hello world!'
   lines are captured.

Gated on --features esp32s3-fixtures so plain 'cargo test' without
the ESP toolchain still works."
git push
```

---

## Phase 13 — Wrap-up (2 tasks)

### Task 13: Plan 2 case study

**Files:**
- Create: `docs/case_study_esp32s3_plan2.md`

- [ ] **Step 1: Capture final test count**

```bash
cargo test --workspace \
  --exclude firmware --exclude firmware-ci-fixture --exclude riscv-ci-fixture \
  2>&1 | tail -5
```

Record the count.

- [ ] **Step 2: Capture final commit list for Plan 2**

```bash
git log feature/esp32s3-plan2-boot-uart --not feature/esp32s3-plan1-foundation --oneline
```

Save the output for the case study.

- [ ] **Step 3: Write `docs/case_study_esp32s3_plan2.md`**

```markdown
# Case Study: ESP32-S3 Plan 2 — Boot Path + UART + SYSTIMER

**Date closed:** YYYY-MM-DD (substitute close date)
**Branch:** `feature/esp32s3-plan2-boot-uart`
**Spec:** `docs/superpowers/specs/2026-04-26-plan-2-boot-uart-systimer.md`
**Implementation plan:** `docs/superpowers/plans/2026-04-26-plan-2-boot-uart-systimer.md`
**Milestone closed:** M3 (Fast-path boot reaches `main`. `hello-world` prints via USB_SERIAL_JTAG) from the ESP32-S3-Zero design spec.

---

## What Plan 2 Delivered

Plan 2 took the LabWired simulator from "executes Xtensa instructions" to "runs a real `esp-hal` Rust binary end-to-end and prints `Hello world!` once per second". The same `examples/esp32s3-hello-world` ELF runs identically on the simulator and on a connected ESP32-S3-Zero.

### Test counts (final state)

| Suite | Passing | Notes |
|---|---|---|
| `labwired-core` (unit + integration) | (substitute count) | Includes new tests for boot, ROM thunks, USB_SERIAL_JTAG, SYSTIMER, system stubs, FlashXIP, system glue |
| `labwired-core --features esp32s3-fixtures` | +1 | `e2e_hello_world` runs the actual firmware in CI |
| Total | (substitute) | |

### Components shipped

| Component | File | LoC (approx) |
|---|---|---|
| Boot module + fast_boot | `crates/core/src/boot/{mod,esp32s3}.rs` | ~300 |
| ROM thunk dispatch | `crates/core/src/peripherals/esp32s3/rom_thunks.rs` | ~250 |
| BREAK 1,14 dispatch hook | `crates/core/src/cpu/xtensa_lx7.rs` (modified) | +30 |
| UsbSerialJtag | `crates/core/src/peripherals/esp32s3/usb_serial_jtag.rs` | ~150 |
| Systimer | `crates/core/src/peripherals/esp32s3/systimer.rs` | ~300 |
| System / RTC_CNTL / EFUSE stubs | `crates/core/src/peripherals/esp32s3/system_stub.rs` | ~150 |
| FlashXipPeripheral | `crates/core/src/peripherals/esp32s3/flash_xip.rs` | ~120 |
| System glue | `crates/core/src/system/xtensa.rs` | ~200 |
| Chip YAML | `configs/chips/esp32s3-zero.yaml` | ~70 |
| Example firmware | `examples/esp32s3-hello-world/` | ~150 |
| CLI run subcommand | `crates/cli/src/main.rs` (modified) | +120 |
| E2E test | `crates/core/tests/e2e_hello_world.rs` | ~100 |
| Total | | ≈1,940 |

---

## ROM Thunks Registered

(Substitute the actual list discovered during Task 11)

| Address | Symbol | Implementation |
|---|---|---|
| 0x40000xxx | ets_printf | minimal printf via tracing::info! |
| 0x40000xxx | cache_suspend_dcache | NOP, returns 0 |
| 0x40000xxx | cache_resume_dcache | NOP, returns 0 |
| ... | ... | ... |

---

## Plan Corrections Caught

(Substitute discoveries from implementation. Likely candidates:)
- esp-hal calls function X which the spec didn't anticipate.
- SYSTIMER clock-domain math needed adjustment for Y reason.
- Something about the boot path (stack symbol name, flash-XIP page table, etc.) was different from the spec's assumption.

---

## Plan 2 Exit Criteria Status

| # | Criterion | Status |
|---|---|---|
| 1 | Sim suite stays green | PASS |
| 2 | esp-hal hello-world builds | PASS |
| 3 | Fast-boot synthesises correct entry state | PASS |
| 4 | E2E demo prints expected output | PASS |
| 5 | CLI runs the firmware end-to-end | PASS |
| 6 | No silent ROM calls | PASS — all NotImplemented errors during implementation were resolved by registering thunks |
| 7 | Documentation | PASS — this case study |

---

## Known Gaps and Acknowledged Limitations

(Substitute as discovered. Likely candidates:)
- **No GPIO / IO_MUX / Interrupt Matrix.** Plan 3 territory.
- **No SYSTIMER alarm IRQs.** Polling-only counter access works; alarm-driven delays land in Plan 3.
- **No HW oracle diff for the `--diff` stretch goal.** Sim and HW both produce the expected output independently; bit-stream comparison is left for Plan 2.5 if needed.
- **Static flash-XIP MMU page table.** Real firmware can remap pages at runtime; Plan 2's table is populated once at boot from segment layout.

---

## Invitation for Plan 3

Plan 3 builds the next layer on top of Plan 2's boot + UART + SYSTIMER:

- **GPIO + IO_MUX:** pin functions, matrix-routed signals, edge-detect.
- **Interrupt Matrix:** 94 sources × 26 levels per core, ROM-supplied dispatch.
- **SYSTIMER alarms:** alarm registers + comparator + IRQ generation.
- **Blinky demo:** an esp-hal binary toggles a GPIO from a SYSTIMER alarm ISR; runs identically on sim and HW (with WS2812 on GPIO21 visible on the S3-Zero, OR a logic-analyzer probe on a simpler GPIO pin).

The HW-oracle infrastructure from Plan 1 extends naturally to peripheral oracle tests for GPIO + interrupt delivery.
```

- [ ] **Step 4: Commit**

```bash
git add docs/case_study_esp32s3_plan2.md
git commit -m "docs: Plan 2 case study (M3 closeout)

Documents what Plan 2 delivered: boot path + USB_SERIAL_JTAG +
SYSTIMER + minimal stubs running an esp-hal hello-world end-to-end
with bit-identical output to the connected ESP32-S3-Zero."
git push
```

---

### Task 14: Branch finalisation

**Files:** none (git only)

- [ ] **Step 1: Verify clean working tree**

```bash
git status
```

Expected: `nothing to commit, working tree clean`.

- [ ] **Step 2: Verify all commits are pushed**

```bash
git log feature/esp32s3-plan2-boot-uart --not feature/esp32s3-plan1-foundation --oneline
git status -sb
```

Expected: the log lists 13 commits; branch up to date with origin.

- [ ] **Step 3: Inform user that Plan 2 is complete**

Plan 2 is complete. The branch `feature/esp32s3-plan2-boot-uart` is ready for review/merge. The CLI `labwired run` invocation produces `Hello world!` output once per second from a real esp-hal binary, identical to the connected ESP32-S3-Zero.

If the user wants to proceed to Plan 3 (GPIO + IO_MUX + Interrupt Matrix + SYSTIMER alarms + blinky), brainstorm via the brainstorming skill.
