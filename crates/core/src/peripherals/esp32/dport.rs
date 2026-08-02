// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! DPORT (data port / system controller) peripheral for ESP32-classic.
//!
//! Per ESP32 TRM v5.0 §6 (system control) and §7 (interrupt matrix). The
//! DPORT block sits at base `0x3FF0_0000`, spans 4 KiB, and owns the
//! cross-cutting plumbing the rest of the chip leans on at boot:
//!
//!   * Peripheral clock gating (`PERIP_CLK_EN`) and per-peripheral reset
//!     control (`PERIP_RST_EN`). ESP-IDF and Arduino-ESP32 hammer these
//!     during early init to ungate VSPI, GPIO, the timer groups, etc.
//!   * The dual-core handshake: PRO_CPU stalls APP_CPU through
//!     `APPCPU_CTRL_*` until `s_cpu_up` flips, and uses
//!     `CPU_PER_CONF` to set the per-core clock divider.
//!   * Cache-control plumbing (`PRO_CACHE_CTRL`, `APP_CACHE_CTRL`,
//!     `PRO_DCACHE_DBUG0..9`, `APP_DCACHE_DBUG0..9`,
//!     `PRO_CACHE_LOCK_0_ADDR..3`) the cache driver pokes before flash-XIP
//!     reads work. We don't model cache behavior — round-trip the writes
//!     so the configure-then-read-back sequences settle.
//!   * The AHB-Lite MPU table (`AHBLITE_MPU_TABLE_x`). Touched at boot;
//!     no enforcement modeled.
//!   * Per-core interrupt-matrix mapping registers
//!     (`PRO_*_INTR_MAP_REG` at `0x3FF0_0100..`, `APP_*_INTR_MAP_REG` at
//!     `0x3FF0_0200..`). These map peripheral source IDs to one of 32
//!     CPU IRQ slots — equivalent in spirit to the ESP32-S3 `intmatrix`
//!     peripheral, but at a different address and with a different
//!     layout. We round-trip every write so firmware probes see what
//!     they wrote; actual interrupt delivery is handled out-of-band
//!     by the WASM IPI bridge today (see
//!     `crates/wasm/src/lib.rs::step_with_esp32_aids`).
//!   * Cross-core software interrupt triggers
//!     (`CPU_INTR_FROM_CPU_0..3` at `0x3FF0_00DC..0x3FF0_00E8`) and
//!     their corresponding PRO/APP mapping registers
//!     (`PRO_INTR_FROM_CPU_0..3` at `0x3FF0_0164..0x3FF0_0170`,
//!     `APP_INTR_FROM_CPU_0..3` at `0x3FF0_0168..0x3FF0_0174`). The WASM
//!     IPI bridge polls these every cycle — it expects writes to be
//!     directly observable on the next read, which our plain HashMap
//!     storage satisfies.
//!
//! ## Cross-core IPIs
//!
//! `cross_core_pending(core)` resolves the `CPU_INTR_FROM_CPU_0..3`
//! trigger registers against the per-core interrupt-matrix MAP registers
//! and reports which CPU-interrupt slots that core should take. The bus
//! ORs this into `pending_cpu_irqs(core_id)` so a `FROM_CPU` write made by
//! one core's firmware is delivered as a real interrupt to the target
//! core inside `Machine::step` — no external bridge. The receiving ISR
//! de-asserts the source by writing the trigger register back to 0.
//!
//! ## What's intentionally NOT modeled
//!
//!   * No clock-gate enforcement. PERIP_CLK_EN is seeded all-ones at
//!     construction so any code that consults the bitmap before writing
//!     it (rare, but it happens in vendor headers gated behind
//!     `DPORT_REG_READ`) sees "everything's on" instead of "everything's
//!     off" — easier than tracking per-peripheral gating.
//!   * Full APPCPU_CTRL state machine beyond last-write-wins register
//!     storage. APP_CPU release is driven by the boot-ROM
//!     `ets_set_appcpu_boot_addr` surface + `Machine` unhalt of a real
//!     secondary LX6 (see `XtensaLx7::new_app_cpu`), not by forging
//!     firmware handshake bytes.
//!
//! ## Why this is a new peripheral instead of a stub
//!
//! Until this lands, the DPORT range was covered by `SystemStub` (the
//! ESP32-S3 catch-all). That works for register reads/writes that just
//! need to settle, but it (a) doesn't pre-seed PERIP_CLK_EN with a
//! "peripherals are live" bitmap and (b) bundles the analog AHB
//! region (0x3FF0_1000..0x3FF1_FFFF) into the same stub, making it
//! harder to inspect DPORT-specific state from tests and observers.
//! Splitting DPORT into its own peripheral gives Phase 2 a documented
//! surface to grow real semantics on (clock-gate tracking, IPI
//! delivery) without churning the rest of the catch-all.

use crate::{Peripheral, SimResult};

// ── Register offsets (per ESP32 TRM v5.0 §6 + §7) ───────────────────────────

/// DPORT_CPU_PER_CONF — bits[1:0] = CPU clock divider per core.
pub const DPORT_CPU_PER_CONF_OFFSET: u32 = 0x003C;
/// DPORT_PRO_CACHE_CTRL — PRO_CPU cache control word.
pub const DPORT_PRO_CACHE_CTRL_OFFSET: u32 = 0x0040;
/// DPORT_APP_CACHE_CTRL — APP_CPU cache control word.
pub const DPORT_APP_CACHE_CTRL_OFFSET: u32 = 0x0044;
/// DPORT_PRO_DCACHE_DBUG0 — PRO cache debug/status (`dport_reg.h`:
/// `DR_REG_DPORT_BASE + 0x3F0`). Bits [18:7] = `PRO_CACHE_STATE` (RO).
pub const DPORT_PRO_DCACHE_DBUG0_OFFSET: u32 = 0x03F0;
/// DPORT_APP_DCACHE_DBUG0 — APP cache debug/status (`+ 0x418`).
/// Bits [18:7] = `APP_CACHE_STATE` (RO).
pub const DPORT_APP_DCACHE_DBUG0_OFFSET: u32 = 0x0418;
/// `PRO/APP_CACHE_STATE` field: start bit and width (12 bits).
pub const DPORT_CACHE_STATE_S: u32 = 7;
pub const DPORT_CACHE_STATE_V: u32 = 0xFFF;
/// Idle / suspended state value that `cache_hal_suspend` polls for
/// (`extui ..., 7, 12` then `bnei ..., 1`).
pub const DPORT_CACHE_STATE_IDLE: u32 = 1;
/// DPORT_PERIP_CLK_EN — peripheral clock-gate bitmap.
pub const DPORT_PERIP_CLK_EN_OFFSET: u32 = 0x00C0;
/// DPORT_PERIP_RST_EN — peripheral reset bitmap.
pub const DPORT_PERIP_RST_EN_OFFSET: u32 = 0x00C4;
/// DPORT_AHBLITE_MPU_TABLE_x — AHB-Lite MPU table (0xC8..0xF8).
pub const DPORT_AHBLITE_MPU_TABLE_BASE: u32 = 0x00C8;
/// DPORT_PRO_CACHE_LOCK_0_ADDR..3 — PRO_CPU cache lock address words (0xD8..0xE4).
pub const DPORT_PRO_CACHE_LOCK_BASE: u32 = 0x00D8;
/// DPORT_CPU_INTR_FROM_CPU_0..3 — cross-core IPI triggers (0xDC..0xE8).
///
/// The WASM IPI bridge polls these every cycle expecting plain
/// last-write-wins semantics, so writes must round-trip verbatim.
pub const DPORT_CPU_INTR_FROM_CPU_0_OFFSET: u32 = 0x00DC;
pub const DPORT_CPU_INTR_FROM_CPU_1_OFFSET: u32 = 0x00E0;
pub const DPORT_CPU_INTR_FROM_CPU_2_OFFSET: u32 = 0x00E4;
pub const DPORT_CPU_INTR_FROM_CPU_3_OFFSET: u32 = 0x00E8;
/// DPORT_PRO_MAC_INTR_MAP_REG — first PRO_CPU intmatrix source entry
/// (`DR_REG_DPORT_BASE + 0x104`, per ESP-IDF `dport_reg.h`). Source `s`
/// maps at `base + s*4`, so e.g. the cross-core source 24 (`FROM_CPU_INTR0`)
/// binding lives at `0x104 + 24*4 = 0x0164`. Round-trips through `regs`
/// like every other word.
pub const DPORT_PRO_MAC_INTR_MAP_REG_OFFSET: u32 = 0x0104;
/// DPORT_APP_MAC_INTR_MAP_REG — first APP_CPU intmatrix source entry.
/// The APP-side matrix mirrors the PRO-side layout (source `s` at
/// `base + s*4`) but starts at 0x208, so e.g. the cross-core source 25
/// (`FROM_CPU_INTR1`) binding lives at `0x208 + 25*4 = 0x026C`.
pub const DPORT_APP_MAC_INTR_MAP_REG_OFFSET: u32 = 0x0208;
/// DPORT_PRO_INTR_FROM_CPU_0..3 — PRO_CPU bindings for the FROM_CPU
/// triggers above. The WASM bridge reads these to learn which CPU
/// INTERRUPT bit to raise when a trigger fires.
pub const DPORT_PRO_INTR_FROM_CPU_0_OFFSET: u32 = 0x0164;
pub const DPORT_PRO_INTR_FROM_CPU_1_OFFSET: u32 = 0x0168;
pub const DPORT_PRO_INTR_FROM_CPU_2_OFFSET: u32 = 0x016C;
pub const DPORT_PRO_INTR_FROM_CPU_3_OFFSET: u32 = 0x0170;
/// DPORT_APP_INTR_FROM_CPU_0..3 — APP_CPU bindings for FROM_CPU triggers.
pub const DPORT_APP_INTR_FROM_CPU_0_OFFSET: u32 = 0x0168;
pub const DPORT_APP_INTR_FROM_CPU_1_OFFSET: u32 = 0x016C;
pub const DPORT_APP_INTR_FROM_CPU_2_OFFSET: u32 = 0x0170;
pub const DPORT_APP_INTR_FROM_CPU_3_OFFSET: u32 = 0x0174;

/// DPORT peripheral.
///
/// Word-granular dense storage — DPORT is exactly 4 KiB / 1024 words, so a
/// flat `[u32; 1024]` (heap-boxed to keep the peripheral handle small) is
/// the same memory order as a real `HashMap` entry plus the SipHash work,
/// minus the hashing. The WASM IPI bridge polls this peripheral every
/// cycle; cache-resident array reads beat hashed lookups by a wide margin
/// on the bench.
#[derive(Debug)]
pub struct Dport {
    /// Base MMIO address (informational; bus dispatches by offset).
    base: u32,
    /// Backing word store. `regs[word_off >> 2]` is the value at byte
    /// offset `word_off`. Heap-boxed so the peripheral handle stays
    /// pointer-sized.
    regs: Box<[u32; Self::WORDS]>,
}

impl Default for Dport {
    fn default() -> Self {
        Self::new()
    }
}

impl Dport {
    /// Canonical MMIO base address on ESP32-classic.
    pub const BASE: u32 = 0x3FF0_0000;
    /// DPORT window size (4 KiB per TRM).
    pub const SIZE: u32 = 0x1000;
    /// Number of 32-bit words in the DPORT window.
    pub const WORDS: usize = (Self::SIZE / 4) as usize;

    /// Construct a freshly-powered DPORT block.
    ///
    /// Seeds:
    ///   * `PERIP_CLK_EN` = 0xFFFF_FFFF — treat all peripherals as
    ///     clock-enabled (we don't model gating).
    ///   * `PERIP_RST_EN` = 0 — no peripheral is held in reset.
    ///   * `CPU_PER_CONF` = 0 — undivided CPU clock; matches the
    ///     real silicon reset value.
    ///   * Every other offset reads back 0 until written.
    pub fn new() -> Self {
        let mut regs = Box::new([0u32; Self::WORDS]);
        regs[(DPORT_PERIP_CLK_EN_OFFSET >> 2) as usize] = 0xFFFF_FFFF;
        regs[(DPORT_PERIP_RST_EN_OFFSET >> 2) as usize] = 0;
        regs[(DPORT_CPU_PER_CONF_OFFSET >> 2) as usize] = 0;
        Self {
            base: Self::BASE,
            regs,
        }
    }

    /// Base MMIO address (informational).
    pub fn base(&self) -> u32 {
        self.base
    }

    fn read_word(&self, word_off: u32) -> u32 {
        // Bus already clamps offset to the registered SIZE, so the index
        // is bounded by WORDS. The cast is checked in debug via the array
        // bounds check; release builds elide it.
        let mut v = self.regs[(word_off >> 2) as usize];
        // PRO/APP_DCACHE_DBUG0: RO field CACHE_STATE[18:7]. ESP-IDF
        // `cache_hal_suspend` spin-waits until this field equals 1 (idle)
        // after requesting a suspend. We do not model cache tag state, so
        // report idle immediately — same class of handshake as
        // CACHE_ENABLE→CACHE_ENABLED on CACHE_CTRL (write_word below).
        // Without this, dual-core Arduino boot hangs forever in
        // cache_hal_suspend while APP_CPU sits in call_start_cpu1 waiting
        // on s_resume_cores (PRO never reaches startup_resume_other_cores).
        if word_off == DPORT_PRO_DCACHE_DBUG0_OFFSET || word_off == DPORT_APP_DCACHE_DBUG0_OFFSET {
            let mask = DPORT_CACHE_STATE_V << DPORT_CACHE_STATE_S;
            v = (v & !mask) | (DPORT_CACHE_STATE_IDLE << DPORT_CACHE_STATE_S);
        }
        v
    }

    fn write_word(&mut self, word_off: u32, value: u32) {
        let mut stored = value;
        // Cache enable/enabled handshake. The BROM `Cache_Read_Init` sets
        // PRO/APP_CACHE_CTRL bit 4 (CACHE_ENABLE) then spin-waits for bit 5
        // (CACHE_ENABLED) to assert — real silicon turns the cache on in a
        // few cycles and latches the status bit. We don't model cache
        // behavior, so mirror the enabled bit to the enable bit immediately,
        // otherwise the BROM's `bnone CACHE_CTRL, 0x20` poll never completes.
        if word_off == DPORT_PRO_CACHE_CTRL_OFFSET || word_off == DPORT_APP_CACHE_CTRL_OFFSET {
            const CACHE_ENABLE_BIT: u32 = 1 << 4;
            const CACHE_ENABLED_BIT: u32 = 1 << 5;
            if stored & CACHE_ENABLE_BIT != 0 {
                stored |= CACHE_ENABLED_BIT;
            } else {
                stored &= !CACHE_ENABLED_BIT;
            }
        }
        self.regs[(word_off >> 2) as usize] = stored;
    }

    /// Cross-core IPI delivery: which CPU-interrupt slots `core` should see
    /// asserted right now, driven by the `CPU_INTR_FROM_CPU_0..3` trigger
    /// registers.
    ///
    /// On real silicon each `CPU_INTR_FROM_CPU_n` register is a single
    /// interrupt SOURCE — `ETS_FROM_CPU_INTR{n}_SOURCE` = `24 + n` — wired
    /// into *both* the PRO and APP interrupt matrices. While its bit 0 is
    /// set the source is level-asserted; each core delivers it to whatever
    /// CPU interrupt its own matrix MAP register binds the source to (or
    /// not at all if that core left it unmapped). The receiving ISR clears
    /// the source by writing the trigger register back to 0
    /// (`esp_crosscore_isr`), so this is a pure read of current register
    /// state — no latch.
    ///
    /// ESP-IDF's `esp_crosscore_int_init` allocates
    /// `ETS_FROM_CPU_INTR0_SOURCE + core_id`, so core 0 listens on source 24
    /// (trigger `FROM_CPU_0`) and core 1 on source 25 (trigger `FROM_CPU_1`);
    /// `esp_crosscore_int_send_yield(1)` writes `FROM_CPU_1` to yield APP_CPU.
    pub fn cross_core_pending(&self, core: u8) -> u32 {
        /// `ETS_FROM_CPU_INTR0_SOURCE` — the first cross-core interrupt source.
        const FROM_CPU_INTR0_SOURCE: u32 = 24;
        let mut slots = 0u32;
        for n in 0..4u32 {
            let trigger = self.read_word(DPORT_CPU_INTR_FROM_CPU_0_OFFSET + n * 4);
            if trigger & 1 == 0 {
                continue;
            }
            let source = FROM_CPU_INTR0_SOURCE + n;
            if let Some(slot) = self.map_source_slot(core, source) {
                slots |= 1u32 << slot;
            }
        }
        slots
    }

    /// Map a peripheral interrupt-matrix *source* to a CPU IRQ slot for `core`
    /// (0 = PRO, 1 = APP), or `None` if unbound (MAP == 0).
    ///
    /// TRM §7 / `dport_reg.h`: PRO map base `0x104`, APP map base `0x208`;
    /// source `s` lives at `base + s*4`, low 5 bits = CPU interrupt number.
    pub fn map_source_slot(&self, core: u8, source: u32) -> Option<u8> {
        let map_base = if core == 0 {
            DPORT_PRO_MAC_INTR_MAP_REG_OFFSET
        } else {
            DPORT_APP_MAC_INTR_MAP_REG_OFFSET
        };
        // Classic ESP32 has ~69 matrix sources (0..68). Bound generously.
        if source > 127 {
            return None;
        }
        let map = self.read_word(map_base + source * 4);
        if map == 0 {
            None
        } else {
            Some((map & 0x1F) as u8)
        }
    }

    /// Route a set of asserting peripheral source IDs into per-core CPU IRQ
    /// slot bitmasks (level-sensitive rebuild, same shape as S3 intmatrix).
    pub fn route_sources(&self, sources: &[u32]) -> [u32; 2] {
        let mut routed = [0u32; 2];
        for &src in sources {
            if let Some(slot) = self.map_source_slot(0, src) {
                routed[0] |= 1u32 << slot;
            }
            if let Some(slot) = self.map_source_slot(1, src) {
                routed[1] |= 1u32 << slot;
            }
        }
        routed
    }
}

impl Peripheral for Dport {
    // Inert walk: DPORT register bank (clock gating / intr-matrix mapping / cache plumbing, all write-settled); tick() is the trait-default no-op.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = (offset & !3) as u32;
        let byte_off = (offset & 3) * 8;
        let word = self.read_word(word_off);
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = (offset & !3) as u32;
        let byte_off = (offset & 3) * 8;
        // Read-modify-write so partial-byte writes don't clobber the
        // other three bytes of the word. PERIP_CLK_EN's all-ones seed
        // survives a single-byte poke this way.
        let mut word = self.read_word(word_off);
        word &= !(0xFFu32 << byte_off);
        word |= (value as u32) << byte_off;
        self.write_word(word_off, word);
        Ok(())
    }

    // Word-granular fast paths: the firmware DPORT polling loop (WASM IPI
    // bridge, intmatrix mapping reads) hits these every CPU cycle. The
    // default Peripheral trait impl issues four byte reads/writes, each
    // hashing the offset through SipHash to index `regs`. Overriding here
    // collapses that to one lookup. Reads and writes are pure
    // last-write-wins storage with no side effects, so a single-word
    // operation is byte-identical to the four-byte default path.
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_word(offset as u32 & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_word(offset as u32 & !3, value);
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Snap {
            regs: Vec<(u32, u32)>,
        }
        // Preserve the sparse on-wire format. Skip zero words so the
        // snapshot stays small (most of the 4 KiB window is unwritten
        // in practice).
        let snap = Snap {
            regs: self
                .regs
                .iter()
                .enumerate()
                .filter(|(_, v)| **v != 0)
                .map(|(i, v)| ((i as u32) << 2, *v))
                .collect(),
        };
        bincode::serialize(&snap).expect("bincode serialize Dport")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Snap {
            regs: Vec<(u32, u32)>,
        }
        let snap: Snap = bincode::deserialize(bytes).map_err(|e| {
            crate::SimulationError::NotImplemented(format!("Dport snapshot decode: {e}"))
        })?;
        *self.regs = [0u32; Self::WORDS];
        for (off, val) in snap.regs {
            self.regs[(off >> 2) as usize] = val;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_u32_at(p: &Dport, offset: u64) -> u32 {
        let mut v = 0u32;
        for i in 0..4u64 {
            v |= (p.read(offset + i).unwrap() as u32) << (i * 8);
        }
        v
    }

    fn write_u32_at(p: &mut Dport, offset: u64, value: u32) {
        for i in 0..4u64 {
            p.write(offset + i, ((value >> (i * 8)) & 0xFF) as u8)
                .unwrap();
        }
    }

    #[test]
    fn fresh_dport_reports_all_peripherals_clock_enabled() {
        let p = Dport::new();
        assert_eq!(
            read_u32_at(&p, DPORT_PERIP_CLK_EN_OFFSET as u64),
            0xFFFF_FFFF,
            "PERIP_CLK_EN must be seeded all-ones so firmware that consults \
             the gating bitmap before writing sees everything as live."
        );
    }

    #[test]
    fn fresh_dport_reports_no_peripheral_in_reset() {
        let p = Dport::new();
        assert_eq!(read_u32_at(&p, DPORT_PERIP_RST_EN_OFFSET as u64), 0);
    }

    #[test]
    fn fresh_dport_reports_undivided_cpu_clock() {
        let p = Dport::new();
        assert_eq!(read_u32_at(&p, DPORT_CPU_PER_CONF_OFFSET as u64), 0);
    }

    #[test]
    fn fresh_dport_reports_zero_at_appcpu_ctrl_b() {
        // APPCPU_CTRL_B @ 0x30 resets to 0 (HashMap default). Dual-core
        // release is not this bit alone: PRO calls boot-ROM
        // ets_set_appcpu_boot_addr, which unhalts a real APP_CPU that runs
        // call_start_cpu1.
        let p = Dport::new();
        assert_eq!(read_u32_at(&p, 0x30), 0);
    }

    #[test]
    fn dcache_dbug0_reports_cache_state_idle() {
        // cache_hal_suspend: read PRO/APP_DCACHE_DBUG0, extui bits[18:7],
        // spin until value == 1. Model reports idle without a full cache FSM.
        let p = Dport::new();
        let pro = read_u32_at(&p, DPORT_PRO_DCACHE_DBUG0_OFFSET as u64);
        let app = read_u32_at(&p, DPORT_APP_DCACHE_DBUG0_OFFSET as u64);
        let state = |w: u32| (w >> DPORT_CACHE_STATE_S) & DPORT_CACHE_STATE_V;
        assert_eq!(state(pro), DPORT_CACHE_STATE_IDLE, "PRO_CACHE_STATE");
        assert_eq!(state(app), DPORT_CACHE_STATE_IDLE, "APP_CACHE_STATE");
    }

    #[test]
    fn cross_core_yield_ipi_delivers_to_app_cpu_only() {
        // The path that quiesces APP_CPU to IDLE in the e-reader boot:
        // core 1 binds FROM_CPU_INTR1 (source 25) to a CPU interrupt via its
        // APP-side matrix MAP (0x208 + 25*4 = 0x26C), then a FROM_CPU_1 write
        // (esp_crosscore_int_send_yield(1)) delivers that interrupt to core 1
        // — and to core 1 only, since PRO left source 25 unbound.
        let mut p = Dport::new();
        write_u32_at(
            &mut p,
            (DPORT_APP_MAC_INTR_MAP_REG_OFFSET + 25 * 4) as u64,
            7,
        );

        assert_eq!(p.cross_core_pending(1), 0, "no IPI until the trigger fires");

        write_u32_at(&mut p, DPORT_CPU_INTR_FROM_CPU_1_OFFSET as u64, 1);
        assert_eq!(
            p.cross_core_pending(1),
            1 << 7,
            "APP_CPU takes the CPU interrupt its matrix bound source 25 to"
        );
        assert_eq!(
            p.cross_core_pending(0),
            0,
            "PRO_CPU left source 25 unbound, so it ignores FROM_CPU_1"
        );

        // The receiving ISR clears the trigger; the source de-asserts.
        write_u32_at(&mut p, DPORT_CPU_INTR_FROM_CPU_1_OFFSET as u64, 0);
        assert_eq!(p.cross_core_pending(1), 0);
    }

    #[test]
    fn cross_core_trigger_with_unmapped_source_delivers_nothing() {
        // A trigger whose per-core MAP is still at its reset value (0) must
        // not synthesize a phantom interrupt 0.
        let mut p = Dport::new();
        write_u32_at(&mut p, DPORT_CPU_INTR_FROM_CPU_1_OFFSET as u64, 1);
        assert_eq!(p.cross_core_pending(1), 0);
    }

    #[test]
    fn cross_core_from_cpu_0_routes_source_24_to_pro_cpu() {
        // FROM_CPU_0 is source 24, the core-0 crosscore IPI; PRO binds it in
        // its own matrix (0x100 + 24*4 = 0x160).
        let mut p = Dport::new();
        write_u32_at(
            &mut p,
            (DPORT_PRO_MAC_INTR_MAP_REG_OFFSET + 24 * 4) as u64,
            13,
        );
        write_u32_at(&mut p, DPORT_CPU_INTR_FROM_CPU_0_OFFSET as u64, 1);
        assert_eq!(p.cross_core_pending(0), 1 << 13);
        assert_eq!(p.cross_core_pending(1), 0);
    }

    #[test]
    fn cpu_intr_from_cpu_0_round_trips() {
        // The WASM IPI bridge polls this register every cycle and expects
        // last-write-wins semantics — its detect-and-clear loop depends on
        // reading back exactly what it wrote.
        let mut p = Dport::new();
        write_u32_at(&mut p, DPORT_CPU_INTR_FROM_CPU_0_OFFSET as u64, 0x0000_0001);
        assert_eq!(
            read_u32_at(&p, DPORT_CPU_INTR_FROM_CPU_0_OFFSET as u64),
            0x0000_0001
        );
        // And clearing it back to 0 sticks.
        write_u32_at(&mut p, DPORT_CPU_INTR_FROM_CPU_0_OFFSET as u64, 0);
        assert_eq!(read_u32_at(&p, DPORT_CPU_INTR_FROM_CPU_0_OFFSET as u64), 0);
    }

    #[test]
    fn pro_intr_from_cpu_0_round_trips() {
        // PRO_INTR_FROM_CPU_0 lives in the intmatrix-style mapping region
        // (0x100..0x180). The WASM bridge reads this to discover which
        // CPU INTERRUPT bit to raise — must round-trip verbatim.
        let mut p = Dport::new();
        write_u32_at(&mut p, DPORT_PRO_INTR_FROM_CPU_0_OFFSET as u64, 0x0000_001F);
        assert_eq!(
            read_u32_at(&p, DPORT_PRO_INTR_FROM_CPU_0_OFFSET as u64),
            0x0000_001F
        );
    }

    #[test]
    fn perip_clk_en_byte_writes_dont_clobber_seeded_ones() {
        // Writing one byte must read-modify-write the word so the other
        // three bytes (all 0xFF from the seed) are preserved.
        let mut p = Dport::new();
        p.write(DPORT_PERIP_CLK_EN_OFFSET as u64, 0x00).unwrap();
        let v = read_u32_at(&p, DPORT_PERIP_CLK_EN_OFFSET as u64);
        assert_eq!(v, 0xFFFF_FF00, "byte 0 cleared, bytes 1..3 preserved");
    }

    #[test]
    fn pro_mac_intr_map_round_trips_a_slot_binding() {
        // The intmatrix region: PRO_MAC_INTR_MAP_REG = 0x100, write slot
        // 13 → read back 13.
        let mut p = Dport::new();
        write_u32_at(&mut p, DPORT_PRO_MAC_INTR_MAP_REG_OFFSET as u64, 13);
        assert_eq!(
            read_u32_at(&p, DPORT_PRO_MAC_INTR_MAP_REG_OFFSET as u64),
            13
        );
    }

    #[test]
    fn unwritten_offsets_read_as_zero() {
        let p = Dport::new();
        // Any random unmodeled offset in the DPORT window reads 0.
        assert_eq!(read_u32_at(&p, 0x200), 0);
        assert_eq!(read_u32_at(&p, 0x3FC), 0);
        assert_eq!(read_u32_at(&p, 0xABC), 0);
    }

    #[test]
    fn runtime_snapshot_round_trip_preserves_state() {
        let mut p = Dport::new();
        write_u32_at(&mut p, DPORT_CPU_INTR_FROM_CPU_0_OFFSET as u64, 0x0000_0001);
        write_u32_at(&mut p, DPORT_PRO_INTR_FROM_CPU_0_OFFSET as u64, 0x0000_001F);
        write_u32_at(&mut p, DPORT_PERIP_RST_EN_OFFSET as u64, 0xAAAA_5555);
        let snap = p.runtime_snapshot();

        let mut restored = Dport::new();
        restored.restore_runtime_snapshot(&snap).unwrap();
        assert_eq!(
            read_u32_at(&restored, DPORT_CPU_INTR_FROM_CPU_0_OFFSET as u64),
            0x0000_0001
        );
        assert_eq!(
            read_u32_at(&restored, DPORT_PRO_INTR_FROM_CPU_0_OFFSET as u64),
            0x0000_001F
        );
        assert_eq!(
            read_u32_at(&restored, DPORT_PERIP_RST_EN_OFFSET as u64),
            0xAAAA_5555
        );
    }

    #[test]
    fn base_is_esp32_classic_canonical_address() {
        let p = Dport::new();
        assert_eq!(p.base(), 0x3FF0_0000);
        assert_eq!(Dport::SIZE, 0x1000);
    }
}
