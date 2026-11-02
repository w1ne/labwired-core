//! Cadence Triple Timer Counter (Xilinx `xlnx,ttcps`), as on the Zynq
//! UltraScale+ MPSoC: three 32-bit up-counters with prescaler, interval and
//! match modes (UG1087 TTC register reference). Counts derive from the bus
//! cycle and CPU Hz with an absolute floor, so no host clock or per-cycle walk
//! exists and elapsed spans of any length (idle fast-forward) resolve in closed
//! form. Match, interval and overflow events latch in the clear-on-read ISR;
//! each counter drives its own GIC SPI level as `ISR & IER`.
//! Not modelled: decrement mode and external clock source (both refused),
//! waveform output, and the event timer (its registers store and read zero).
use super::gic::Gic;
use crate::cycle_clock::CycleClock;
use crate::{Peripheral, SimResult, SimulationError};
use std::sync::{Arc, Mutex};

const CNT_DIS: u32 = 1 << 0;
const CNT_INT: u32 = 1 << 1;
const CNT_DEC: u32 = 1 << 2;
const CNT_MATCH: u32 = 1 << 3;
const CNT_RST: u32 = 1 << 4;
const IXR_INTERVAL: u32 = 1 << 0;
const IXR_OVERFLOW: u32 = 1 << 4;
const WRAP: u64 = 1 << 32;

#[derive(Debug, Clone)]
struct Counter {
    clk: u32,
    cnt: u32,
    interval: u32,
    matches: [u32; 3],
    isr: u32,
    ier: u32,
    event_ctl: u32,
    value: u32,
    /// Cycle up to which `value` and `isr` are resolved.
    anchor: u64,
}
impl Default for Counter {
    fn default() -> Self {
        Self {
            clk: 0,
            cnt: 0x21,
            interval: 0,
            matches: [0; 3],
            isr: 0,
            ier: 0,
            event_ctl: 0,
            value: 0,
            anchor: 0,
        }
    }
}
impl Counter {
    fn prescale(&self) -> u64 {
        if self.clk & 1 != 0 {
            1 << (((self.clk >> 1) & 0xf) + 1)
        } else {
            1
        }
    }
    fn running(&self) -> bool {
        self.cnt & CNT_DIS == 0
    }
    /// Counter period: interval mode counts 0..=INTERVAL, otherwise 2^32.
    fn period(&self) -> u64 {
        if self.cnt & CNT_INT != 0 {
            u64::from(self.interval) + 1
        } else {
            WRAP
        }
    }
    /// Ticks from `value` until the counter next equals `target` (1..=period).
    fn ticks_to(&self, target: u64) -> Option<u64> {
        let p = self.period();
        if target >= p {
            return None;
        }
        let t = (target + p - u64::from(self.value) % p) % p;
        Some(if t == 0 { p } else { t })
    }
    /// Ticks until each IXR source next fires, with its ISR bit.
    fn next_events(&self) -> impl Iterator<Item = (u32, u64)> + '_ {
        let matches = (self.cnt & CNT_MATCH != 0)
            .then(|| {
                self.matches
                    .iter()
                    .enumerate()
                    .filter_map(|(k, &m)| Some((1 << (k + 1), self.ticks_to(u64::from(m))?)))
            })
            .into_iter()
            .flatten();
        // Interval: the INTERVAL -> 0 transition. Overflow: 0xFFFFFFFF -> 0.
        let wrap = self.period() - u64::from(self.value) % self.period();
        let wrap_bit = if self.cnt & CNT_INT != 0 {
            IXR_INTERVAL
        } else {
            IXR_OVERFLOW
        };
        matches.chain(std::iter::once((wrap_bit, wrap)))
    }
    fn advance_ticks(&mut self, d: u64) {
        if d == 0 {
            return;
        }
        let mut fired = 0;
        for (bit, t) in self.next_events() {
            if t <= d {
                fired |= bit;
            }
        }
        self.isr |= fired;
        let p = self.period();
        self.value = ((u64::from(self.value) % p + d % p) % p) as u32;
    }
}

#[derive(Debug)]
pub struct Ttc {
    counters: [Counter; 3],
    gic: Arc<Mutex<Gic>>,
    intids: [usize; 3],
    clock: CycleClock,
    cpu_hz: u64,
    input_hz: u64,
}
impl Ttc {
    /// `input_hz` is the counter input clock before the prescaler (the board
    /// DT `clock-frequency`); `intids` are the three counters' GIC INTIDs.
    pub fn new(
        gic: Arc<Mutex<Gic>>,
        intids: [usize; 3],
        clock: CycleClock,
        cpu_hz: u64,
        input_hz: u64,
    ) -> SimResult<Self> {
        if cpu_hz == 0 || input_hz == 0 {
            return Err(SimulationError::Other(
                "TTC requires nonzero CPU and input clocks".into(),
            ));
        }
        let mut t = Self {
            counters: Default::default(),
            gic,
            intids,
            clock,
            cpu_hz,
            input_hz,
        };
        let now = t.clock.now();
        for c in &mut t.counters {
            c.anchor = now;
        }
        Ok(t)
    }
    /// Absolute counter ticks at `cycle` for a prescale.
    fn ticks_at(&self, cycle: u64, prescale: u64) -> u128 {
        u128::from(cycle) * u128::from(self.input_hz)
            / (u128::from(self.cpu_hz) * u128::from(prescale))
    }
    /// First cycle whose absolute tick count reaches `ticks`.
    fn cycle_of(&self, ticks: u128, prescale: u64) -> Option<u64> {
        let num = ticks * u128::from(self.cpu_hz) * u128::from(prescale);
        u64::try_from(num.div_ceil(u128::from(self.input_hz))).ok()
    }
    fn resolve(&mut self, n: usize, now: u64) {
        let (anchor, ps, running) = {
            let c = &self.counters[n];
            (c.anchor, c.prescale(), c.running())
        };
        if now > anchor && running {
            let d = self.ticks_at(now, ps) - self.ticks_at(anchor, ps);
            self.counters[n].advance_ticks(u64::try_from(d).unwrap_or(u64::MAX));
        }
        self.counters[n].anchor = now.max(anchor);
    }
    fn sync_levels(&self) {
        let mut g = self.gic.lock().unwrap();
        for (c, &id) in self.counters.iter().zip(&self.intids) {
            g.set_level(id, c.isr & c.ier != 0);
        }
    }
    /// Resolve all counters to `cycle` and publish their GIC levels.
    pub fn synchronize(&mut self, cycle: u64) {
        for n in 0..3 {
            self.resolve(n, cycle);
        }
        self.sync_levels();
    }
    /// Earliest future cycle at which an enabled counter event fires.
    pub fn next_deadline(&self) -> Option<u64> {
        self.counters
            .iter()
            .filter(|c| c.running())
            .flat_map(|c| {
                let ps = c.prescale();
                let base = self.ticks_at(c.anchor, ps);
                c.next_events()
                    .filter(|(bit, _)| c.ier & bit != 0)
                    .filter_map(move |(_, t)| self.cycle_of(base + u128::from(t), ps))
                    .collect::<Vec<_>>()
            })
            .min()
    }
    fn locate(o: u64) -> SimResult<(u64, usize)> {
        if o % 4 != 0 || o >= 0x84 {
            return Err(SimulationError::NotImplemented(format!(
                "TTC offset {o:#x}"
            )));
        }
        let reg = o / 12 * 12;
        Ok((reg, ((o - reg) / 4) as usize))
    }
    pub fn read(&mut self, o: u64) -> SimResult<u32> {
        let (reg, n) = Self::locate(o)?;
        let now = self.clock.now();
        self.resolve(n, now);
        let c = &mut self.counters[n];
        let v = match reg {
            0x00 => c.clk,
            0x0c => c.cnt,
            0x18 => c.value,
            0x24 => c.interval,
            0x30 => c.matches[0],
            0x3c => c.matches[1],
            0x48 => c.matches[2],
            0x54 => std::mem::take(&mut c.isr),
            0x60 => c.ier,
            0x6c => c.event_ctl,
            _ => 0,
        };
        self.sync_levels();
        Ok(v)
    }
    pub fn write(&mut self, o: u64, v: u32) -> SimResult<()> {
        let (reg, n) = Self::locate(o)?;
        let now = self.clock.now();
        self.resolve(n, now);
        let c = &mut self.counters[n];
        match reg {
            0x00 => {
                if v & (1 << 5) != 0 {
                    return Err(SimulationError::NotImplemented(
                        "TTC external clock source".into(),
                    ));
                }
                c.clk = v & 0x7f;
            }
            0x0c => {
                if v & CNT_DEC != 0 {
                    return Err(SimulationError::NotImplemented("TTC decrement mode".into()));
                }
                if v & CNT_RST != 0 {
                    c.value = 0;
                }
                c.cnt = v & 0x6f;
            }
            0x24 => c.interval = v,
            0x30 => c.matches[0] = v,
            0x3c => c.matches[1] = v,
            0x48 => c.matches[2] = v,
            0x60 => c.ier = v & 0x3f,
            0x6c => c.event_ctl = v & 7,
            // Counter value, ISR (clear-on-read) and event count are read-only.
            _ => {}
        }
        self.sync_levels();
        Ok(())
    }
}

/// MMIO window sharing the fabric-owned TTC state.
#[derive(Debug)]
pub struct TtcWindow(pub Arc<Mutex<Ttc>>);
impl Peripheral for TtcWindow {
    fn read(&self, o: u64) -> SimResult<u8> {
        Err(SimulationError::NotImplemented(format!(
            "TTC byte read {o:#x}"
        )))
    }
    fn write(&mut self, o: u64, _: u8) -> SimResult<()> {
        Err(SimulationError::NotImplemented(format!(
            "TTC byte write {o:#x}"
        )))
    }
    fn read_u32(&self, o: u64) -> SimResult<u32> {
        self.0.lock().unwrap().read(o)
    }
    fn write_u32(&mut self, o: u64, v: u32) -> SimResult<()> {
        self.0.lock().unwrap().write(o, v)
    }
    // Counts are closed-form from the bus cycle; the fabric synchronises them
    // and publishes the next match as a CPU interrupt deadline.
    fn needs_legacy_walk(&self) -> bool {
        false
    }
    fn peek(&self, o: u64) -> Option<u8> {
        let t = self.0.lock().unwrap();
        let (reg, n) = Ttc::locate(o & !3).ok()?;
        let c = &t.counters[n];
        let word = match reg {
            0x00 => c.clk,
            0x0c => c.cnt,
            0x18 => c.value,
            0x24 => c.interval,
            0x30 => c.matches[0],
            0x3c => c.matches[1],
            0x48 => c.matches[2],
            0x54 => c.isr,
            0x60 => c.ier,
            0x6c => c.event_ctl,
            _ => 0,
        };
        Some((word >> ((o & 3) * 8)) as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn level(gic: &Arc<Mutex<Gic>>, id: usize) -> bool {
        let word = gic
            .lock()
            .unwrap()
            .dist_read(0x200 + (id / 32 * 4) as u64, 4)
            .unwrap();
        word >> (id % 32) & 1 != 0
    }
    // 500 MHz CPU, 5 MHz TTC input: one counter tick per 100 cycles.
    fn ttc() -> (Ttc, CycleClock, Arc<Mutex<Gic>>) {
        let gic = Arc::new(Mutex::new(Gic::default()));
        let clock = CycleClock::default();
        let t = Ttc::new(
            gic.clone(),
            [68, 69, 70],
            clock.clone(),
            500_000_000,
            5_000_000,
        )
        .unwrap();
        (t, clock, gic)
    }
    #[test]
    fn zephyr_match_mode_sequence() {
        let (mut t, clock, _gic) = ttc();
        // sys_clock_driver_init: stop, reset value, clocks, match 0, IER.
        t.write(0x0c, CNT_DIS).unwrap();
        t.write(0x0c, 0x21).unwrap();
        t.write(0x00, 0).unwrap();
        t.write(0x54, 0x1f).unwrap();
        let r = t.read(0x0c).unwrap();
        t.write(0x0c, r | CNT_RST).unwrap();
        let r = t.read(0x0c).unwrap();
        assert_eq!(r & CNT_RST, 0, "RST self-clears");
        t.write(0x0c, r | CNT_MATCH).unwrap();
        t.write(0x30, 5000).unwrap();
        t.write(0x60, 2).unwrap();
        clock.publish(1000);
        let r = t.read(0x0c).unwrap();
        t.write(0x0c, r & !CNT_DIS).unwrap();
        // Started at cycle 1000; match 5000 ticks later = cycle 501000.
        assert_eq!(t.next_deadline(), Some(501_000));
        t.synchronize(500_999);
        assert_eq!(t.read(0x18).unwrap(), 4999);
        assert_eq!(t.counters[0].isr, 0);
        t.synchronize(501_000);
        assert!(level(&t.gic, 68));
        clock.publish(501_000);
        assert_eq!(t.read(0x54).unwrap(), 2, "match 0 latched");
        assert_eq!(t.read(0x54).unwrap(), 0, "ISR clears on read");
        assert!(!level(&t.gic, 68));
    }
    #[test]
    fn interval_overflow_and_prescale_resolve_in_closed_form() {
        let (mut t, clock, _) = ttc();
        // Counter 1: interval 99 (period 100 ticks), prescale 2^(0+1) = 2.
        t.write(0x04, 1).unwrap();
        t.write(0x28, 99).unwrap();
        t.write(0x64, IXR_INTERVAL).unwrap();
        t.write(0x10, CNT_INT).unwrap();
        // 200 cycles per tick; interval wrap after 100 ticks.
        assert_eq!(t.next_deadline(), Some(20_000));
        clock.publish(10_000_000);
        assert_eq!(t.read(0x1c).unwrap(), 0, "50000 ticks = 500 whole periods");
        assert_eq!(t.read(0x58).unwrap(), IXR_INTERVAL);
        // Counter 2 free-running with no IER: no deadline contribution.
        t.write(0x14, 0).unwrap();
        clock.publish(10_000_000 + 100 * u64::from(u32::MAX) + 100);
        assert_eq!(t.read(0x5c).unwrap() & IXR_OVERFLOW, IXR_OVERFLOW);
        assert!(t.write(0x0c, CNT_DEC).is_err());
        assert!(t.write(0x00, 1 << 5).is_err());
        assert!(t.read(0x84).is_err());
    }
}
