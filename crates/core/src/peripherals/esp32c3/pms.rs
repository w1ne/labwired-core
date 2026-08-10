// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C3 PMS — the permission-control (memory-protection) unit.
//!
//! # Why this exists
//!
//! ESP-IDF turns memory protection ON by default on the C3
//! (`CONFIG_ESP_SYSTEM_MEMPROT_FEATURE`), so every stock Arduino/IDF image runs
//! with IRAM text write-protected and the IRAM view of the data region
//! non-executable. Until this model existed, the twin permitted both: a stray
//! store into `.iram.text`, or a jump into a data buffer through a corrupted
//! function pointer, ran clean in the browser and only faulted on a bench
//! (`Guru Meditation Error: Core 0 panic'ed (Memory protection fault)`).
//!
//! # What the silicon does (and what this models)
//!
//! The PMS is NOT a synchronous trap. It watches the IRAM0 and DRAM0 bus
//! ports, blocks the offending access, latches *what* was violated into
//! `SENSITIVE` status registers, and raises an interrupt-matrix source. IDF
//! routes that source to CPU interrupt `ETS_MEMPROT_ERR_INUM` (26), whose
//! vector-table slot is `_panic_handler` — which is why the faulting PC has
//! skid and why the panic handler reads the violating address, world and
//! operation out of registers (`esp_memprot_get_violate_addr/world/operation`)
//! rather than trusting `mepc`.
//!
//! Everything below is derived from ESP-IDF v5.3.1 sources
//! (`components/soc/esp32c3/include/soc/memprot_defs.h`,
//! `components/hal/esp32c3/include/hal/memprot_ll.h`,
//! `components/esp_hw_support/port/esp32c3/esp_memprot.c`) and from the
//! silicon-corroborated register descriptor in
//! `configs/peripherals/esp32c3/sensitive.yaml`. No live re-capture backs the
//! *behaviour*; the register map and reset values were captured from a real
//! rev-v0.4 part (see `examples/esp32c3/VALIDATION.md`).
//!
//! # Deliberately NOT modelled
//!
//! * **Loads.** Only stores and instruction fetches are checked. The read path
//!   on the bus is `&self`, and a read check is the single most likely way to
//!   fault firmware that silicon runs — the two cases that actually bite
//!   (write to IRAM text, execute from data) are both covered without it.
//! * **World 1.** Non-TEE firmware runs entirely in world 0; the world-1
//!   permission words (`..._CONSTRAIN_1`) are stored but never consulted, and
//!   a latched violation always reports `MEMP_HAL_WORLD_0` (0x1).
//! * **PIF / RTC-FAST / DMA-APBPERI PMS.** Peripheral-bus and RTC-FAST
//!   permissions and the per-DMA-master SRAM constraints are storage-only.
//! * **The cache data array** (`..._CACHEDATAARRAY_PMS_0`) and the ROM
//!   permission fields.
//!
//! # Modelled, and load-bearing: the write locks
//!
//! `CONFIG_ESP_SYSTEM_MEMPROT_FEATURE_LOCK=y` is the default, and the image
//! that faulted on silicon has it set. Once firmware writes a `..._CONSTRAIN_0`
//! / `..._MONITOR_0` lock bit, the registers it guards become read-only until
//! reset — protection genuinely CANNOT be turned off again. A model that let a
//! later write disable the monitor would quietly restore the old blindness, so
//! locked writes are rejected here rather than merely recorded.

/// Permission bits, matching `SENSITIVE_CORE_X_IRAM0_PMS_CONSTRAIN_SRAM_WORLD_X_*`
/// in `soc/memprot_defs.h`. DRAM0 uses only R and W (its 2-bit fields).
pub const PERM_R: u8 = 0x1;
pub const PERM_W: u8 = 0x2;
pub const PERM_X: u8 = 0x4;

/// `SENSITIVE` register offsets that carry PMS state (base 0x600C_1000).
pub mod reg {
    pub const SPLIT_LINE_CONSTRAIN_0_LOCK: u64 = 0x090;
    pub const SPLIT_LINE_CONSTRAIN_1_MAIN_ID: u64 = 0x094;
    pub const SPLIT_LINE_CONSTRAIN_2_IRAM_0: u64 = 0x098;
    pub const SPLIT_LINE_CONSTRAIN_3_IRAM_1: u64 = 0x09C;
    pub const SPLIT_LINE_CONSTRAIN_4_DRAM_0: u64 = 0x0A0;
    pub const SPLIT_LINE_CONSTRAIN_5_DRAM_1: u64 = 0x0A4;
    pub const IRAM0_PMS_CONSTRAIN_0_LOCK: u64 = 0x0A8;
    pub const IRAM0_PMS_CONSTRAIN_1_WORLD1: u64 = 0x0AC;
    pub const IRAM0_PMS_CONSTRAIN_2_WORLD0: u64 = 0x0B0;
    pub const IRAM0_PMS_MONITOR_0_LOCK: u64 = 0x0B4;
    pub const IRAM0_PMS_MONITOR_1: u64 = 0x0B8;
    pub const IRAM0_PMS_MONITOR_2: u64 = 0x0BC;
    pub const DRAM0_PMS_CONSTRAIN_0_LOCK: u64 = 0x0C0;
    pub const DRAM0_PMS_CONSTRAIN_1: u64 = 0x0C4;
    pub const DRAM0_PMS_MONITOR_0_LOCK: u64 = 0x0C8;
    pub const DRAM0_PMS_MONITOR_1: u64 = 0x0CC;
    pub const DRAM0_PMS_MONITOR_2: u64 = 0x0D0;
    pub const DRAM0_PMS_MONITOR_3: u64 = 0x0D4;

    /// Lowest and highest PMS-relevant offsets — a write outside this span
    /// cannot change the model, so the bus can skip the refresh.
    pub const PMS_SPAN: std::ops::RangeInclusive<u64> = 0x090..=0x0D4;
    /// First offset in [`PMS_SPAN`] — the base of the register shadow.
    pub const PMS_SPAN_BASE: u64 = 0x090;
    /// Number of 32-bit words in [`PMS_SPAN`].
    pub const PMS_SPAN_WORDS: usize = 18;
}

/// SRAM windows the IRAM0 / DRAM0 PMS areas cover, from `memprot_defs.h`
/// (`IRAM0_SRAM_LEVEL_1_LOW` .. `IRAM0_SRAM_LEVEL_3_HIGH`, likewise DRAM0).
pub const IRAM0_SRAM_LOW: u32 = 0x4038_0000;
pub const IRAM0_SRAM_END: u32 = 0x403E_0000; // exclusive
pub const DRAM0_SRAM_LOW: u32 = 0x3FC8_0000;
pub const DRAM0_SRAM_END: u32 = 0x3FCE_0000; // exclusive

/// 128 KiB `I_D_SRAM_SEGMENT_SIZE`.
const SRAM_SEGMENT: u32 = 0x2_0000;

/// Address bases the latched status fields are stored relative to
/// (`IRAM0_VIOLATE_STATUS_ADDR_OFFSET` / `DRAM0_VIOLATE_STATUS_ADDR_OFFSET`).
const IRAM0_STATUS_ADDR_BASE: u32 = 0x4000_0000;
const DRAM0_STATUS_ADDR_BASE: u32 = 0x3C00_0000;

/// `MEMP_HAL_WORLD_0` (`hal/memprot_types.h`). Reported for every violation —
/// see "Deliberately NOT modelled" above.
const WORLD_0: u32 = 0x1;

/// Which bus port saw the violation. Each has its own monitor, its own status
/// registers and its own interrupt-matrix source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmsPort {
    /// Instruction bus, `ETS_CORE0_IRAM0_PMS_INTR_SOURCE`.
    Iram0,
    /// Data bus, `ETS_CORE0_DRAM0_PMS_INTR_SOURCE`.
    Dram0,
}

impl PmsPort {
    /// Interrupt-matrix source ID, from the `periph_interrput_t` enum in
    /// `soc/esp32c3/include/soc/interrupts.h` (ASSIST_DEBUG = 54, then
    /// DMA_APBPERI_PMS = 55, CORE0_IRAM0_PMS = 56, CORE0_DRAM0_PMS = 57).
    pub const fn intr_source(self) -> u32 {
        match self {
            PmsPort::Iram0 => 56,
            PmsPort::Dram0 => 57,
        }
    }
}

/// The operation a violating access was performing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmsOp {
    Fetch,
    Store,
}

/// A latched violation, shaped exactly like what firmware reads back through
/// `esp_mprot_get_violate_addr/world/operation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmsViolation {
    pub port: PmsPort,
    pub addr: u32,
    pub op: PmsOp,
    /// `MEMP_HAL_WORLD_0` == 1.
    pub world: u32,
}

/// One bus port's PMS state: four areas delimited by three split lines, plus
/// the monitor enable and the latched violation.
#[derive(Debug, Clone, Default)]
struct PortState {
    /// Ascending area boundaries; `bounds[i]` is the exclusive top of area `i`.
    /// Area 3 runs to the end of the SRAM window.
    bounds: [u32; 3],
    /// Per-area permission bits (world 0).
    perm: [u8; 4],
    monitor_en: bool,
    /// `true` once at least one area denies something — until then the port
    /// cannot possibly fault and the bus skips the check entirely.
    restricted: bool,
    latched: Option<PmsViolation>,
}

/// The C3 permission-control unit, derived from the `SENSITIVE` register file.
///
/// This is a *derived cache*, never a second source of truth: every field is
/// recomputed from the `SENSITIVE` peripheral's own register storage by
/// [`Esp32C3Pms::refresh`] whenever firmware writes into the PMS register span.
#[derive(Debug, Clone)]
pub struct Esp32C3Pms {
    iram0: PortState,
    dram0: PortState,
    /// Last accepted value of every word in [`reg::PMS_SPAN`], so a write to a
    /// register the firmware has already locked can be undone — silicon simply
    /// ignores it.
    shadow: [u32; reg::PMS_SPAN_WORDS],
    /// Total violations detected since reset (diagnostics / tests).
    pub violations: u64,
}

impl Default for Esp32C3Pms {
    fn default() -> Self {
        Self {
            iram0: PortState::default(),
            dram0: PortState::default(),
            shadow: [0; reg::PMS_SPAN_WORDS],
            violations: 0,
        }
    }
}

/// What a write into the PMS register span is allowed to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmsWriteVerdict {
    /// Ordinary configuration write — accept it and re-derive.
    Accept,
    /// The target register is locked (`..._CONSTRAIN_0` / `..._MONITOR_0` bit
    /// 0 is set), or is a hardware-owned status register. Silicon ignores the
    /// write; restore this value.
    Reject(u32),
}

/// Decode a split-line configuration register into an absolute address.
/// Mirrors `memprot_ll_get_split_addr_from_reg`: an 8-bit 512-byte-granular
/// offset plus a 3-way category selecting which 128 KiB SRAM segment it is
/// relative to. `None` when the register was never configured (all categories
/// zero) — exactly what the IDF helper returns as `NULL`.
pub fn decode_split_line(regval: u32, base: u32) -> Option<u32> {
    let off = ((regval >> 14) & 0xFF) << 9;
    let cat = [regval & 0x3, (regval >> 2) & 0x3, (regval >> 4) & 0x3];
    for (i, c) in cat.iter().enumerate() {
        if *c == 0x1 || *c == 0x2 {
            return Some(base + SRAM_SEGMENT * i as u32 + off);
        }
    }
    None
}

impl Esp32C3Pms {
    /// Rebuild the whole cached configuration from the `SENSITIVE` register
    /// file. `read` returns the raw 32-bit contents at a `SENSITIVE` offset.
    ///
    /// Latched violations are preserved: they are cleared only by the
    /// firmware's `VIOLATE_CLR` pulse (see [`Self::apply_monitor_ctrl`]).
    /// Decide what a firmware write to `offset` (already committed to the
    /// register storage as `written`) is permitted to do.
    ///
    /// Lock bits are the reason this exists. `CONFIG_ESP_SYSTEM_MEMPROT_FEATURE_LOCK`
    /// is on by default, so production firmware locks the split lines, the
    /// permissions and the monitors during startup; after that the protection
    /// cannot be relaxed. The lock registers are themselves set-only — writing
    /// 0 back does not unlock.
    pub fn write_verdict(&self, offset: u64, written: u32) -> PmsWriteVerdict {
        let prev = self.shadow_of(offset);
        // Hardware-owned status: the model is the only writer.
        if matches!(
            offset,
            reg::IRAM0_PMS_MONITOR_2 | reg::DRAM0_PMS_MONITOR_2 | reg::DRAM0_PMS_MONITOR_3
        ) {
            return PmsWriteVerdict::Reject(prev);
        }
        let locked = match offset {
            reg::SPLIT_LINE_CONSTRAIN_0_LOCK
            | reg::SPLIT_LINE_CONSTRAIN_1_MAIN_ID
            | reg::SPLIT_LINE_CONSTRAIN_2_IRAM_0
            | reg::SPLIT_LINE_CONSTRAIN_3_IRAM_1
            | reg::SPLIT_LINE_CONSTRAIN_4_DRAM_0
            | reg::SPLIT_LINE_CONSTRAIN_5_DRAM_1 => {
                self.shadow_of(reg::SPLIT_LINE_CONSTRAIN_0_LOCK) & 1 != 0
            }
            reg::IRAM0_PMS_CONSTRAIN_0_LOCK
            | reg::IRAM0_PMS_CONSTRAIN_1_WORLD1
            | reg::IRAM0_PMS_CONSTRAIN_2_WORLD0 => {
                self.shadow_of(reg::IRAM0_PMS_CONSTRAIN_0_LOCK) & 1 != 0
            }
            reg::IRAM0_PMS_MONITOR_0_LOCK | reg::IRAM0_PMS_MONITOR_1 => {
                self.shadow_of(reg::IRAM0_PMS_MONITOR_0_LOCK) & 1 != 0
            }
            reg::DRAM0_PMS_CONSTRAIN_0_LOCK | reg::DRAM0_PMS_CONSTRAIN_1 => {
                self.shadow_of(reg::DRAM0_PMS_CONSTRAIN_0_LOCK) & 1 != 0
            }
            reg::DRAM0_PMS_MONITOR_0_LOCK | reg::DRAM0_PMS_MONITOR_1 => {
                self.shadow_of(reg::DRAM0_PMS_MONITOR_0_LOCK) & 1 != 0
            }
            _ => false,
        };
        if locked && written != prev {
            PmsWriteVerdict::Reject(prev)
        } else {
            PmsWriteVerdict::Accept
        }
    }

    /// Record a hardware-side write (a latched status word) in the shadow, so
    /// a later firmware write to that register is rejected back to the value
    /// the hardware actually holds rather than a stale one.
    pub fn set_shadow(&mut self, offset: u64, value: u32) {
        if let Some(i) = offset
            .checked_sub(reg::PMS_SPAN_BASE)
            .map(|d| (d / 4) as usize)
        {
            if let Some(slot) = self.shadow.get_mut(i) {
                *slot = value;
            }
        }
    }

    /// Last accepted value of the PMS word at `offset` (0 outside the span).
    pub fn shadow_of(&self, offset: u64) -> u32 {
        offset
            .checked_sub(reg::PMS_SPAN_BASE)
            .map(|d| (d / 4) as usize)
            .and_then(|i| self.shadow.get(i).copied())
            .unwrap_or(0)
    }

    pub fn refresh(&mut self, read: impl Fn(u64) -> Option<u32>) {
        for (i, slot) in self.shadow.iter_mut().enumerate() {
            *slot = read(reg::PMS_SPAN_BASE + (i as u64) * 4).unwrap_or(0);
        }
        let rd = |off: u64| read(off).unwrap_or(0);

        // ── IRAM0 ────────────────────────────────────────────────────────────
        // Boundaries in the order the TRM lays them out (line 0, line 1, main
        // I/D). IDF's `esp_mprot_set_prot` writes `_iram_text_end` to all
        // three, so they coincide; clamping to a non-decreasing sequence keeps
        // a hand-rolled non-monotonic configuration from producing inverted
        // areas rather than silently mis-assigning permissions.
        let i0 = decode_split_line(rd(reg::SPLIT_LINE_CONSTRAIN_2_IRAM_0), IRAM0_SRAM_LOW);
        let i1 = decode_split_line(rd(reg::SPLIT_LINE_CONSTRAIN_3_IRAM_1), IRAM0_SRAM_LOW);
        let id = decode_split_line(rd(reg::SPLIT_LINE_CONSTRAIN_1_MAIN_ID), IRAM0_SRAM_LOW);
        self.iram0.bounds = monotonic([i0, i1, id], IRAM0_SRAM_LOW, IRAM0_SRAM_END);

        // Three permission bits per area, `..._SRAM_WORLD_0_PMS_{0..3}` at
        // bits [2:0] [5:3] [8:6] [11:9] of CONSTRAIN_2.
        let iperm = rd(reg::IRAM0_PMS_CONSTRAIN_2_WORLD0);
        for (a, p) in self.iram0.perm.iter_mut().enumerate() {
            *p = ((iperm >> (a * 3)) & 0x7) as u8;
        }
        self.iram0.restricted = self
            .iram0
            .perm
            .iter()
            .any(|p| *p & (PERM_R | PERM_W | PERM_X) != (PERM_R | PERM_W | PERM_X));
        self.iram0.monitor_en = rd(reg::IRAM0_PMS_MONITOR_1) & 0x2 != 0;

        // ── DRAM0 ────────────────────────────────────────────────────────────
        // Area 0 is the IRAM text region seen from the data bus, so its top is
        // the main I/D line mapped into the DRAM window; then DMA lines 0/1.
        let d_main = id.map(map_iram_to_dram);
        let d0 = decode_split_line(rd(reg::SPLIT_LINE_CONSTRAIN_4_DRAM_0), DRAM0_SRAM_LOW);
        let d1 = decode_split_line(rd(reg::SPLIT_LINE_CONSTRAIN_5_DRAM_1), DRAM0_SRAM_LOW);
        self.dram0.bounds = monotonic([d_main, d0, d1], DRAM0_SRAM_LOW, DRAM0_SRAM_END);

        // Two permission bits per area, `..._SRAM_WORLD_0_PMS_{0..3}` at bits
        // [1:0] [3:2] [5:4] [7:6] of CONSTRAIN_1.
        let dperm = rd(reg::DRAM0_PMS_CONSTRAIN_1);
        for (a, p) in self.dram0.perm.iter_mut().enumerate() {
            *p = ((dperm >> (a * 2)) & 0x3) as u8;
        }
        self.dram0.restricted = self
            .dram0
            .perm
            .iter()
            .any(|p| *p & (PERM_R | PERM_W) != (PERM_R | PERM_W));
        self.dram0.monitor_en = rd(reg::DRAM0_PMS_MONITOR_1) & 0x2 != 0;
    }

    /// Apply a write to a `..._PMS_MONITOR_1` register: bit 0 is
    /// `VIOLATE_CLR`, which drops the latched status (and with it the
    /// interrupt-matrix source). Returns `true` if a latch was cleared.
    pub fn apply_monitor_ctrl(&mut self, port: PmsPort, value: u32) -> bool {
        let st = self.port_mut(port);
        st.monitor_en = value & 0x2 != 0;
        if value & 0x1 != 0 && st.latched.is_some() {
            st.latched = None;
            return true;
        }
        false
    }

    /// Could this bus possibly fault? False until firmware narrows some area,
    /// which is what keeps the model inert on the reset configuration (every
    /// `..._PMS_CONSTRAIN_*` reset value grants full permissions) and therefore
    /// byte-identical for firmware that never enables memory protection.
    #[inline]
    pub fn armed(&self) -> bool {
        (self.iram0.restricted && self.iram0.monitor_en)
            || (self.dram0.restricted && self.dram0.monitor_en)
    }

    /// Would `op` at `addr` be blocked? `None` means permitted (including
    /// every address outside the two PMS-covered SRAM windows).
    pub fn check(&self, addr: u32, op: PmsOp) -> Option<PmsViolation> {
        let (port, st) = if (IRAM0_SRAM_LOW..IRAM0_SRAM_END).contains(&addr) {
            (PmsPort::Iram0, &self.iram0)
        } else if (DRAM0_SRAM_LOW..DRAM0_SRAM_END).contains(&addr) {
            (PmsPort::Dram0, &self.dram0)
        } else {
            return None;
        };
        if !st.monitor_en || !st.restricted {
            return None;
        }
        let area = st.area_of(addr);
        let need = match op {
            PmsOp::Fetch => PERM_X,
            PmsOp::Store => PERM_W,
        };
        // The DRAM0 port has no fetch permission bit at all — the data bus
        // simply cannot serve instructions. On silicon a fetch from a DRAM0
        // address never reaches the PMS: it is an *instruction access fault*
        // (mcause 1), a different mechanism. Reporting it as a DRAM0 violation
        // is therefore an APPROXIMATION, chosen because the alternative is to
        // execute from the data bus silently — the exact blindness this model
        // exists to remove. It is gated on the DRAM0 monitor being enabled and
        // some area narrowed, so it can only fire on firmware that has itself
        // turned memory protection on. The faithful case (a corrupted pointer
        // into the IRAM VIEW of the data region, area 3) is handled above and
        // really is a PMS violation.
        if op == PmsOp::Fetch && port == PmsPort::Dram0 {
            return Some(PmsViolation {
                port,
                addr,
                op,
                world: WORLD_0,
            });
        }
        if st.perm[area] & need != 0 {
            return None;
        }
        Some(PmsViolation {
            port,
            addr,
            op,
            world: WORLD_0,
        })
    }

    /// Latch `v` into the port's status. Silicon keeps the FIRST violation
    /// until `VIOLATE_CLR`, so a second one while a latch is live is counted
    /// but does not overwrite. Returns `true` when this call latched.
    pub fn latch(&mut self, v: PmsViolation) -> bool {
        self.violations = self.violations.saturating_add(1);
        let st = self.port_mut(v.port);
        if st.latched.is_some() {
            return false;
        }
        st.latched = Some(v);
        true
    }

    /// The currently latched violation on `port`, if any.
    pub fn latched(&self, port: PmsPort) -> Option<PmsViolation> {
        match port {
            PmsPort::Iram0 => self.iram0.latched,
            PmsPort::Dram0 => self.dram0.latched,
        }
    }

    /// The `SENSITIVE` status words for `port`, as `(offset, value)` pairs to
    /// write back into the descriptor's register storage so firmware — and the
    /// inspect wall — read exactly what real IDF reads.
    pub fn status_words(&self, port: PmsPort) -> Vec<(u64, u32)> {
        match port {
            PmsPort::Iram0 => {
                let Some(v) = self.iram0.latched else {
                    return vec![(reg::IRAM0_PMS_MONITOR_2, 0)];
                };
                // CORE_0_IRAM0_PMS_MONITOR_2: INTR[0], WR[1], LOADSTORE[2],
                // WORLD[4:3], ADDR[28:5] (address stored >> 2, relative to
                // 0x4000_0000).
                let mut w = 0x1u32;
                if v.op == PmsOp::Store {
                    w |= 1 << 1;
                    w |= 1 << 2; // LOADSTORE: this was a load/store, not a fetch
                }
                w |= (v.world & 0x3) << 3;
                w |= ((v.addr.wrapping_sub(IRAM0_STATUS_ADDR_BASE) >> 2) & 0x00FF_FFFF) << 5;
                vec![(reg::IRAM0_PMS_MONITOR_2, w)]
            }
            PmsPort::Dram0 => {
                let Some(v) = self.dram0.latched else {
                    return vec![(reg::DRAM0_PMS_MONITOR_2, 0), (reg::DRAM0_PMS_MONITOR_3, 0)];
                };
                // CORE_0_DRAM0_PMS_MONITOR_2: INTR[0], LOCK[1], WORLD[3:2],
                // ADDR[27:4] (>> 2, relative to 0x3C00_0000).
                let mut w2 = 0x1u32;
                w2 |= (v.world & 0x3) << 2;
                w2 |= ((v.addr.wrapping_sub(DRAM0_STATUS_ADDR_BASE) >> 2) & 0x00FF_FFFF) << 4;
                // CORE_0_DRAM0_PMS_MONITOR_3: WR[0], BYTEEN[4:1].
                let w3 = if v.op == PmsOp::Store { 0x1 } else { 0x0 };
                vec![
                    (reg::DRAM0_PMS_MONITOR_2, w2),
                    (reg::DRAM0_PMS_MONITOR_3, w3),
                ]
            }
        }
    }

    /// Interrupt-matrix source IDs currently asserted by latched violations.
    /// Level semantics: a source stays asserted until `VIOLATE_CLR`.
    pub fn asserted_sources(&self) -> [u64; 2] {
        let mut out = [0u64; 2];
        for port in [PmsPort::Iram0, PmsPort::Dram0] {
            let live = match port {
                PmsPort::Iram0 => self.iram0.latched.is_some() && self.iram0.monitor_en,
                PmsPort::Dram0 => self.dram0.latched.is_some() && self.dram0.monitor_en,
            };
            if live {
                let src = port.intr_source();
                out[(src / 64) as usize] |= 1u64 << (src % 64);
            }
        }
        out
    }

    fn port_mut(&mut self, port: PmsPort) -> &mut PortState {
        match port {
            PmsPort::Iram0 => &mut self.iram0,
            PmsPort::Dram0 => &mut self.dram0,
        }
    }
}

impl PortState {
    #[inline]
    fn area_of(&self, addr: u32) -> usize {
        for (i, &b) in self.bounds.iter().enumerate() {
            if addr < b {
                return i;
            }
        }
        3
    }
}

/// `MAP_IRAM_TO_DRAM` — the C3's SRAM is dual-mapped; the data view sits
/// `SOC_DIRAM_IRAM_LOW - SOC_DIRAM_DRAM_LOW` below the instruction view.
#[inline]
pub fn map_iram_to_dram(addr: u32) -> u32 {
    addr.wrapping_sub(IRAM0_SRAM_LOW)
        .wrapping_add(DRAM0_SRAM_LOW)
}

/// Clamp three optional split lines into a non-decreasing boundary triple
/// inside `[low, high]`. An unconfigured line (category bits zero, i.e. the
/// reset state) collapses to `low`, which leaves the areas below it empty —
/// the same thing silicon does with a split line that was never programmed.
fn monotonic(lines: [Option<u32>; 3], low: u32, high: u32) -> [u32; 3] {
    let mut out = [low; 3];
    let mut prev = low;
    for (i, l) in lines.iter().enumerate() {
        let v = l.unwrap_or(low).clamp(low, high);
        prev = v.max(prev);
        out[i] = prev;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The IDF default configuration, byte-for-byte as `esp_mprot_set_prot`
    /// writes it: every split line at `_iram_text_end`, IRAM areas 0..2 = R|X,
    /// area 3 = none, DRAM area 0 = none, areas 1..3 = R|W.
    fn idf_default(iram_text_end: u32) -> Esp32C3Pms {
        // Encode the split line the way `memprot_ll_set_iram0_split_line` does.
        let enc = |addr: u32, base: u32| -> u32 {
            let seg = (addr - base) / SRAM_SEGMENT;
            let mut cat = [0u32; 3];
            match seg {
                0 => {
                    cat[0] = 2;
                    cat[1] = 3;
                    cat[2] = 3;
                }
                1 => {
                    cat[1] = 2;
                    cat[2] = 3;
                }
                _ => cat[2] = 2,
            }
            let splitaddr = (addr >> 9) & 0xFF;
            (splitaddr << 14) | cat[0] | (cat[1] << 2) | (cat[2] << 4)
        };
        let dram_line = map_iram_to_dram(iram_text_end);
        let i_enc = enc(iram_text_end, IRAM0_SRAM_LOW);
        let d_enc = enc(dram_line, DRAM0_SRAM_LOW);
        // IRAM areas 0,1,2 = R|X (0b101 = 5), area 3 = 0.
        let iperm = 5 | (5 << 3) | (5 << 6);
        // DRAM area 0 = 0, areas 1,2,3 = R|W (0b11).
        let dperm = (3 << 2) | (3 << 4) | (3 << 6);
        let mut pms = Esp32C3Pms::default();
        pms.refresh(|off| {
            Some(match off {
                reg::SPLIT_LINE_CONSTRAIN_1_MAIN_ID => i_enc,
                reg::SPLIT_LINE_CONSTRAIN_2_IRAM_0 => i_enc,
                reg::SPLIT_LINE_CONSTRAIN_3_IRAM_1 => i_enc,
                reg::SPLIT_LINE_CONSTRAIN_4_DRAM_0 => d_enc,
                reg::SPLIT_LINE_CONSTRAIN_5_DRAM_1 => d_enc,
                reg::IRAM0_PMS_CONSTRAIN_2_WORLD0 => iperm,
                reg::DRAM0_PMS_CONSTRAIN_1 => dperm,
                reg::IRAM0_PMS_MONITOR_1 => 0x2,
                reg::DRAM0_PMS_MONITOR_1 => 0x2,
                _ => 0,
            })
        });
        pms
    }

    #[test]
    fn split_line_decode_matches_idf_helper() {
        // 0x4039_0000: segment 1 of the IRAM window, offset 0x1_0000.
        let regval = ((0x4039_0000u32 >> 9) & 0xFF) << 14 | 0x2 | (0x3 << 2) | (0x3 << 4);
        assert_eq!(decode_split_line(regval, IRAM0_SRAM_LOW), Some(0x4039_0000));
        // Never configured (all categories 0) -> NULL, as in IDF.
        assert_eq!(decode_split_line(0, IRAM0_SRAM_LOW), None);
    }

    #[test]
    fn reset_configuration_permits_everything() {
        // Every PMS CONSTRAIN register's reset value grants full permissions
        // (0x001C_7FFF / 0x0F0F_F0FF), which is why the model is inert until
        // firmware narrows it — the property that protects existing labs.
        let mut pms = Esp32C3Pms::default();
        pms.refresh(|off| {
            Some(match off {
                reg::IRAM0_PMS_CONSTRAIN_2_WORLD0 => 0x001C_7FFF,
                reg::DRAM0_PMS_CONSTRAIN_1 => 0x0F0F_F0FF,
                reg::IRAM0_PMS_MONITOR_1 | reg::DRAM0_PMS_MONITOR_1 => 0x3,
                _ => 0,
            })
        });
        assert!(!pms.armed(), "reset state must not arm the checker");
        assert_eq!(pms.check(0x4038_5000, PmsOp::Store), None);
        assert_eq!(pms.check(0x4038_5000, PmsOp::Fetch), None);
        assert_eq!(pms.check(0x3FCA_0000, PmsOp::Store), None);
    }

    #[test]
    fn idf_default_blocks_iram_text_write_and_data_execute() {
        let text_end = 0x4039_0000;
        let pms = idf_default(text_end);
        assert!(pms.armed());

        // A store into IRAM text (area 0..2, R|X): blocked, reported as a
        // write on the IRAM0 port.
        let v = pms
            .check(text_end - 0x400, PmsOp::Store)
            .expect("IRAM text must be write-protected");
        assert_eq!(v.port, PmsPort::Iram0);
        assert_eq!(v.op, PmsOp::Store);

        // Fetching that same text is fine.
        assert_eq!(pms.check(text_end - 0x400, PmsOp::Fetch), None);

        // Executing from the IRAM view of the data region (area 3, no perms):
        // blocked.
        let v = pms
            .check(text_end + 0x1000, PmsOp::Fetch)
            .expect("IRAM area 3 must be non-executable");
        assert_eq!(v.port, PmsPort::Iram0);
        assert_eq!(v.op, PmsOp::Fetch);

        // Ordinary data stores above the split line are permitted.
        assert_eq!(
            pms.check(map_iram_to_dram(text_end) + 0x1000, PmsOp::Store),
            None
        );
        // ...and stores into the DRAM view of IRAM text are not.
        assert!(pms
            .check(map_iram_to_dram(text_end) - 0x400, PmsOp::Store)
            .is_some());
    }

    #[test]
    fn status_words_round_trip_the_idf_getters() {
        let text_end = 0x4039_0000;
        let mut pms = idf_default(text_end);
        let addr = text_end - 0x400;
        let v = pms.check(addr, PmsOp::Store).unwrap();
        assert!(pms.latch(v));

        let words = pms.status_words(PmsPort::Iram0);
        let (_, w) = words[0];
        // memprot_ll_iram0_get_monitor_status_intr()
        assert_eq!(w & 0x1, 1);
        // memprot_ll_iram0_get_monitor_status_fault_wr()
        assert_eq!((w >> 1) & 0x1, 1);
        // ..._fault_loadstore() == 1 -> not a fetch, so the operation decodes
        // to MEMPROT_OP_WRITE in esp_mprot_get_violate_operation().
        assert_eq!((w >> 2) & 0x1, 1);
        // ..._fault_world() -> MEMP_HAL_WORLD_0
        assert_eq!((w >> 3) & 0x3, WORLD_0);
        // ..._fault_addr(): (field << 2) + 0x4000_0000
        let field = (w >> 5) & 0x00FF_FFFF;
        assert_eq!((field << 2) + IRAM0_STATUS_ADDR_BASE, addr);

        // The source is asserted while latched, and drops on VIOLATE_CLR.
        assert_eq!(pms.asserted_sources()[0] & (1 << 56), 1 << 56);
        assert!(pms.apply_monitor_ctrl(PmsPort::Iram0, 0x3));
        assert_eq!(pms.asserted_sources()[0], 0);
        assert_eq!(pms.status_words(PmsPort::Iram0)[0].1, 0);
    }

    #[test]
    fn fetch_status_decodes_as_exec_not_write() {
        let text_end = 0x4039_0000;
        let mut pms = idf_default(text_end);
        let v = pms.check(text_end + 0x1000, PmsOp::Fetch).unwrap();
        pms.latch(v);
        let (_, w) = pms.status_words(PmsPort::Iram0)[0];
        // LOADSTORE == 0 -> esp_mprot_get_violate_operation() reports
        // MEMPROT_OP_EXEC regardless of the WR bit.
        assert_eq!((w >> 2) & 0x1, 0);
    }

    #[test]
    fn first_violation_is_kept_until_cleared() {
        let text_end = 0x4039_0000;
        let mut pms = idf_default(text_end);
        let first = pms.check(text_end - 0x400, PmsOp::Store).unwrap();
        let second = pms.check(text_end - 0x800, PmsOp::Store).unwrap();
        assert!(pms.latch(first));
        assert!(!pms.latch(second), "silicon keeps the first violation");
        assert_eq!(pms.latched(PmsPort::Iram0).unwrap().addr, first.addr);
        assert_eq!(pms.violations, 2);
    }

    #[test]
    fn monitor_disabled_suppresses_enforcement() {
        let text_end = 0x4039_0000;
        let mut pms = idf_default(text_end);
        // `esp_mprot_set_prot` disables the monitor before reprogramming the
        // areas; nothing may fault in that window.
        pms.apply_monitor_ctrl(PmsPort::Iram0, 0x0);
        pms.apply_monitor_ctrl(PmsPort::Dram0, 0x0);
        assert!(!pms.armed());
        assert_eq!(pms.check(text_end - 0x400, PmsOp::Store), None);
    }
}
