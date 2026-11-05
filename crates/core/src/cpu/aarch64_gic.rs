// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The GICv3 and the generic timer of one [`super::Aarch64`] PE.
//!
//! Both are folded into the core, as the RV64 hart folds in its CLINT: the
//! CPU interface is system registers (`ICC_*_EL1`), the timer is system
//! registers (`CNT*`), and the distributor/redistributor MMIO windows are
//! claimed by the core before an access reaches the bus. One PE, so one
//! redistributor (`GICR_TYPER.Last` = 1).
//!
//! What is modelled (Arm GICv3/v4 Architecture Specification, IHI 0069H;
//! Arm ARM DDI 0487 D11 "The Generic Timer"):
//!
//! * Distributor: `GICD_CTLR` (stored; RWP reads 0), `TYPER`, `IIDR`,
//!   `PIDR2` (ArchRev 3), and per-SPI `IGROUPR`, `I[SC]ENABLER`, `I[SC]PENDR`,
//!   `I[SC]ACTIVER`, `IPRIORITYR`, `ICFGR`, `IGRPMODR`, `IROUTER`.
//! * Redistributor: `GICR_CTLR`, `IIDR`, `TYPER` (affinity from `MPIDR_EL1`,
//!   Last), `WAKER` (ChildrenAsleep follows ProcessorSleep), `PIDR2`, and the
//!   SGI frame's SGI/PPI copies of the per-interrupt registers.
//! * CPU interface: `IAR1` (acknowledge the highest-priority pending enabled
//!   interrupt above the running priority and `PMR`), `EOIR1` (priority drop
//!   plus deactivate, EOImode 0), `HPPIR1`, `RPR`, `PMR`, `BPR1`, `CTLR`,
//!   `IGRPEN1`, `SRE_EL1/2/3`, `SGI1R` (to this PE).
//! * Level-sensitive timer PPIs: a timer's line is its `ISTATUS && ENABLE &&
//!   !IMASK`, re-evaluated on every look, so acknowledging it does not clear
//!   it and it re-pends after EOI if the compare still holds.
//!
//! Not modelled: interrupt groups and security as separate signals (every
//! enabled interrupt is signalled as an IRQ through `IAR1`; `IGROUPR` /
//! `IGRPMODR` are storage), binary-point preemption subgroups (preemption is
//! on the full priority), EOImode 1 / `DIR`, LPIs and the ITS, and 1-of-N
//! routing (there is one PE).

/// SGIs and PPIs: INTIDs below this are banked in the redistributor.
pub const PRIVATE_INTIDS: usize = 32;
/// The value `IAR1` returns when nothing is pending.
pub const SPURIOUS: u32 = 1023;

/// Generic timer PPIs (Arm BSA / every GICv3 SoC's devicetree).
pub const PPI_HYP_TIMER: usize = 26;
pub const PPI_VIRT_TIMER: usize = 27;
pub const PPI_SEC_PHYS_TIMER: usize = 29;
pub const PPI_PHYS_TIMER: usize = 30;

const GICD_SIZE: u64 = 0x1_0000;
/// One redistributor: RD_base frame plus SGI_base frame.
const GICR_SIZE: u64 = 0x2_0000;
const GICR_SGI_OFFSET: u64 = 0x1_0000;

const CTLR_RWP: u32 = 1 << 31;
const WAKER_PS: u32 = 1 << 1;
const WAKER_CA: u32 = 1 << 2;

/// One generic-timer channel (`CNT{P,V,HP,PS}_{CTL,CVAL}`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TimerChannel {
    /// `CTL` as written: ENABLE (bit 0), IMASK (bit 1). ISTATUS is computed.
    pub ctl: u64,
    pub cval: u64,
}

impl TimerChannel {
    pub fn istatus(&self, count: u64) -> bool {
        self.ctl & 1 != 0 && count >= self.cval
    }

    /// The interrupt line: condition met, timer enabled, output not masked.
    pub fn line(&self, count: u64) -> bool {
        self.istatus(count) && self.ctl & 2 == 0
    }

    pub fn read_ctl(&self, count: u64) -> u64 {
        (self.ctl & 3) | (u64::from(self.istatus(count)) << 2)
    }

    /// `TVAL` reads as the signed 32-bit distance to the compare value.
    pub fn read_tval(&self, count: u64) -> u64 {
        u64::from(self.cval.wrapping_sub(count) as u32)
    }

    pub fn write_tval(&mut self, count: u64, v: u64) {
        self.cval = count.wrapping_add(v as u32 as i32 as i64 as u64);
    }
}

/// The system counter and the PE's timers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GenericTimer {
    /// Physical count (`CNTPCT_EL0`).
    pub count: u64,
    /// CPU clocks per counter tick (CPU clock / system counter clock).
    pub divider: u64,
    /// CPU clocks accumulated toward the next tick.
    pub residue: u64,
    pub cntfrq: u64,
    pub cntvoff: u64,
    pub cnthctl: u64,
    pub cntkctl: u64,
    pub phys: TimerChannel,
    pub virt: TimerChannel,
    pub hyp: TimerChannel,
    pub sec_phys: TimerChannel,
}

impl GenericTimer {
    pub fn new(divider: u64) -> Self {
        Self {
            count: 0,
            divider: divider.max(1),
            residue: 0,
            cntfrq: 0,
            cntvoff: 0,
            cnthctl: 0,
            cntkctl: 0,
            phys: TimerChannel::default(),
            virt: TimerChannel::default(),
            hyp: TimerChannel::default(),
            sec_phys: TimerChannel::default(),
        }
    }

    pub fn advance(&mut self, cycles: u64) {
        if cycles == 1 {
            // The per-instruction path: no division.
            self.residue += 1;
            if self.residue >= self.divider {
                self.residue = 0;
                self.count = self.count.wrapping_add(1);
            }
            return;
        }
        let total = self.residue + cycles;
        self.count = self.count.wrapping_add(total / self.divider);
        self.residue = total % self.divider;
    }

    pub fn virtual_count(&self) -> u64 {
        self.count.wrapping_sub(self.cntvoff)
    }

    /// Whether any timer line is asserted (cheap check for the step loop).
    pub fn any_line(&self) -> bool {
        self.phys.line(self.count)
            || self.virt.line(self.virtual_count())
            || self.hyp.line(self.count)
            || self.sec_phys.line(self.count)
    }

    /// Timer PPI lines, by INTID.
    pub fn ppi_lines(&self) -> [(usize, bool); 4] {
        [
            (PPI_PHYS_TIMER, self.phys.line(self.count)),
            (PPI_VIRT_TIMER, self.virt.line(self.virtual_count())),
            (PPI_HYP_TIMER, self.hyp.line(self.count)),
            (PPI_SEC_PHYS_TIMER, self.sec_phys.line(self.count)),
        ]
    }

    /// CPU clocks until the first unmasked, enabled channel fires, `None` if
    /// none is armed (or one already has).
    pub fn cycles_until_line(&self) -> Option<u64> {
        let mut best: Option<u64> = None;
        let mut consider = |ch: &TimerChannel, count: u64| {
            if ch.ctl & 1 == 0 || ch.ctl & 2 != 0 {
                return;
            }
            if count >= ch.cval {
                best = Some(0);
                return;
            }
            let c = (ch.cval - count)
                .saturating_mul(self.divider)
                .saturating_sub(self.residue);
            best = Some(best.map_or(c, |b| b.min(c)));
        };
        consider(&self.phys, self.count);
        consider(&self.virt, self.virtual_count());
        consider(&self.hyp, self.count);
        consider(&self.sec_phys, self.count);
        best
    }
}

/// Distributor, redistributor and CPU-interface state for one PE.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Gic {
    pub dist_base: Option<u64>,
    pub redist_base: Option<u64>,
    /// INTIDs implemented (32 private + SPIs), a multiple of 32.
    pub num_intids: usize,
    /// Affinity this redistributor reports (`MPIDR_EL1` Aff3..Aff0).
    pub affinity: u64,

    pub gicd_ctlr: u32,
    pub gicr_ctlr: u32,
    pub gicr_waker: u32,

    pub enabled: Vec<bool>,
    /// Latched pending state (edge, software-set or peripheral-set).
    pub pending: Vec<bool>,
    pub active: Vec<bool>,
    pub priority: Vec<u8>,
    pub group: Vec<bool>,
    pub grpmod: Vec<bool>,
    /// `ICFGR` per-interrupt: true = edge-triggered.
    pub edge: Vec<bool>,
    pub router: Vec<u64>,

    pub pmr: u8,
    pub bpr1: u8,
    pub icc_ctlr: u64,
    pub igrpen1: bool,
    pub igrpen0: bool,
    pub sre_el1: u64,
    pub sre_el2: u64,
    pub sre_el3: u64,
    /// Priorities of the interrupts acknowledged and not yet EOId, innermost
    /// last, with the INTID each belongs to.
    pub active_stack: Vec<(u32, u8)>,
    /// Some latched pending bit is set: a summary of `pending`, refreshed
    /// whenever it changes, so the per-instruction check does not scan.
    #[serde(default)]
    pub any_latched: bool,
}

impl Gic {
    pub fn new(
        dist_base: Option<u64>,
        redist_base: Option<u64>,
        num_spis: usize,
        affinity: u64,
    ) -> Self {
        let n = (PRIVATE_INTIDS + num_spis).div_ceil(32) * 32;
        let mut edge = vec![false; n];
        // SGIs are always edge-triggered.
        for e in edge.iter_mut().take(16) {
            *e = true;
        }
        Self {
            dist_base,
            redist_base,
            num_intids: n,
            affinity,
            gicd_ctlr: 0,
            gicr_ctlr: 0,
            // Out of reset the redistributor is asleep (IHI 0069H 12.11.42).
            gicr_waker: WAKER_PS | WAKER_CA,
            enabled: vec![false; n],
            pending: vec![false; n],
            active: vec![false; n],
            priority: vec![0; n],
            group: vec![false; n],
            grpmod: vec![false; n],
            edge,
            router: vec![0; n],
            pmr: 0,
            bpr1: 0,
            icc_ctlr: 0,
            igrpen1: false,
            igrpen0: false,
            // SRE is RAO/WI here: this GIC has no memory-mapped CPU
            // interface, so the system-register interface is the only one.
            sre_el1: 1,
            sre_el2: 1,
            sre_el3: 1,
            active_stack: Vec::new(),
            any_latched: false,
        }
    }

    fn refresh_latched(&mut self) {
        self.any_latched = self.pending.iter().any(|p| *p);
    }

    pub fn dist_offset(&self, addr: u64) -> Option<u64> {
        let base = self.dist_base?;
        addr.checked_sub(base).filter(|o| *o < GICD_SIZE)
    }

    pub fn redist_offset(&self, addr: u64) -> Option<u64> {
        let base = self.redist_base?;
        addr.checked_sub(base).filter(|o| *o < GICR_SIZE)
    }

    pub fn running_priority(&self) -> u8 {
        self.active_stack.last().map_or(0xFF, |(_, p)| *p)
    }

    fn pending_now(&self, id: usize, timer: &GenericTimer) -> bool {
        if self.pending[id] {
            return true;
        }
        timer
            .ppi_lines()
            .iter()
            .any(|(ppi, line)| *ppi == id && *line && !self.edge[id])
    }

    fn forwarded(&self, id: usize) -> bool {
        if !self.enabled[id] {
            return false;
        }
        if id >= PRIVATE_INTIDS {
            // Any group enable in the distributor (bits 0..2).
            if self.gicd_ctlr & 0b111 == 0 {
                return false;
            }
        }
        self.igrpen1 || self.igrpen0
    }

    /// Highest-priority pending, enabled, inactive interrupt (lowest INTID
    /// on a tie), regardless of masking.
    pub fn highest_pending(&self, timer: &GenericTimer) -> Option<(u32, u8)> {
        let mut best: Option<(u32, u8)> = None;
        for id in 0..self.num_intids {
            if self.active[id] || !self.forwarded(id) || !self.pending_now(id, timer) {
                continue;
            }
            let p = self.priority[id];
            if best.is_none_or(|(_, bp)| p < bp) {
                best = Some((id as u32, p));
            }
        }
        best
    }

    /// The INTID that would be signalled to the PE as an IRQ now.
    pub fn signalled(&self, timer: &GenericTimer) -> Option<u32> {
        if !self.any_latched && !timer.any_line() {
            return None;
        }
        let (id, p) = self.highest_pending(timer)?;
        (p < self.pmr && p < self.running_priority()).then_some(id)
    }

    /// `ICC_IAR1_EL1` read: acknowledge.
    pub fn acknowledge(&mut self, timer: &GenericTimer) -> u32 {
        let Some(id) = self.signalled(timer) else {
            return SPURIOUS;
        };
        let i = id as usize;
        self.active[i] = true;
        // An edge interrupt's pending latch is consumed by the acknowledge.
        // A level interrupt's latch (a peripheral line set through
        // `set_pending`) is also consumed: the peripheral re-asserts it.
        self.pending[i] = false;
        self.refresh_latched();
        self.active_stack.push((id, self.priority[i]));
        id
    }

    /// `ICC_EOIR1_EL1` write: priority drop and deactivate (EOImode 0).
    pub fn end_of_interrupt(&mut self, id: u32) {
        if let Some(pos) = self.active_stack.iter().rposition(|(i, _)| *i == id) {
            self.active_stack.remove(pos);
        }
        if let Some(a) = self.active.get_mut(id as usize) {
            *a = false;
        }
    }

    pub fn set_pending(&mut self, id: u32) {
        if let Some(p) = self.pending.get_mut(id as usize) {
            *p = true;
            self.any_latched = true;
        }
    }

    /// `ICC_SGI1R_EL1` write: an SGI to this PE when the target list or
    /// IRM names it.
    pub fn write_sgi1r(&mut self, v: u64) {
        let intid = ((v >> 24) & 0xF) as usize;
        let irm = v & (1 << 40) != 0;
        let aff1 = (v >> 16) & 0xFF;
        let aff2 = (v >> 32) & 0xFF;
        let aff3 = (v >> 48) & 0xFF;
        let targets = v & 0xFFFF;
        let my_aff0 = self.affinity & 0xFF;
        let mine = (aff1 == (self.affinity >> 8) & 0xFF)
            && (aff2 == (self.affinity >> 16) & 0xFF)
            && (aff3 == (self.affinity >> 32) & 0xFF)
            && my_aff0 < 16
            && targets & (1 << my_aff0) != 0;
        // IRM=1 means "all PEs except self".
        if !irm && mine {
            self.pending[intid] = true;
            self.any_latched = true;
        }
    }

    // ---- MMIO ------------------------------------------------------------

    fn read_bits(v: &[bool], first: usize) -> u32 {
        (0..32).fold(0, |acc, b| {
            acc | (u32::from(v.get(first + b).copied().unwrap_or(false)) << b)
        })
    }

    fn set_bits(v: &mut [bool], first: usize, mask: u32, value: bool) {
        for b in 0..32 {
            if mask & (1 << b) != 0 {
                if let Some(x) = v.get_mut(first + b) {
                    *x = value;
                }
            }
        }
    }

    fn assign_bits(v: &mut [bool], first: usize, word: u32) {
        for b in 0..32 {
            if let Some(x) = v.get_mut(first + b) {
                *x = word & (1 << b) != 0;
            }
        }
    }

    /// The per-interrupt register block shared by GICD (SPIs) and the GICR
    /// SGI frame (INTIDs 0-31). `first_ok` limits which INTIDs this frame
    /// owns. Returns `None` for offsets outside the block.
    fn per_irq_read(
        &self,
        off: u64,
        timer: &GenericTimer,
        own: &dyn Fn(usize) -> bool,
    ) -> Option<u32> {
        let word = |base: u64| ((off - base) / 4) as usize * 32;
        let masked = |w: u32, first: usize| -> u32 {
            (0..32).fold(0, |acc, b| {
                if own(first + b) {
                    acc | (w & (1 << b))
                } else {
                    acc
                }
            })
        };
        Some(match off {
            0x080..=0x0FF => masked(Self::read_bits(&self.group, word(0x080)), word(0x080)),
            0x100..=0x17F => masked(Self::read_bits(&self.enabled, word(0x100)), word(0x100)),
            0x180..=0x1FF => masked(Self::read_bits(&self.enabled, word(0x180)), word(0x180)),
            0x200..=0x2FF => {
                let first = word(if off < 0x280 { 0x200 } else { 0x280 });
                let w = (0..32).fold(0u32, |acc, b| {
                    let id = first + b;
                    if id < self.num_intids && self.pending_now(id, timer) {
                        acc | (1 << b)
                    } else {
                        acc
                    }
                });
                masked(w, first)
            }
            0x300..=0x37F => masked(Self::read_bits(&self.active, word(0x300)), word(0x300)),
            0x380..=0x3FF => masked(Self::read_bits(&self.active, word(0x380)), word(0x380)),
            0x400..=0x7FF => {
                let first = (off - 0x400) as usize & !3;
                (0..4).fold(0, |acc, i| {
                    let id = first + i;
                    if own(id) {
                        acc | (u32::from(self.priority.get(id).copied().unwrap_or(0)) << (8 * i))
                    } else {
                        acc
                    }
                })
            }
            0xC00..=0xCFF => {
                let first = ((off - 0xC00) / 4) as usize * 16;
                (0..16).fold(0, |acc, i| {
                    let id = first + i;
                    if own(id) && self.edge.get(id).copied().unwrap_or(false) {
                        acc | (2 << (2 * i))
                    } else {
                        acc
                    }
                })
            }
            0xD00..=0xD7F => masked(Self::read_bits(&self.grpmod, word(0xD00)), word(0xD00)),
            _ => return None,
        })
    }

    fn per_irq_write(
        &mut self,
        off: u64,
        size: u64,
        value: u32,
        own: &dyn Fn(usize) -> bool,
    ) -> bool {
        let word = |base: u64| ((off - base) / 4) as usize * 32;
        let own_mask = |first: usize| -> u32 {
            (0..32).fold(
                0,
                |acc, b| if own(first + b) { acc | (1 << b) } else { acc },
            )
        };
        match off {
            0x080..=0x0FF => {
                let f = word(0x080);
                let m = own_mask(f);
                Self::set_bits(&mut self.group, f, m & value, true);
                Self::set_bits(&mut self.group, f, m & !value, false);
            }
            0x100..=0x17F => {
                let f = word(0x100);
                Self::set_bits(&mut self.enabled, f, own_mask(f) & value, true);
            }
            0x180..=0x1FF => {
                let f = word(0x180);
                Self::set_bits(&mut self.enabled, f, own_mask(f) & value, false);
            }
            0x200..=0x27F => {
                let f = word(0x200);
                Self::set_bits(&mut self.pending, f, own_mask(f) & value, true);
            }
            0x280..=0x2FF => {
                let f = word(0x280);
                Self::set_bits(&mut self.pending, f, own_mask(f) & value, false);
            }
            0x300..=0x37F => {
                let f = word(0x300);
                Self::set_bits(&mut self.active, f, own_mask(f) & value, true);
            }
            0x380..=0x3FF => {
                let f = word(0x380);
                Self::set_bits(&mut self.active, f, own_mask(f) & value, false);
            }
            0x400..=0x7FF => {
                let first = (off - 0x400) as usize;
                for i in 0..size.min(4) as usize {
                    let id = first + i;
                    if own(id) {
                        if let Some(p) = self.priority.get_mut(id) {
                            *p = (value >> (8 * i)) as u8;
                        }
                    }
                }
            }
            0xC00..=0xCFF => {
                let first = ((off - 0xC00) / 4) as usize * 16;
                for i in 0..16 {
                    let id = first + i;
                    // SGI configuration is read-only (always edge).
                    if own(id) && id >= 16 {
                        if let Some(e) = self.edge.get_mut(id) {
                            *e = value & (2 << (2 * i)) != 0;
                        }
                    }
                }
            }
            0xD00..=0xD7F => {
                let f = word(0xD00);
                let m = own_mask(f);
                let mut cur = Self::read_bits(&self.grpmod, f);
                cur = (cur & !m) | (value & m);
                Self::assign_bits(&mut self.grpmod, f, cur);
            }
            _ => return false,
        }
        self.refresh_latched();
        true
    }

    pub fn dist_read(&self, off: u64, size: u64, timer: &GenericTimer) -> u64 {
        let n = self.num_intids;
        let spi = move |id: usize| (PRIVATE_INTIDS..n).contains(&id);
        let aligned = off & !3;
        let v: u64 = match aligned {
            0x0000 => u64::from(self.gicd_ctlr & !CTLR_RWP),
            // ITLinesNumber, IDbits = 10 bits (no LPIs), no ESPI.
            0x0004 => (((n / 32) - 1) as u64) | (9 << 19),
            // Arm implementer code, GIC-600-like product id.
            0x0008 => 0x0200_043B,
            0xFFE8 => 0x3 << 4,
            0x6000..=0x7FFF => {
                let id = ((off - 0x6000) / 8) as usize;
                let r = self.router.get(id).copied().unwrap_or(0);
                if off & 4 != 0 {
                    r >> 32
                } else if size >= 8 {
                    r
                } else {
                    r & 0xFFFF_FFFF
                }
            }
            _ => u64::from(self.per_irq_read(aligned, timer, &spi).unwrap_or(0)),
        };
        let shift = (off & 3) * 8;
        if size >= 8 || (0x6000..=0x7FFF).contains(&aligned) {
            v
        } else {
            (v >> shift) & ((1u64 << (size * 8)) - 1)
        }
    }

    pub fn dist_write(&mut self, off: u64, size: u64, value: u64) {
        let n = self.num_intids;
        let spi = move |id: usize| (PRIVATE_INTIDS..n).contains(&id);
        match off {
            0x0000 => self.gicd_ctlr = value as u32 & !CTLR_RWP,
            0x6000..=0x7FFF => {
                let id = ((off - 0x6000) / 8) as usize;
                if let Some(r) = self.router.get_mut(id) {
                    if size >= 8 {
                        *r = value;
                    } else if off & 4 != 0 {
                        *r = (*r & 0xFFFF_FFFF) | (value << 32);
                    } else {
                        *r = (*r & !0xFFFF_FFFF) | (value & 0xFFFF_FFFF);
                    }
                }
            }
            // Priority bytes are written byte-exact, not word-aligned.
            0x0400..=0x07FF => {
                let _ = self.per_irq_write(off, size, value as u32, &spi);
            }
            _ => {
                let _ = self.per_irq_write(off & !3, size, value as u32, &spi);
            }
        }
    }

    pub fn redist_read(&self, off: u64, size: u64, timer: &GenericTimer) -> u64 {
        let private = |id: usize| id < PRIVATE_INTIDS;
        let aligned = off & !3;
        let v: u64 = if off >= GICR_SGI_OFFSET {
            u64::from(
                self.per_irq_read(aligned - GICR_SGI_OFFSET, timer, &private)
                    .unwrap_or(0),
            )
        } else {
            match aligned {
                0x0000 => u64::from(self.gicr_ctlr & !CTLR_RWP),
                0x0004 => 0x0200_043B,
                0x0008 | 0x000C => {
                    let aff = ((self.affinity >> 32) & 0xFF) << 24 | (self.affinity & 0xFF_FFFF);
                    // Last (bit 4): the only redistributor.
                    let typer = (aff << 32) | (1 << 4);
                    if size >= 8 {
                        return typer;
                    }
                    if aligned == 0x000C {
                        typer >> 32
                    } else {
                        typer & 0xFFFF_FFFF
                    }
                }
                0x0014 => u64::from(self.gicr_waker),
                0xFFE8 => 0x3 << 4,
                _ => 0,
            }
        };
        let shift = (off & 3) * 8;
        if size >= 8 {
            v
        } else {
            (v >> shift) & ((1u64 << (size * 8)) - 1)
        }
    }

    pub fn redist_write(&mut self, off: u64, size: u64, value: u64) {
        let private = |id: usize| id < PRIVATE_INTIDS;
        if off >= GICR_SGI_OFFSET {
            let o = off - GICR_SGI_OFFSET;
            if (0x400..=0x41F).contains(&o) {
                let _ = self.per_irq_write(o, size, value as u32, &private);
            } else {
                let _ = self.per_irq_write(o & !3, size, value as u32, &private);
            }
            return;
        }
        match off {
            0x0000 => self.gicr_ctlr = value as u32 & !CTLR_RWP,
            0x0014 => {
                let ps = value as u32 & WAKER_PS;
                // ChildrenAsleep follows ProcessorSleep at once.
                self.gicr_waker = ps | if ps != 0 { WAKER_CA } else { 0 };
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gic() -> Gic {
        let mut g = Gic::new(Some(0x0180_0000), Some(0x0188_0000), 256, 0);
        g.gicd_ctlr = 0x12;
        g.igrpen1 = true;
        g.pmr = 0xF0;
        g
    }

    #[test]
    fn spi_acknowledge_and_eoi() {
        let t = GenericTimer::new(1);
        let mut g = gic();
        g.dist_write(0x100 + 4 * (210 / 32), 4, 1 << (210 % 32));
        g.dist_write(0x400 + 210, 1, 0xA0);
        assert_eq!(g.acknowledge(&t), SPURIOUS);
        g.set_pending(210);
        assert_eq!(g.signalled(&t), Some(210));
        assert_eq!(g.acknowledge(&t), 210);
        assert_eq!(g.running_priority(), 0xA0);
        assert_eq!(g.signalled(&t), None);
        g.end_of_interrupt(210);
        assert_eq!(g.running_priority(), 0xFF);
    }

    #[test]
    fn level_timer_ppi_repends_after_eoi() {
        let mut t = GenericTimer::new(1);
        let mut g = gic();
        g.redist_write(GICR_SGI_OFFSET + 0x100, 4, 1 << PPI_VIRT_TIMER);
        g.redist_write(GICR_SGI_OFFSET + 0x400 + PPI_VIRT_TIMER as u64, 1, 0xA0);
        t.virt = TimerChannel { ctl: 1, cval: 10 };
        assert_eq!(g.signalled(&t), None);
        t.advance(10);
        assert_eq!(g.acknowledge(&t), PPI_VIRT_TIMER as u32);
        g.end_of_interrupt(PPI_VIRT_TIMER as u32);
        // Still asserted: pending again.
        assert_eq!(g.signalled(&t), Some(PPI_VIRT_TIMER as u32));
        t.virt.ctl |= 2; // IMASK
        assert_eq!(g.signalled(&t), None);
    }

    #[test]
    fn redistributor_wakes_and_reports_last() {
        let t = GenericTimer::new(1);
        let mut g = gic();
        assert_ne!(g.redist_read(0x14, 4, &t) as u32 & WAKER_CA, 0);
        g.redist_write(0x14, 4, 0);
        assert_eq!(g.redist_read(0x14, 4, &t), 0);
        assert_ne!(g.redist_read(0x8, 8, &t) & (1 << 4), 0);
    }

    #[test]
    fn pmr_masks_lower_priority() {
        let t = GenericTimer::new(1);
        let mut g = gic();
        g.dist_write(0x100 + 4, 4, 1);
        g.dist_write(0x400 + 32, 1, 0xF8);
        g.set_pending(32);
        assert_eq!(g.signalled(&t), None);
        g.pmr = 0xFF;
        assert_eq!(g.signalled(&t), Some(32));
    }
}
