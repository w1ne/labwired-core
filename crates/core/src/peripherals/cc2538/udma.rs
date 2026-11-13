//! TI CC2538 µDMA controller (`0x400F_F000`, an ARM PrimeCell PL230),
//! software-requested transfers.
//!
//! Source: SWRU319C §10 "Micro Direct Memory Access": §10.2.3 (control
//! table, Table 10-3/10-4: 16-byte entries `src end`, `dst end`, control
//! word, at `CTLBASE + 16 * channel`, alternates at `+0x200`), §10.3.9
//! "Software Request", §10.3.10 "Interrupts and Errors" (Table 10-6:
//! interrupt 46 software transfer done, 47 error), §10.4.3.3 (on completion
//! the channel's `ENASET` bit clears and the control word's `XFERMODE` reads
//! 0), §10.5 register summary (Table 10-12).
//!
//! Modelled:
//! - every register of Table 10-12 stored and read back; the set/clear pairs
//!   (`USEBURST`, `REQMASK`, `ENA`, `ALT`, `PRIO`) on one state word each;
//!   `STAT` reports 32 channels and `MASTEN`;
//! - `SWREQ` on an enabled channel with `CFG.MASTEN` set runs the whole
//!   transfer of the channel's primary (or, with `ALT` set, alternate)
//!   control structure on the next bus tick, in basic or auto-request mode:
//!   `XFERSIZE + 1` items of `SRCSIZE` bytes, each pointer stepping by its
//!   `*INC` (3 = fixed address), from `end - (n - 1) * inc` up to `end`;
//! - completion: the control word's `XFERSIZE` and `XFERMODE` written back to
//!   0, the channel's `ENA` bit cleared, its `CHIS` bit set; the µDMA
//!   software interrupt is the level `CHIS != 0` (on silicon the request
//!   clears when the handler is entered; Contiki-NG's `udma_isr` writes
//!   `CHIS` to clear it);
//! - a bus error on a transfer: the channel disabled and `ERRCLR` set.
//!
//! NOT modelled: peripheral (hardware) requests and `CHASGN`/`CHMAP*`
//! routing (stored only; a channel only moves on `SWREQ`), `REQMASK`, the
//! `ARBSIZE`/priority arbitration (a transfer is atomic), ping-pong,
//! scatter-gather and the alternate-structure switch, mismatched
//! `SRCSIZE`/`DSTSIZE` (the source size is used for both), transfer timing
//! (the transfer completes within one bus tick) and the error interrupt.

use super::{lane_merge, lane_read};
use crate::{Bus, Peripheral, SimResult};

pub const UDMA_SIZE: usize = 0x1000;

const STAT: u64 = 0x000;
const CFG: u64 = 0x004;
const CTLBASE: u64 = 0x008;
const ALTBASE: u64 = 0x00C;
const WAITSTAT: u64 = 0x010;
const SWREQ: u64 = 0x014;
const USEBURSTSET: u64 = 0x018;
const USEBURSTCLR: u64 = 0x01C;
const REQMASKSET: u64 = 0x020;
const REQMASKCLR: u64 = 0x024;
const ENASET: u64 = 0x028;
const ENACLR: u64 = 0x02C;
const ALTSET: u64 = 0x030;
const ALTCLR: u64 = 0x034;
const PRIOSET: u64 = 0x038;
const PRIOCLR: u64 = 0x03C;
const ERRCLR: u64 = 0x04C;
const CHASGN: u64 = 0x500;
const CHIS: u64 = 0x504;
const CHMAP0: u64 = 0x510;
const CHMAP3: u64 = 0x51C;

/// `STAT.DMACHANS` = 31 (32 channels).
const STAT_DMACHANS: u32 = 31 << 16;
const CFG_MASTEN: u32 = 1;
/// The control table base is 1024-byte aligned: `CTLBASE[9:0]` read 0.
const CTLBASE_MASK: u32 = !(1024 - 1);
const ALT_OFFSET: u32 = 0x200;

const XFERMODE_STOP: u32 = 0;
const XFERMODE_BASIC: u32 = 1;
const XFERMODE_AUTO: u32 = 2;
const XFERSIZE_SHIFT: u32 = 4;
const XFERSIZE_MASK: u32 = 0x3FF << XFERSIZE_SHIFT;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Cc2538Udma {
    cfg: u32,
    ctlbase: u32,
    useburst: u32,
    reqmask: u32,
    enable: u32,
    alt: u32,
    prio: u32,
    chasgn: u32,
    chis: u32,
    chmap: [u32; 4],
    err: bool,
    /// Channels requested by software, run on the next bus tick.
    pending: u32,
}

impl Cc2538Udma {
    pub fn new() -> Self {
        Self::default()
    }

    fn read_reg(&self, off: u64) -> u32 {
        match off {
            STAT => STAT_DMACHANS | (self.cfg & CFG_MASTEN),
            CTLBASE => self.ctlbase,
            ALTBASE => self.ctlbase + ALT_OFFSET,
            USEBURSTSET => self.useburst,
            REQMASKSET => self.reqmask,
            ENASET => self.enable,
            ALTSET => self.alt,
            PRIOSET => self.prio,
            ERRCLR => u32::from(self.err),
            CHASGN => self.chasgn,
            CHIS => self.chis,
            o @ CHMAP0..=CHMAP3 if o % 4 == 0 => self.chmap[((o - CHMAP0) / 4) as usize],
            // CFG, SWREQ and the *CLR registers are write-only; WAITSTAT:
            // no channel waits on a peripheral request.
            CFG | SWREQ | WAITSTAT | USEBURSTCLR | REQMASKCLR | ENACLR | ALTCLR | PRIOCLR => 0,
            _ => 0,
        }
    }

    fn write_reg(&mut self, off: u64, v: u32) {
        match off {
            CFG => self.cfg = v & CFG_MASTEN,
            CTLBASE => self.ctlbase = v & CTLBASE_MASK,
            SWREQ if self.cfg & CFG_MASTEN != 0 => self.pending |= v & self.enable,
            USEBURSTSET => self.useburst |= v,
            USEBURSTCLR => self.useburst &= !v,
            REQMASKSET => self.reqmask |= v,
            REQMASKCLR => self.reqmask &= !v,
            ENASET => self.enable |= v,
            ENACLR => {
                self.enable &= !v;
                self.pending &= !v;
            }
            ALTSET => self.alt |= v,
            ALTCLR => self.alt &= !v,
            PRIOSET => self.prio |= v,
            PRIOCLR => self.prio &= !v,
            ERRCLR if v & 1 != 0 => self.err = false,
            CHASGN => self.chasgn = v,
            CHIS => self.chis &= !v,
            o @ CHMAP0..=CHMAP3 if o % 4 == 0 => self.chmap[((o - CHMAP0) / 4) as usize] = v,
            _ => {}
        }
    }

    /// Run channel `ch`'s transfer. Returns false on a bus error.
    fn run_channel(&mut self, ch: u32, bus: &mut dyn Bus) -> bool {
        let entry = u64::from(
            self.ctlbase
                + if self.alt & (1 << ch) != 0 {
                    ALT_OFFSET
                } else {
                    0
                }
                + 16 * ch,
        );
        let (src_end, dst_end, ctrl) = match (
            bus.read_u32(entry),
            bus.read_u32(entry + 4),
            bus.read_u32(entry + 8),
        ) {
            (Ok(s), Ok(d), Ok(c)) => (s, d, c),
            _ => return false,
        };
        let mode = ctrl & 7;
        if mode != XFERMODE_BASIC && mode != XFERMODE_AUTO {
            // Stopped (or a mode not modelled): nothing moves.
            return true;
        }
        let n = ((ctrl & XFERSIZE_MASK) >> XFERSIZE_SHIFT) + 1;
        let size = 1u32 << ((ctrl >> 24) & 3).min(2);
        let step = |inc: u32| if inc == 3 { 0 } else { 1u32 << inc };
        let src_inc = step((ctrl >> 26) & 3);
        let dst_inc = step(ctrl >> 30);
        let src0 = src_end.wrapping_sub((n - 1) * src_inc);
        let dst0 = dst_end.wrapping_sub((n - 1) * dst_inc);
        let ok = crate::with_device_write_bypass(bus, |bus| {
            (0..n).all(|i| {
                let s = u64::from(src0.wrapping_add(i * src_inc));
                let d = u64::from(dst0.wrapping_add(i * dst_inc));
                match size {
                    1 => bus.read_u8(s).and_then(|b| bus.write_u8(d, b)).is_ok(),
                    2 => bus.read_u16(s).and_then(|h| bus.write_u16(d, h)).is_ok(),
                    _ => bus.read_u32(s).and_then(|w| bus.write_u32(d, w)).is_ok(),
                }
            })
        });
        if !ok {
            return false;
        }
        let done = ctrl & !(XFERSIZE_MASK | 7) | XFERMODE_STOP;
        bus.write_u32(entry + 8, done).is_ok()
    }
}

impl Peripheral for Cc2538Udma {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(lane_read(self.read_reg(offset & !3), offset))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        // The set/clear and request registers act on the bits written; merge
        // only the plain read/write ones.
        let reg = offset & !3;
        let v = match reg {
            CFG | CTLBASE | CHASGN | CHMAP0..=CHMAP3 => {
                lane_merge(self.read_reg(reg), offset, value)
            }
            _ => lane_merge(0, offset, value),
        };
        self.write_reg(reg, v);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }

    fn needs_bus_tick(&self) -> bool {
        self.pending != 0
    }

    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        while self.pending != 0 {
            let ch = self.pending.trailing_zeros();
            self.pending &= !(1 << ch);
            if self.run_channel(ch, bus) {
                self.chis |= 1 << ch;
            } else {
                self.err = true;
            }
            self.enable &= !(1 << ch);
        }
    }

    fn irq_line_level(&self) -> Option<bool> {
        Some(self.chis != 0)
    }

    /// No `tick()`: transfers run in `tick_with_bus`.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn on_node_reset(&mut self) {
        *self = Self::new();
    }

    snapshot_impl!(Cc2538Udma, "Cc2538Udma");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SystemBus;

    const TABLE: u32 = 0x2000_0400;

    fn bus_with_udma() -> SystemBus {
        let mut bus = SystemBus::new();
        bus.add_peripheral(
            "udma",
            0x400F_F000,
            UDMA_SIZE as u64,
            None,
            Box::new(Cc2538Udma::new()),
        );
        bus
    }

    const BASE: u64 = 0x400F_F000;

    /// The µDMA seen from the CPU: MMIO through the bus.
    struct Regs<'a>(&'a mut SystemBus);

    impl Regs<'_> {
        fn write_u32(&mut self, off: u64, v: u32) -> SimResult<()> {
            self.0.write_u32(BASE + off, v)
        }

        fn read_u32(&mut self, off: u64) -> SimResult<u32> {
            self.0.read_u32(BASE + off)
        }

        fn needs_bus_tick(&mut self) -> bool {
            let i = self.0.find_peripheral_index_by_name("udma").unwrap();
            self.0.peripherals[i].dev.needs_bus_tick()
        }

        fn irq_line_level(&mut self) -> Option<bool> {
            let i = self.0.find_peripheral_index_by_name("udma").unwrap();
            self.0.peripherals[i].dev.irq_line_level()
        }
    }

    fn udma(bus: &mut SystemBus) -> Regs<'_> {
        Regs(bus)
    }

    fn setup(bus: &mut SystemBus, ch: u32, src_end: u32, dst_end: u32, ctrl: u32) {
        let e = u64::from(TABLE + 16 * ch);
        bus.write_u32(e, src_end).unwrap();
        bus.write_u32(e + 4, dst_end).unwrap();
        bus.write_u32(e + 8, ctrl).unwrap();
        let mut u = udma(bus);
        u.write_u32(CFG, CFG_MASTEN).unwrap();
        u.write_u32(CTLBASE, TABLE).unwrap();
        u.write_u32(ENASET, 1 << ch).unwrap();
    }

    fn run(bus: &mut SystemBus) {
        let i = bus.find_peripheral_index_by_name("udma").unwrap();
        let placeholder: Box<dyn Peripheral> =
            Box::new(crate::peripherals::stub::StubPeripheral::new(0));
        let mut dev = std::mem::replace(&mut bus.peripherals[i].dev, placeholder);
        dev.tick_with_bus(bus);
        bus.peripherals[i].dev = dev;
    }

    #[test]
    fn software_request_runs_an_auto_transfer_and_reports_completion() {
        let mut bus = bus_with_udma();
        for i in 0..8u32 {
            bus.write_u8(u64::from(0x2000_0100 + i), (i + 1) as u8)
                .unwrap();
        }
        // 8 bytes (XFERSIZE 7), byte size, byte src/dst increment (all 0),
        // auto mode: Contiki cc2538-rf RX.
        let ctrl = (7 << XFERSIZE_SHIFT) | XFERMODE_AUTO;
        setup(&mut bus, 3, 0x2000_0107, 0x2000_0207, ctrl);
        assert!(!udma(&mut bus).needs_bus_tick());
        udma(&mut bus).write_u32(SWREQ, 1 << 3).unwrap();
        assert!(udma(&mut bus).needs_bus_tick());
        run(&mut bus);
        for i in 0..8u32 {
            assert_eq!(
                bus.read_u8(u64::from(0x2000_0200 + i)).unwrap(),
                (i + 1) as u8
            );
        }
        let ctrl_after = bus.read_u32(u64::from(TABLE + 16 * 3 + 8)).unwrap();
        assert_eq!(ctrl_after & 7, XFERMODE_STOP, "XFERMODE reads 0");
        assert_eq!(ctrl_after & XFERSIZE_MASK, 0);
        let mut u = udma(&mut bus);
        assert_eq!(
            u.read_u32(ENASET).unwrap() & (1 << 3),
            0,
            "channel disabled"
        );
        assert_eq!(u.read_u32(CHIS).unwrap(), 1 << 3);
        assert_eq!(u.irq_line_level(), Some(true));
        u.write_u32(CHIS, 0xFFFF_FFFF).unwrap();
        assert_eq!(u.irq_line_level(), Some(false));
    }

    #[test]
    fn a_fixed_destination_receives_every_item() {
        // Contiki cc2538-rf TX: byte source increment into the fixed RFDATA.
        let mut bus = bus_with_udma();
        for i in 0..4u32 {
            bus.write_u8(u64::from(0x2000_0100 + i), 0xA0 + i as u8)
                .unwrap();
        }
        // DSTINC 3 (fixed), byte source increment, 4 items.
        let ctrl = (3 << 30) | (3 << XFERSIZE_SHIFT) | XFERMODE_AUTO;
        setup(&mut bus, 0, 0x2000_0103, 0x2000_0300, ctrl);
        udma(&mut bus).write_u32(SWREQ, 1).unwrap();
        run(&mut bus);
        assert_eq!(bus.read_u8(0x2000_0300).unwrap(), 0xA3, "last item wins");
        assert_eq!(bus.read_u8(0x2000_0301).unwrap(), 0, "never stepped");
    }

    #[test]
    fn requests_need_master_enable_and_an_enabled_channel() {
        let mut bus = bus_with_udma();
        let ctrl = XFERMODE_AUTO;
        setup(&mut bus, 1, 0x2000_0100, 0x2000_0200, ctrl);
        let mut u = udma(&mut bus);
        u.write_u32(ENACLR, 1 << 1).unwrap();
        u.write_u32(SWREQ, 1 << 1).unwrap();
        assert!(!u.needs_bus_tick(), "channel disabled");
        u.write_u32(ENASET, 1 << 1).unwrap();
        u.write_u32(CFG, 0).unwrap();
        u.write_u32(SWREQ, 1 << 1).unwrap();
        assert!(!u.needs_bus_tick(), "master disabled");
        assert_eq!(u.read_u32(STAT).unwrap(), STAT_DMACHANS);
        u.write_u32(CFG, CFG_MASTEN).unwrap();
        assert_eq!(u.read_u32(STAT).unwrap(), STAT_DMACHANS | 1);
    }

    #[test]
    fn a_bus_error_disables_the_channel_and_sets_errclr() {
        let mut bus = bus_with_udma();
        // Source in unmapped space.
        let ctrl = (1 << XFERSIZE_SHIFT) | XFERMODE_AUTO;
        setup(&mut bus, 2, 0xE100_0001, 0x2000_0201, ctrl);
        udma(&mut bus).write_u32(SWREQ, 1 << 2).unwrap();
        run(&mut bus);
        let mut u = udma(&mut bus);
        assert_eq!(u.read_u32(ERRCLR).unwrap(), 1);
        assert_eq!(u.read_u32(CHIS).unwrap(), 0);
        assert_eq!(u.read_u32(ENASET).unwrap() & (1 << 2), 0);
        u.write_u32(ERRCLR, 1).unwrap();
        assert_eq!(u.read_u32(ERRCLR).unwrap(), 0);
    }
}
