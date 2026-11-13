// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Renesas RA interrupt controller unit: the event link (`IELSRn`) and the
//! external IRQ pins (`IRQCRn`).
//!
//! On RA parts no peripheral is wired to a fixed NVIC line. Each peripheral
//! interrupt is an ELC *event number*, and `IELSRn` (one per NVIC line n)
//! selects which event drives line n. Source: FSP `R7FA8M1AH.h`
//! `R_ICU_Type` (`IELSR[96]` at R_ICU_BASE + 0x6300; `IELS` bits 8:0, `IR`
//! bit 16, `DTCE` bit 24) and FSP `bsp_irq.h` `R_BSP_IrqStatusClear` ("When
//! an interrupt is triggered the IR bit is set. If it is not cleared in the
//! ISR then the interrupt will trigger again immediately.").
//!
//! This model covers the 2 KiB window from R_ICU_BASE + 0x6000 (0x4000_C000
//! on the RA8M1, `WUPEN0/1` at +0x1A0/+0x1A4 and `IELSR0-95` at +0x300).
//! Every word in the window is plain storage; `IELSRn` adds:
//! - an event linked to line n (`IELS == event`, `IELS != 0`) sets `IR` and
//!   drives NVIC line n;
//! - `IR` is the level of line n ([`Peripheral::irq_line_levels`]), so the
//!   bus drops a pend raised by the link once firmware clears `IR`;
//! - `IR` is write-0-to-clear: a store of 1 leaves it as it was.
//!
//! IRQ-pin half (a window that starts at R_ICU_BASE, `irq_pins` configured):
//! `IRQCRn` (bytes at +0x000..+0x00F; `IRQMD` bits 1:0 = falling, rising,
//! both, low level) selects the edge that raises ELC event `ICU_IRQn`
//! (event number n + 1, FSP `bsp_elc.h` `ELC_EVENT_ICU_IRQ0 = 0x001` on the
//! RA6M5 and RA8M1). A pad is an IRQn input only while its `PmnPFS.ISEL`
//! (bit 14) is set (FSP `R_PFS_PORT_PIN_PmnPFS_ISEL_Pos`), so a matching
//! edge is checked against that bit over the bus before the event leaves.
//! Which pad feeds which IRQn is fixed per part: `irq_pins` lists
//! `[port, pin, n]` from the part's pin-function table (Zephyr
//! `r7fa8m1xh.dtsi` / `r7fa6m5xh.dtsi` `port-irqN-pins`). The event goes out
//! through `explicit_irqs` like any other ELC source, so the IELSR window
//! links it — on the RA8M1 that is the other `ra_icu` instance at
//! R_ICU_BASE + 0x6000 (this one is `ielsr_lines: 0`).
//!
//! NOT modelled: `DTCE` (stored, no DTC activation); NMI (`NMICR`/`NMIER`,
//! stored only); the `IRQCRn` digital filter (`FLTEN`/`FCLKSEL`: an edge is
//! taken at once, never filtered out); low-level mode raises on the falling
//! edge only, not again while the pad stays low; and the re-entry when a
//! handler returns with `IR` still set — the line is only re-pended by the
//! next linked event, not by the level alone, because walking this block
//! per cycle while any `IR` is held would stop idle fast-forward.

use crate::{Peripheral, SimResult};

/// Window size: R_ICU_BASE + 0x6000 .. + 0x6800.
const WINDOW: usize = 0x800;
/// `IELSR0` offset in the window (R_ICU_BASE + 0x6300).
pub const IELSR_OFFSET: u64 = 0x300;
/// RA8M1 `IELSR` count (NVIC lines 0..95).
pub const LINES: u32 = 96;
const IELS_MASK: u32 = 0x1FF;
const IR: u32 = 1 << 16;
const DTCE: u32 = 1 << 24;
/// `IRQCR0..15`: one byte each from the start of the window.
const IRQCR_COUNT: u8 = 16;
const IRQMD_MASK: u8 = 0x03;
/// `ELC_EVENT_ICU_IRQ0`; IRQn is this + n.
const ICU_IRQ0_EVENT: u32 = 0x001;
/// `PmnPFS.ISEL`: the pad is an IRQ input.
const PFS_ISEL: u32 = 1 << 14;

/// An edge on an IRQ pad that matched its `IRQCRn`, waiting for the
/// `PmnPFS.ISEL` check in `tick_with_bus`.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
struct PinEdge {
    port: u8,
    pin: u8,
    irq: u8,
}

/// The RA ICU: event-link registers and external IRQ pins.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RaIcu {
    words: Vec<u32>,
    /// `IELSRn` count; 0 for an instance that is only the IRQ-pin half.
    #[serde(default)]
    lines: u32,
    /// `(port, pin, IRQn)` for every pad wired to an IRQ input.
    #[serde(default)]
    irq_pins: Vec<(u8, u8, u8)>,
    /// `PmnPFS` base (`0x40·port + 4·pin` from here).
    #[serde(default)]
    pfs_base: Option<u64>,
    #[serde(default)]
    pending: Vec<PinEdge>,
    /// `ICU_IRQn` events that passed the ISEL check, for the next walk.
    #[serde(default)]
    ready: Vec<u32>,
}

impl Default for RaIcu {
    fn default() -> Self {
        Self::new()
    }
}

impl RaIcu {
    pub fn new() -> Self {
        Self::with_irq_pins(LINES, Vec::new(), None)
    }

    /// An ICU with `lines` `IELSRn` (0: IRQ-pin half only) and the IRQ pads
    /// `irq_pins` (`(port, pin, IRQn)`), whose `PmnPFS` words start at
    /// `pfs_base`.
    pub fn with_irq_pins(lines: u32, irq_pins: Vec<(u8, u8, u8)>, pfs_base: Option<u64>) -> Self {
        Self {
            words: vec![0; WINDOW / 4],
            lines: lines.min(LINES),
            irq_pins: irq_pins
                .into_iter()
                .filter(|&(_, _, irq)| irq < IRQCR_COUNT)
                .collect(),
            pfs_base,
            pending: Vec::new(),
            ready: Vec::new(),
        }
    }

    fn ielsr_index(&self, offset: u64) -> Option<usize> {
        let rel = offset.checked_sub(IELSR_OFFSET)?;
        let n = rel / 4;
        (n < u64::from(self.lines)).then_some(n as usize)
    }

    /// `IRQCRn` byte.
    fn irqcr(&self, n: u8) -> u8 {
        (self.words[usize::from(n / 4)] >> ((n % 4) * 8)) as u8
    }

    /// Whether `IRQCRn` asks for the `before` -> `after` transition.
    fn edge_matches(&self, n: u8, before: bool, after: bool) -> bool {
        match self.irqcr(n) & IRQMD_MASK {
            0 | 3 => before && !after,
            1 => !before && after,
            _ => before != after,
        }
    }

    /// `IELSRn` value.
    pub fn ielsr(&self, n: usize) -> u32 {
        self.words[(IELSR_OFFSET / 4) as usize + n]
    }

    fn load(&self, offset: u64) -> u32 {
        self.words.get((offset / 4) as usize).copied().unwrap_or(0)
    }

    fn store_word(&mut self, offset: u64, value: u32) {
        let w = (offset / 4) as usize;
        if w >= self.words.len() {
            return;
        }
        let value = if self.ielsr_index(offset & !3).is_some() {
            // IR is write-0-to-clear; IELS and DTCE are plain fields.
            let old = self.words[w];
            (value & (IELS_MASK | DTCE)) | (old & value & IR)
        } else {
            value
        };
        self.words[w] = value;
    }

    /// Sub-word store: merge into the word, then apply the word rules.
    fn store(&mut self, offset: u64, width: u32, value: u32) {
        let shift = (offset & 3) * 8;
        let mask = if width == 4 {
            u32::MAX
        } else {
            ((1u32 << (width * 8)) - 1) << shift
        };
        let word = offset & !3;
        let old = self.load(word);
        // Lanes not written keep their bits, including IR (old & IR = IR).
        self.store_word(word, (old & !mask) | ((value << shift) & mask));
    }
}

impl Peripheral for RaIcu {
    // `needs_legacy_walk` keeps its default: the walk is only asked for while
    // an `ICU_IRQn` event waits in `ready`, which is what
    // `legacy_tick_active` answers, so an idle ICU leaves the bus walk-free.
    fn legacy_tick_active(&self) -> bool {
        !self.ready.is_empty()
    }

    fn legacy_tick_dynamic(&self) -> bool {
        !self.irq_pins.is_empty()
    }

    fn explicit_irqs_are_link_events(&self) -> bool {
        true
    }

    fn tick(&mut self) -> crate::PeripheralTickResult {
        self.tick_elapsed(1)
    }

    fn tick_elapsed(&mut self, _cycles: u64) -> crate::PeripheralTickResult {
        let events = std::mem::take(&mut self.ready);
        crate::PeripheralTickResult {
            explicit_irqs: (!events.is_empty()).then_some(events),
            ..Default::default()
        }
    }

    // Edges arrive pushed from the pad (`SystemBus::set_pad_external`), so
    // `observes_gpio_edges` stays false: the per-cycle IN-register poll it
    // would switch on serves `observe_gpio_change`, which this model does not
    // use, and it would cost the bus its walk-free fast path.
    fn gpio_input_edge(
        &mut self,
        port: u8,
        pin: u8,
        before: bool,
        after: bool,
        _line_source: Option<u8>,
    ) -> bool {
        if let Some(&(_, _, irq)) = self
            .irq_pins
            .iter()
            .find(|&&(p, n, _)| p == port && n == pin)
        {
            if self.edge_matches(irq, before, after) {
                self.pending.push(PinEdge { port, pin, irq });
            }
        }
        false
    }

    fn needs_bus_tick(&self) -> bool {
        !self.pending.is_empty()
    }

    fn tick_with_bus(&mut self, bus: &mut dyn crate::Bus) {
        let Some(pfs_base) = self.pfs_base else {
            self.pending.clear();
            return;
        };
        for edge in std::mem::take(&mut self.pending) {
            let pfs = pfs_base + 0x40 * u64::from(edge.port) + 4 * u64::from(edge.pin);
            if bus.read_u32(pfs).is_ok_and(|v| v & PFS_ISEL != 0) {
                self.ready.push(ICU_IRQ0_EVENT + u32::from(edge.irq));
            }
        }
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.load(offset & !3) >> ((offset & 3) * 8)) as u8)
    }

    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        Ok((self.load(offset & !3) >> ((offset & 2) * 8)) as u16)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.load(offset & !3))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        self.store(offset, 1, u32::from(value));
        Ok(())
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        self.store(offset & !1, 2, u32::from(value));
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.store(offset & !3, 4, value);
        Ok(())
    }

    fn irq_line_levels(&self, report: &mut dyn FnMut(u32, bool)) {
        for n in 0..self.lines as usize {
            let v = self.ielsr(n);
            if v & IELS_MASK != 0 {
                report(n as u32, v & IR != 0);
            }
        }
    }

    fn link_event(&mut self, event: u32, line: &mut dyn FnMut(u32)) -> bool {
        if self.lines == 0 {
            return false;
        }
        if event != 0 {
            let first = (IELSR_OFFSET / 4) as usize;
            for n in 0..self.lines as usize {
                let w = &mut self.words[first + n];
                if *w & IELS_MASK == event {
                    *w |= IR;
                    line(n as u32);
                }
            }
        }
        true
    }

    fn on_node_reset(&mut self) {
        *self = Self::with_irq_pins(
            self.lines,
            std::mem::take(&mut self.irq_pins),
            self.pfs_base,
        );
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        let restored: RaIcu = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!("RaIcu snapshot decode: {error}"))
        })?;
        // The wiring (line count, IRQ pads, PFS base) is the chip's, not
        // state: keep this instance's.
        self.words = restored.words;
        self.words.resize(WINDOW / 4, 0);
        self.pending = restored.pending;
        self.ready = restored.ready;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGT0_INT: u32 = 0x046;

    fn ielsr_off(n: u64) -> u64 {
        IELSR_OFFSET + 4 * n
    }

    fn linked(icu: &mut RaIcu, event: u32) -> Vec<u32> {
        let mut lines = Vec::new();
        assert!(icu.link_event(event, &mut |n| lines.push(n)));
        lines
    }

    fn levels(icu: &RaIcu) -> Vec<(u32, bool)> {
        let mut out = Vec::new();
        icu.irq_line_levels(&mut |n, l| out.push((n, l)));
        out
    }

    #[test]
    fn a_linked_event_sets_ir_and_drives_its_line() {
        let mut icu = RaIcu::new();
        icu.write_u32(ielsr_off(83), AGT0_INT).unwrap();
        assert_eq!(linked(&mut icu, AGT0_INT), vec![83]);
        assert_eq!(icu.read_u32(ielsr_off(83)).unwrap(), IR | AGT0_INT);
        assert_eq!(levels(&icu), vec![(83, true)]);
        // Another event is not linked anywhere.
        assert!(linked(&mut icu, AGT0_INT + 1).is_empty());
    }

    #[test]
    fn ir_is_write_zero_to_clear() {
        let mut icu = RaIcu::new();
        // Firmware cannot set IR itself.
        icu.write_u32(ielsr_off(5), IR | AGT0_INT).unwrap();
        assert_eq!(icu.read_u32(ielsr_off(5)).unwrap(), AGT0_INT);
        linked(&mut icu, AGT0_INT);
        // A store of 1 keeps it, the R_BSP_IrqStatusClear store of 0 clears it.
        icu.write_u32(ielsr_off(5), IR | AGT0_INT).unwrap();
        assert_eq!(icu.read_u32(ielsr_off(5)).unwrap() & IR, IR);
        icu.write_u32(ielsr_off(5), AGT0_INT).unwrap();
        assert_eq!(icu.read_u32(ielsr_off(5)).unwrap(), AGT0_INT);
        assert_eq!(levels(&icu), vec![(5, false)]);
    }

    #[test]
    fn a_byte_store_to_iels_keeps_ir() {
        let mut icu = RaIcu::new();
        icu.write_u32(ielsr_off(1), AGT0_INT).unwrap();
        linked(&mut icu, AGT0_INT);
        icu.write(ielsr_off(1), 0x47).unwrap();
        assert_eq!(icu.read_u32(ielsr_off(1)).unwrap(), IR | 0x47);
        // The byte holding IR, written with 0, clears it.
        icu.write(ielsr_off(1) + 2, 0).unwrap();
        assert_eq!(icu.read_u32(ielsr_off(1)).unwrap(), 0x47);
    }

    #[test]
    fn one_event_can_drive_several_lines_and_unlinked_lines_report_nothing() {
        let mut icu = RaIcu::new();
        icu.write_u32(ielsr_off(2), AGT0_INT).unwrap();
        icu.write_u32(ielsr_off(40), AGT0_INT | DTCE).unwrap();
        assert_eq!(linked(&mut icu, AGT0_INT), vec![2, 40]);
        assert_eq!(levels(&icu), vec![(2, true), (40, true)]);
        // Event 0 means "no event" (IELS reset value): never linked.
        assert!(linked(&mut icu, 0).is_empty());
    }

    #[test]
    fn other_words_are_storage() {
        let mut icu = RaIcu::new();
        icu.write_u32(0x1A0, 0xDEAD_BEEF).unwrap();
        assert_eq!(icu.read_u32(0x1A0).unwrap(), 0xDEAD_BEEF);
        assert_eq!(icu.read_u32(0x7FC).unwrap(), 0);
    }

    // ── IRQ-pin half ────────────────────────────────────────────────────

    /// RA8M1 P009 is IRQ13 (Zephyr `r7fa8m1xh.dtsi`).
    const PFS_BASE: u64 = 0x4040_0800;
    const P009_PFS: u64 = PFS_BASE + 9 * 4;

    /// Just enough bus for the `PmnPFS.ISEL` read.
    struct PfsBus {
        mem: std::collections::HashMap<u64, u8>,
        config: crate::SimulationConfig,
    }

    impl PfsBus {
        fn new(isel: bool) -> Self {
            let mut mem = std::collections::HashMap::new();
            let word = if isel { PFS_ISEL } else { 0 };
            for (i, b) in word.to_le_bytes().into_iter().enumerate() {
                mem.insert(P009_PFS + i as u64, b);
            }
            Self {
                mem,
                config: crate::SimulationConfig::default(),
            }
        }
    }

    impl crate::Bus for PfsBus {
        fn read_u8(&self, addr: u64) -> crate::SimResult<u8> {
            Ok(*self.mem.get(&addr).unwrap_or(&0))
        }
        fn write_u8(&mut self, addr: u64, value: u8) -> crate::SimResult<()> {
            self.mem.insert(addr, value);
            Ok(())
        }
        fn tick_peripherals(&mut self) -> Vec<u32> {
            Vec::new()
        }
        fn execute_dma(&mut self, _requests: &[crate::DmaRequest]) -> crate::SimResult<()> {
            Ok(())
        }
        fn config(&self) -> &crate::SimulationConfig {
            &self.config
        }
    }

    fn pin_icu() -> RaIcu {
        RaIcu::with_irq_pins(0, vec![(0, 9, 13), (0, 8, 12)], Some(PFS_BASE))
    }

    /// Press-and-release of P009 (high -> low -> high), then the walk.
    fn press(icu: &mut RaIcu, isel: bool) -> Vec<u32> {
        let mut bus = PfsBus::new(isel);
        let mut events = Vec::new();
        for (before, after) in [(true, false), (false, true)] {
            icu.gpio_input_edge(0, 9, before, after, None);
            if icu.needs_bus_tick() {
                icu.tick_with_bus(&mut bus);
            }
            events.extend(icu.tick_elapsed(1).explicit_irqs.unwrap_or_default());
        }
        events
    }

    const ICU_IRQ13: u32 = 0x00E;

    #[test]
    fn irqmd_selects_the_edge_that_raises_icu_irqn() {
        // IRQCR13 is byte 13 of the window.
        for (irqmd, want) in [
            (0u8, vec![ICU_IRQ13]),
            (1, vec![ICU_IRQ13]),
            (2, vec![ICU_IRQ13, ICU_IRQ13]),
            (3, vec![ICU_IRQ13]),
        ] {
            let mut icu = pin_icu();
            icu.write(13, irqmd).unwrap();
            assert_eq!(press(&mut icu, true), want, "IRQMD {irqmd}");
        }
        // Falling (0) takes the press, rising (1) does not.
        let mut icu = pin_icu();
        icu.gpio_input_edge(0, 9, true, false, None);
        assert!(icu.needs_bus_tick());
        let mut icu = pin_icu();
        icu.write(13, 1).unwrap();
        icu.gpio_input_edge(0, 9, true, false, None);
        assert!(!icu.needs_bus_tick());
    }

    #[test]
    fn a_pad_without_isel_raises_nothing() {
        let mut icu = pin_icu();
        assert!(press(&mut icu, false).is_empty());
        assert!(!icu.legacy_tick_active());
    }

    #[test]
    fn pads_that_are_not_irq_inputs_are_ignored() {
        let mut icu = pin_icu();
        icu.gpio_input_edge(0, 7, true, false, None);
        icu.gpio_input_edge(1, 9, true, false, None);
        assert!(!icu.needs_bus_tick());
    }

    #[test]
    fn the_irq_pin_half_alone_is_not_an_event_link_controller() {
        let mut icu = pin_icu();
        // IELSR offsets are plain storage when there are no lines.
        icu.write_u32(ielsr_off(0), IR | ICU_IRQ13).unwrap();
        assert_eq!(icu.read_u32(ielsr_off(0)).unwrap(), IR | ICU_IRQ13);
        let mut lines = Vec::new();
        assert!(!icu.link_event(ICU_IRQ13, &mut |n| lines.push(n)));
        assert!(lines.is_empty() && levels(&icu).is_empty());
    }

    #[test]
    fn one_window_carries_both_halves() {
        // RA6M5: IRQCRn and IELSRn in one 2 KiB block at R_ICU_BASE.
        let mut icu = RaIcu::with_irq_pins(LINES, vec![(0, 9, 13)], Some(PFS_BASE));
        icu.write_u32(ielsr_off(89), ICU_IRQ13).unwrap();
        let events = press(&mut icu, true);
        assert_eq!(events, vec![ICU_IRQ13]);
        assert_eq!(linked(&mut icu, events[0]), vec![89]);
        assert_eq!(levels(&icu), vec![(89, true)]);
    }

    #[test]
    fn reset_and_restore_keep_the_wiring() {
        let mut icu = pin_icu();
        icu.write(13, 2).unwrap();
        let snap = icu.snapshot();
        icu.on_node_reset();
        assert_eq!(icu.irqcr(13), 0);
        assert_eq!(press(&mut icu, true), vec![ICU_IRQ13]);
        icu.restore(snap).unwrap();
        assert_eq!(icu.irqcr(13), 2);
        assert_eq!(press(&mut icu, true).len(), 2);
    }
}
