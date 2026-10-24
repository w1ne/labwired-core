// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STM32 DMA request multiplexer (DMAMUX) — G0 / G4 / WB / L4+ (RM0444 §11,
//! RM0440 §13).
//!
//! On these parts a DMA channel has no fixed peripheral: DMAMUX channel `x`
//! feeds DMA channel `x + 1`, and its `DMAMUX_CxCR.DMAREQ_ID` names the
//! request that drives it. [`Peripheral::dma_route_request`] answers "which
//! DMA channels does `spi1_tx` drive right now" by scanning the `CxCR`
//! registers for the request's DMAREQ_ID (from the chip yaml's `requests:`
//! table, the datasheet's "DMAMUX: assignment of multiplexer inputs to
//! resources"), naming the DMA controller the multiplexer feeds (`dma:`).
//!
//! Register map (RM0444 §11.6, G071 SVD): `C0CR..` at `4·x`, `CSR` 0x80
//! (read-only), `CFR` 0x84 (write-only), `RG0CR..RG3CR` 0x100, `RGSR` 0x140
//! (read-only), `RGCFR` 0x144 (write-only), and the identification block at
//! the top of the window (`HWCFGR2` 0x3EC .. `SIDR` 0x3FC, constants from the
//! chip yaml's `id_registers`).
//!
//! ## Request generators (`RGxCR`)
//!
//! Generator `g` is request input `DMAREQ_ID = g + 1` (RM0444 Table 59,
//! `dmamux_req_gen0..3`). With `GE` set, an edge of its trigger input
//! (`SIG_ID`) of the polarity `GPOL` selects (01 rising, 10 falling, 11
//! both) makes it issue `GNBREQ + 1` DMA requests, one per cycle, to the
//! channels whose `DMAREQ_ID` selects it. A trigger edge that comes while
//! the previous ones are still being issued sets `RGSR.OFx` (and the
//! interrupt with `OIE`) and is not counted.
//!
//! ## Synchronisation (`CxCR.SE`)
//!
//! With `SE` set, a channel forwards requests only after an edge (`SPOL`)
//! of its synchronisation input (`SYNC_ID`), and then `NBREQ + 1` of them;
//! later requests are held (not lost) until the next synchronisation edge.
//! A synchronisation edge that comes before the previous `NBREQ + 1`
//! requests went through sets `CSR.SOFx` (and the interrupt with `SOIE`).
//! With `EGE`, every `NBREQ + 1` forwarded requests (with or without `SE`)
//! the channel emits an event pulse, `dmamux_evtx` for channels 0..3.
//!
//! ## Trigger / synchronisation inputs (G071, RM0444 Tables 60 and 61)
//!
//! | Input  | Source             | Modelled                                    |
//! |-------:|--------------------|---------------------------------------------|
//! | 0..15  | EXTI line 0..15    | yes: the pad of the port `EXTI_EXTICRx` selects for the line |
//! | 16..19 | `dmamux_evt0..3`   | yes: channel 0..3 event pulses              |
//! | 20     | LPTIM1_OUT         | no (no output signal to source it from)     |
//! | 21     | LPTIM2_OUT         | no                                          |
//! | 22     | TIM14_OC           | no                                          |
//!
//! `HWCFGR2` / `HWCFGR1` (23 inputs, 4 generators, 7 channels) agree.

use crate::{DmaRoute, Peripheral, PeripheralTickResult, SimResult};

/// `DMAMUX_CxCR` writable bits (G071 SVD): DMAREQ_ID[7:0], SOIE[8], EGE[9],
/// SE[16], SPOL[18:17], NBREQ[23:19], SYNC_ID[28:24].
const CXCR_MASK: u32 = 0x1FFF_03FF;
/// `DMAREQ_ID` field.
const DMAREQ_ID_MASK: u32 = 0xFF;
const CXCR_SOIE: u32 = 1 << 8;
const CXCR_EGE: u32 = 1 << 9;
const CXCR_SE: u32 = 1 << 16;
const CXCR_SPOL_SHIFT: u32 = 17;
const CXCR_NBREQ_SHIFT: u32 = 19;
const CXCR_SYNC_ID_SHIFT: u32 = 24;
/// `DMAMUX_RGxCR` writable bits: SIG_ID[4:0], OIE[8], GE[16], GPOL[18:17],
/// GNBREQ[23:19].
const RGXCR_MASK: u32 = 0x00FF_011F;
const RGXCR_OIE: u32 = 1 << 8;
const RGXCR_GE: u32 = 1 << 16;
const RGXCR_GPOL_SHIFT: u32 = 17;
const RGXCR_GNBREQ_SHIFT: u32 = 19;
/// Request generators on every DMAMUX this models.
const GENERATORS: usize = 4;
const CSR: u64 = 0x80;
const CFR: u64 = 0x84;
const RG_BASE: u64 = 0x100;
const RGSR: u64 = 0x140;
const RGCFR: u64 = 0x144;
/// Trigger / synchronisation input of EXTI line 0 (G0: lines 0..15 are
/// inputs 0..15).
const INPUT_EXTI0: u32 = 0;
/// Input of `dmamux_evt0` (G0: channel 0..3 events are inputs 16..19).
const INPUT_EVT0: u32 = 16;
/// Channels with an event output (`dmamux_evt0..3`).
const EVENT_CHANNELS: usize = 4;
/// `dma_signals` id of "forward one request on multiplexer channel x":
/// `OUT_SIGNAL_BASE + x` ([`Peripheral::dma_mux_output`]).
const OUT_SIGNAL_BASE: u32 = 0x100;
/// The scheduler token of the multiplexer's output event.
const OUTPUT_TOKEN: u32 = 0;

/// Does an edge (`rising` or falling) match a 2-bit polarity selector
/// (00 none, 01 rising, 10 falling, 11 both)?
fn polarity_matches(pol: u32, rising: bool) -> bool {
    match pol & 3 {
        0b01 => rising,
        0b10 => !rising,
        0b11 => true,
        _ => false,
    }
}

#[derive(Debug, Default, serde::Serialize)]
pub struct Dmamux {
    cxcr: Vec<u32>,
    rgxcr: [u32; GENERATORS],
    /// `CSR.SOFx` synchronisation overrun flags.
    csr: u32,
    /// `RGSR.OFx` generator overrun flags.
    rgsr: u32,
    /// Per channel: requests still allowed through since the last
    /// synchronisation edge (`SE` = 1).
    sync_window: Vec<u32>,
    /// Per channel: requests held while the synchronisation gate is shut.
    held: Vec<u32>,
    /// Per channel: forwarded requests toward the next `EGE` event.
    event_count: Vec<u32>,
    /// Per generator: requests still to issue.
    gen_left: [u32; GENERATORS],
    /// Requests to forward, by multiplexer channel, at the next cycle.
    out: Vec<u32>,
    /// An overrun interrupt to pend at the next cycle.
    irq_pending: bool,
    /// Request name (`spi1_tx`) → DMAREQ_ID.
    #[serde(skip)]
    requests: Vec<(String, u32)>,
    /// The DMA controller (peripheral id) DMAMUX channel `x` feeds as its
    /// channel `x + 1`.
    #[serde(skip)]
    dma: String,
    /// Read-only identification registers `(offset, value)`.
    #[serde(skip)]
    id_registers: Vec<(u64, u32)>,
    #[serde(skip)]
    clock: Option<crate::CycleClock>,
    /// Scheduler mode only: the output event is live in the heap.
    #[serde(skip)]
    output_live: bool,
}

impl Dmamux {
    /// A multiplexer with `channels` request lines feeding the DMA
    /// controller `dma`.
    pub fn new(channels: usize, dma: &str) -> Self {
        Self {
            cxcr: vec![0; channels],
            sync_window: vec![0; channels],
            held: vec![0; channels],
            event_count: vec![0; channels],
            dma: dma.to_string(),
            ..Default::default()
        }
    }

    /// Name the DMAREQ_ID of a peripheral request (`spi1_tx` → 17 on G0).
    pub fn with_request(mut self, request: &str, id: u32) -> Self {
        self.requests.push((request.to_string(), id));
        self
    }

    /// A read-only identification register (HWCFGRx / VERR / IPIDR / SIDR).
    pub fn with_id_register(mut self, offset: u64, value: u32) -> Self {
        self.id_registers.push((offset, value));
        self
    }

    crate::cycle_clock::scheduler_mode!();

    fn read_reg(&self, offset: u64) -> u32 {
        let word = (offset / 4) as usize;
        if word < self.cxcr.len() {
            return self.cxcr[word];
        }
        if (RG_BASE..RG_BASE + 4 * GENERATORS as u64).contains(&offset) {
            return self.rgxcr[((offset - RG_BASE) / 4) as usize];
        }
        match offset {
            CSR => self.csr,
            RGSR => self.rgsr,
            // The clear-flag registers are write-only.
            CFR | RGCFR => 0,
            _ => self
                .id_registers
                .iter()
                .find(|(o, _)| *o == offset)
                .map_or(0, |(_, v)| *v),
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        let word = (offset / 4) as usize;
        if word < self.cxcr.len() {
            let old = self.cxcr[word];
            self.cxcr[word] = value & CXCR_MASK;
            if old & CXCR_SE != 0 && value & CXCR_SE == 0 {
                // Synchronisation off: what was held goes through.
                self.sync_window[word] = 0;
                for _ in 0..std::mem::take(&mut self.held[word]) {
                    self.forward(word);
                }
            } else if old & CXCR_SE == 0 && value & CXCR_SE != 0 {
                self.sync_window[word] = 0;
            }
        } else if (RG_BASE..RG_BASE + 4 * GENERATORS as u64).contains(&offset) {
            let g = ((offset - RG_BASE) / 4) as usize;
            self.rgxcr[g] = value & RGXCR_MASK;
            if value & RGXCR_GE == 0 {
                self.gen_left[g] = 0;
            }
        } else if offset == CFR {
            self.csr &= !value;
        } else if offset == RGCFR {
            self.rgsr &= !value;
        }
    }

    fn nbreq(cr: u32) -> u32 {
        ((cr >> CXCR_NBREQ_SHIFT) & 0x1F) + 1
    }

    /// A request reaches multiplexer channel `x`: forwarded now (`true`), or
    /// held behind the synchronisation gate.
    fn channel_request(&mut self, x: usize) -> bool {
        if self.cxcr[x] & CXCR_SE != 0 {
            if self.sync_window[x] == 0 {
                self.held[x] += 1;
                return false;
            }
            self.sync_window[x] -= 1;
        }
        self.count_event(x);
        true
    }

    /// Queue one request on channel `x` for the next cycle.
    fn forward(&mut self, x: usize) {
        self.count_event(x);
        self.out.push(x as u32);
    }

    /// One more request forwarded on channel `x`: every `NBREQ + 1` of them,
    /// `EGE` raises the channel's event.
    fn count_event(&mut self, x: usize) {
        let cr = self.cxcr[x];
        self.event_count[x] += 1;
        if self.event_count[x] >= Self::nbreq(cr) {
            self.event_count[x] = 0;
            if cr & CXCR_EGE != 0 && x < EVENT_CHANNELS {
                // A pulse: a rising edge, then a falling one.
                self.input_edge(INPUT_EVT0 + x as u32, true);
                self.input_edge(INPUT_EVT0 + x as u32, false);
            }
        }
    }

    /// An edge on trigger / synchronisation input `input`.
    fn input_edge(&mut self, input: u32, rising: bool) {
        for g in 0..GENERATORS {
            let cr = self.rgxcr[g];
            if cr & RGXCR_GE == 0
                || cr & 0x1F != input
                || !polarity_matches(cr >> RGXCR_GPOL_SHIFT, rising)
            {
                continue;
            }
            if self.gen_left[g] > 0 {
                self.rgsr |= 1 << g;
                self.irq_pending |= cr & RGXCR_OIE != 0;
            } else {
                self.gen_left[g] = ((cr >> RGXCR_GNBREQ_SHIFT) & 0x1F) + 1;
            }
        }
        for x in 0..self.cxcr.len() {
            let cr = self.cxcr[x];
            if cr & CXCR_SE == 0
                || (cr >> CXCR_SYNC_ID_SHIFT) & 0x1F != input
                || !polarity_matches(cr >> CXCR_SPOL_SHIFT, rising)
            {
                continue;
            }
            if self.sync_window[x] > 0 {
                self.csr |= 1 << x;
                self.irq_pending |= cr & CXCR_SOIE != 0;
            }
            self.sync_window[x] = Self::nbreq(cr);
            while self.held[x] > 0 && self.sync_window[x] > 0 {
                self.held[x] -= 1;
                self.sync_window[x] -= 1;
                self.forward(x);
            }
        }
    }

    /// Each generator with requests left issues one to the channels that
    /// select it. A generator no channel selects is never served and keeps
    /// its count (the next trigger overruns).
    fn run_generators(&mut self) {
        for g in 0..GENERATORS {
            if self.gen_left[g] == 0 || self.rgxcr[g] & RGXCR_GE == 0 {
                continue;
            }
            let id = g as u32 + 1;
            let mut taken = false;
            for x in 0..self.cxcr.len() {
                if self.cxcr[x] & DMAREQ_ID_MASK == id {
                    taken = true;
                    if self.channel_request(x) {
                        self.out.push(x as u32);
                    }
                }
            }
            if taken {
                self.gen_left[g] -= 1;
            }
        }
    }

    fn has_work(&self) -> bool {
        !self.out.is_empty()
            || self.irq_pending
            || (0..GENERATORS).any(|g| self.gen_left[g] > 0 && self.rgxcr[g] & RGXCR_GE != 0)
    }

    /// One cycle: generators issue, and the queued requests go out.
    fn step(&mut self) -> PeripheralTickResult {
        self.run_generators();
        let signals: Vec<u32> = self.out.drain(..).map(|x| OUT_SIGNAL_BASE + x).collect();
        PeripheralTickResult {
            irq: std::mem::take(&mut self.irq_pending),
            dma_signals: (!signals.is_empty()).then_some(signals),
            ..Default::default()
        }
    }
}

impl Peripheral for Dmamux {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let v = self.read_reg(offset & !3);
        Ok((v >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let shift = (offset & 3) * 8;
        let v = if reg == CFR || reg == RGCFR {
            u32::from(value) << shift // write-1-to-clear: the byte alone
        } else {
            (self.read_reg(reg) & !(0xFF << shift)) | (u32::from(value) << shift)
        };
        self.write_reg(reg, v);
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }

    fn dma_route_request(&self, request: &str, out: &mut Vec<DmaRoute>) {
        let Some(&(_, id)) = self.requests.iter().find(|(name, _)| name == request) else {
            return;
        };
        for (x, cr) in self.cxcr.iter().enumerate() {
            // DMAREQ_ID 0 is "no request" (RM0444 Table 59), so a channel left
            // at reset never routes.
            if id != 0 && cr & DMAREQ_ID_MASK == id {
                out.push(DmaRoute {
                    controller: Some(self.dma.clone()),
                    channel: x as u32 + 1,
                });
            }
        }
    }

    fn dma_mux_forward(&mut self, channel: u32) -> bool {
        let x = channel.saturating_sub(1) as usize;
        if x >= self.cxcr.len() {
            return true;
        }
        self.channel_request(x)
    }

    fn dma_mux_output(&self, signal: u32) -> Option<DmaRoute> {
        let x = signal.checked_sub(OUT_SIGNAL_BASE)?;
        ((x as usize) < self.cxcr.len()).then(|| DmaRoute {
            controller: Some(self.dma.clone()),
            channel: x + 1,
        })
    }

    /// EXTI lines 0..15 are inputs 0..15: the pad edge counts when the
    /// line's source mux selects its port.
    fn gpio_input_edge(
        &mut self,
        port: u8,
        pin: u8,
        before: bool,
        after: bool,
        line_source: Option<u8>,
    ) -> bool {
        if pin >= 16 || before == after || line_source != Some(port) {
            return false;
        }
        self.input_edge(INPUT_EXTI0 + u32::from(pin), after);
        self.has_work()
    }

    fn tick(&mut self) -> PeripheralTickResult {
        if self.scheduler_mode() {
            return PeripheralTickResult::default();
        }
        self.step()
    }

    fn tick_elapsed_forced(&mut self, _cycles: u64) -> PeripheralTickResult {
        self.step()
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    fn needs_legacy_walk(&self) -> bool {
        !self.scheduler_mode()
    }

    fn attach_cycle_clock(&mut self, clock: crate::CycleClock) {
        self.clock = Some(clock);
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if !self.scheduler_mode() || self.output_live || !self.has_work() {
            return Vec::new();
        }
        self.output_live = true;
        vec![(0, OUTPUT_TOKEN)]
    }

    fn on_event(
        &mut self,
        event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if !self.scheduler_mode() || event_token != OUTPUT_TOKEN {
            return crate::sched::EventResult::default();
        }
        let res = self.step();
        let again = self.has_work();
        self.output_live = again;
        crate::sched::EventResult {
            raise_own_irq: res.irq,
            dma_signals: res.dma_signals.unwrap_or_default(),
            reschedule_delay: again.then_some(1),
            ..Default::default()
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g0() -> Dmamux {
        Dmamux::new(7, "dma1")
            .with_request("spi1_rx", 16)
            .with_request("spi1_tx", 17)
            .with_id_register(0x3FC, 0xA3C5_DD01)
    }

    fn routes(m: &Dmamux, request: &str) -> Vec<DmaRoute> {
        let mut out = Vec::new();
        m.dma_route_request(request, &mut out);
        out
    }

    /// The multiplexer channels each cycle forwards a request on.
    fn outputs(m: &mut Dmamux) -> Vec<u32> {
        m.tick()
            .dma_signals
            .unwrap_or_default()
            .iter()
            .map(|&s| m.dma_mux_output(s).unwrap().channel - 1)
            .collect()
    }

    /// A rising then a falling edge on EXTI line `line` of port A.
    fn pulse(m: &mut Dmamux, line: u8) -> bool {
        let a = m.gpio_input_edge(0, line, false, true, Some(0));
        let b = m.gpio_input_edge(0, line, true, false, Some(0));
        a || b
    }

    #[test]
    fn a_request_drives_the_dma_channel_whose_cxcr_selects_it() {
        let mut m = g0();
        assert!(
            routes(&m, "spi1_tx").is_empty(),
            "nothing selected at reset"
        );
        m.write_u32(0x04, 17).unwrap(); // C1CR: SPI1_TX → DMA channel 2
        m.write_u32(0x00, 16).unwrap(); // C0CR: SPI1_RX → DMA channel 1
        assert_eq!(
            routes(&m, "spi1_tx"),
            vec![DmaRoute {
                controller: Some("dma1".into()),
                channel: 2
            }]
        );
        assert_eq!(routes(&m, "spi1_rx")[0].channel, 1);
        assert!(routes(&m, "spi2_tx").is_empty(), "an unknown request");
        assert!(m.dma_mux_forward(2), "no synchronisation: straight through");
        m.write_u32(0x04, 0).unwrap();
        assert!(routes(&m, "spi1_tx").is_empty());
    }

    #[test]
    fn registers_store_under_their_masks_and_id_registers_are_constant() {
        let mut m = g0();
        m.write_u32(0x00, 0xFFFF_FFFF).unwrap();
        assert_eq!(m.read_reg(0x00), CXCR_MASK);
        m.write_u32(0x100, 0xFFFF_FFFF).unwrap();
        assert_eq!(m.read_reg(0x100), RGXCR_MASK);
        m.write_u32(0x84, 0xFFFF_FFFF).unwrap();
        assert_eq!(m.read_reg(0x84), 0, "CFR is write-only");
        m.write_u32(0x3FC, 0).unwrap();
        assert_eq!(m.read_reg(0x3FC), 0xA3C5_DD01);
        // A byte write merges into the word.
        m.write(0x01, 0x02).unwrap();
        assert_eq!(m.read_reg(0x00), (CXCR_MASK & !0xFF00) | 0x0200);
    }

    /// Request generator 0 on EXTI line 3, rising edge, GNBREQ = 3: one edge
    /// issues 4 requests, one per cycle, on the channel selecting
    /// dmamux_req_gen0 (DMAREQ_ID 1); a falling edge does nothing.
    #[test]
    fn a_generator_issues_gnbreq_plus_one_requests_on_an_exti_edge() {
        let mut m = g0();
        m.write_u32(0x08, 1).unwrap(); // C2CR: DMAREQ_ID 1 → DMA channel 3
        m.write_u32(
            0x100,
            3 | RGXCR_GE | (0b01 << RGXCR_GPOL_SHIFT) | (3 << RGXCR_GNBREQ_SHIFT),
        )
        .unwrap();
        assert!(outputs(&mut m).is_empty());
        assert!(!m.gpio_input_edge(0, 3, true, false, Some(0)), "falling");
        assert!(!m.gpio_input_edge(1, 3, false, true, Some(0)), "port B");
        assert!(m.gpio_input_edge(0, 3, false, true, Some(0)), "rising");
        let mut got = Vec::new();
        for _ in 0..6 {
            got.push(outputs(&mut m));
        }
        assert_eq!(
            got,
            vec![vec![2], vec![2], vec![2], vec![2], vec![], vec![]]
        );
        assert_eq!(m.read_reg(RGSR), 0);
    }

    /// A trigger while the generator is still issuing: RGSR.OF0 (and the
    /// interrupt with OIE); RGCFR clears it.
    #[test]
    fn a_trigger_during_generation_overruns() {
        let mut m = g0();
        m.write_u32(0x00, 1).unwrap();
        m.write_u32(
            0x100,
            5 | RGXCR_GE | RGXCR_OIE | (0b11 << RGXCR_GPOL_SHIFT) | (7 << RGXCR_GNBREQ_SHIFT),
        )
        .unwrap();
        pulse(&mut m, 5); // rising starts it, falling overruns
        assert_eq!(m.read_reg(RGSR), 1);
        assert!(m.tick().irq, "OIE");
        m.write_u32(RGCFR, 1).unwrap();
        assert_eq!(m.read_reg(RGSR), 0);
    }

    /// Synchronisation: requests are held until an edge of SYNC_ID (EXTI
    /// line 0, rising), then NBREQ + 1 = 2 of them go through; the rest wait
    /// for the next edge. A sync edge inside an unfinished window sets SOF.
    #[test]
    fn synchronisation_gates_requests_to_nbreq_plus_one_per_edge() {
        let mut m = g0();
        let cr = 17 | CXCR_SE | CXCR_SOIE | (0b01 << CXCR_SPOL_SHIFT) | (1 << CXCR_NBREQ_SHIFT);
        m.write_u32(0x04, cr).unwrap(); // C1CR: SPI1_TX, sync on input 0
        for _ in 0..3 {
            assert!(!m.dma_mux_forward(2), "held before any sync edge");
        }
        assert!(outputs(&mut m).is_empty());
        assert!(pulse(&mut m, 0));
        assert_eq!(outputs(&mut m), vec![1, 1], "two released");
        assert!(!m.dma_mux_forward(2), "window used up");
        assert!(pulse(&mut m, 0));
        assert_eq!(outputs(&mut m), vec![1, 1], "the third and the fourth");
        assert_eq!(m.read_reg(CSR), 0);
        // One more request, one more edge: it goes through and leaves a
        // place in the window — the next edge overruns it.
        assert!(!m.dma_mux_forward(2));
        pulse(&mut m, 0);
        assert_eq!(outputs(&mut m), vec![1]);
        pulse(&mut m, 0);
        assert_eq!(m.read_reg(CSR), 1 << 1, "SOF1");
        assert!(m.tick().irq, "SOIE");
        m.write_u32(CFR, 1 << 1).unwrap();
        assert_eq!(m.read_reg(CSR), 0);
        // SE off: what is held goes through, and requests pass straight.
        m.write_u32(0x04, 17).unwrap();
        assert!(m.dma_mux_forward(2));
    }

    /// EGE: every NBREQ + 1 forwarded requests channel 0 pulses
    /// dmamux_evt0 (input 16), which here triggers request generator 1.
    #[test]
    fn a_channel_event_triggers_a_generator() {
        let mut m = g0();
        m.write_u32(0x00, 16 | CXCR_EGE | (2 << CXCR_NBREQ_SHIFT))
            .unwrap(); // SPI1_RX
        m.write_u32(0x0C, 2).unwrap(); // C3CR: dmamux_req_gen1
        m.write_u32(0x104, INPUT_EVT0 | RGXCR_GE | (0b01 << RGXCR_GPOL_SHIFT))
            .unwrap();
        assert!(m.dma_mux_forward(1));
        assert!(m.dma_mux_forward(1));
        assert!(outputs(&mut m).is_empty(), "two of three");
        assert!(m.dma_mux_forward(1));
        assert_eq!(outputs(&mut m), vec![3], "the event fired generator 1");
        assert!(outputs(&mut m).is_empty());
    }
}
