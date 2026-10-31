//! Armv8-M PMSAv8 MPU register file (Cortex-M23/M33/M55/M85).
//!
//! The Armv8-M MPU is programmed through a per-region RBAR/RLAR pair selected
//! by RNR, plus three alias pairs that address regions RNR+1..RNR+3 (ARMv8-M
//! ARM DDI 0553, B3.5 and D1.2.142-150). It is not the Armv7-M PMSAv7 layout:
//! RBAR bit 4 is SH[0] here, where PMSAv7 has VALID, and the word at 0x10 is
//! RLAR (limit address + attribute index + enable), not RASR. That is why the
//! block is opt-in per chip and not part of the SCB model, whose MPU offsets
//! keep the PMSAv7 storage the Armv7-M parts use.
//!
//! | Offset (0xE000_ED90+) | Register        |
//! |-----------------------|-----------------|
//! | 0x00                  | MPU_TYPE        |
//! | 0x04                  | MPU_CTRL        |
//! | 0x08                  | MPU_RNR         |
//! | 0x0C / 0x10           | MPU_RBAR / RLAR |
//! | 0x14..0x28            | RBAR_A1..RLAR_A3 |
//! | 0x30 / 0x34           | MPU_MAIR0/MAIR1 |
//!
//! ```yaml
//! - id: "mpu"
//!   type: "armv8m_mpu"
//!   base_address: 0xE000ED90
//!   size: "56B"
//!   config: { regions: 8 }
//! ```
//!
//! `configure_cortex_m` finds it through `Peripheral::armv8m_mpu_state`, ends
//! the SCB window at 0xE000_ED90 so this block serves the MPU offsets, and
//! gives the region table to the core for the TT instruction (`cmse_TT`),
//! which reports the MPU region an address falls in. Zephyr's Armv8-M MPU
//! driver finds the static region that a dynamic area (thread stack) sits in
//! that way; with no region file, `get_region_index` fails its assertion.
//!
//! What is not modelled: enforcement. Regions are stored and reported by TT,
//! but no access is checked against them, so a MemManage fault never
//! happens. Firmware that relies on the MPU to trap a bad access (Zephyr
//! `tests/kernel/mem_protect`) is therefore not covered.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use crate::{Peripheral, SimResult};

/// Most regions an Armv8-M MPU implements (MPU_TYPE.DREGION is 8 bits, but
/// the Cortex-M23/M33/M55/M85 TRMs give 0, 4, 8, 12 or 16).
pub const MAX_REGIONS: usize = 16;

const TYPE: u64 = 0x00;
const CTRL: u64 = 0x04;
const RNR: u64 = 0x08;
const RBAR: u64 = 0x0C;
#[cfg(test)]
const RLAR: u64 = 0x10;
const ALIAS_FIRST: u64 = 0x14;
const ALIAS_LAST: u64 = 0x28;
const MAIR0: u64 = 0x30;
const MAIR1: u64 = 0x34;

// RBAR: BASE[31:5], SH[4:3], AP[2:1], XN[0]. RLAR: LIMIT[31:5], PXN[4]
// (Armv8.1-M), AttrIndx[3:1], EN[0]. Both are stored as written.
const RLAR_EN: u32 = 1;
const CTRL_ENABLE: u32 = 1;
const CTRL_MASK: u32 = 0x7;

/// The MPU state shared between this model and the core (TT).
#[derive(Debug)]
pub struct MpuV8State {
    regions: usize,
    ctrl: AtomicU32,
    rnr: AtomicU32,
    rbar: [AtomicU32; MAX_REGIONS],
    rlar: [AtomicU32; MAX_REGIONS],
}

/// What TT reports about one address (ARMv8-M ARM C2.4.247, the TT result).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MpuLookup {
    pub region: Option<u8>,
    /// Read / read-write permitted for the privilege TT was asked about.
    pub r: bool,
    pub rw: bool,
}

impl MpuV8State {
    pub fn new(regions: usize) -> Self {
        Self {
            regions: regions.min(MAX_REGIONS),
            ctrl: AtomicU32::new(0),
            rnr: AtomicU32::new(0),
            rbar: std::array::from_fn(|_| AtomicU32::new(0)),
            rlar: std::array::from_fn(|_| AtomicU32::new(0)),
        }
    }

    pub fn regions(&self) -> usize {
        self.regions
    }

    pub fn ctrl(&self) -> u32 {
        self.ctrl.load(Ordering::Relaxed)
    }

    pub fn rbar(&self, n: usize) -> u32 {
        self.rbar.get(n).map_or(0, |r| r.load(Ordering::Relaxed))
    }

    pub fn rlar(&self, n: usize) -> u32 {
        self.rlar.get(n).map_or(0, |r| r.load(Ordering::Relaxed))
    }

    /// TT's MPU half. MREGION is valid only with the MPU enabled and the
    /// address inside exactly one enabled region; overlapping regions or no
    /// hit leave it invalid (the default memory map then decides access,
    /// which is read-write here). Region permissions come from RBAR.AP:
    /// 0b00 RW privileged-only, 0b01 RW any, 0b10 RO privileged-only,
    /// 0b11 RO any.
    pub fn lookup(&self, addr: u32, privileged: bool) -> MpuLookup {
        let default = MpuLookup {
            region: None,
            r: true,
            rw: true,
        };
        if self.ctrl() & CTRL_ENABLE == 0 {
            return default;
        }
        let mut hit = None;
        for n in 0..self.regions {
            let rlar = self.rlar(n);
            if rlar & RLAR_EN == 0 {
                continue;
            }
            let base = self.rbar(n) & !0x1F;
            let limit = rlar | 0x1F;
            if addr >= base && addr <= limit {
                if hit.is_some() {
                    return default;
                }
                hit = Some(n);
            }
        }
        let Some(n) = hit else {
            return default;
        };
        let ap = (self.rbar(n) >> 1) & 0x3;
        let any = ap & 1 != 0;
        let r = privileged || any;
        MpuLookup {
            region: Some(n as u8),
            r,
            rw: r && ap & 2 == 0,
        }
    }

    fn selected(&self, offset: u64) -> usize {
        let rnr = self.rnr.load(Ordering::Relaxed) as usize;
        if offset < ALIAS_FIRST {
            rnr
        } else {
            // Alias n (1..=3) addresses region (RNR[7:2] << 2) + n.
            (rnr & !3) + ((offset - ALIAS_FIRST) / 8 + 1) as usize
        }
    }

    fn reset(&self) {
        self.ctrl.store(0, Ordering::Relaxed);
        self.rnr.store(0, Ordering::Relaxed);
        for n in 0..MAX_REGIONS {
            self.rbar[n].store(0, Ordering::Relaxed);
            self.rlar[n].store(0, Ordering::Relaxed);
        }
    }
}

#[derive(Debug)]
pub struct Armv8mMpu {
    state: Arc<MpuV8State>,
    mair: [u32; 2],
}

impl Armv8mMpu {
    pub fn new(regions: usize) -> Self {
        Self {
            state: Arc::new(MpuV8State::new(regions)),
            mair: [0; 2],
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        let s = &self.state;
        match offset {
            // DREGION[15:8]; SEPARATE = 0 (unified).
            TYPE => (s.regions as u32) << 8,
            CTRL => s.ctrl(),
            RNR => s.rnr.load(Ordering::Relaxed),
            RBAR..=ALIAS_LAST => {
                let n = s.selected(offset);
                if (offset - RBAR) % 8 == 0 {
                    s.rbar(n)
                } else {
                    s.rlar(n)
                }
            }
            MAIR0 => self.mair[0],
            MAIR1 => self.mair[1],
            _ => 0,
        }
    }
}

impl Peripheral for Armv8mMpu {
    // A pure register window with no `tick()`.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let v = self.read_reg(offset & !3);
        Ok((v >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let shift = (offset & 3) * 8;
        let old = self.read_reg(offset & !3);
        let v = (old & !(0xFF << shift)) | (u32::from(value) << shift);
        self.write_u32(offset & !3, v)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        let offset = offset & !3;
        let s = &self.state;
        match offset {
            CTRL => s.ctrl.store(value & CTRL_MASK, Ordering::Relaxed),
            // Writes of a region number the MPU does not have are UNPREDICTABLE;
            // keep the low bits, which is what silicon with 8 regions does.
            RNR => s.rnr.store(value & 0xFF, Ordering::Relaxed),
            RBAR..=ALIAS_LAST => {
                let n = s.selected(offset);
                if n < s.regions {
                    if (offset - RBAR) % 8 == 0 {
                        s.rbar[n].store(value, Ordering::Relaxed);
                    } else {
                        s.rlar[n].store(value, Ordering::Relaxed);
                    }
                }
            }
            MAIR0 => self.mair[0] = value,
            MAIR1 => self.mair[1] = value,
            _ => crate::census_reg!("armv8m_mpu:Armv8mMpu", offset, "write"),
        }
        Ok(())
    }

    fn armv8m_mpu_state(&self) -> Option<Arc<MpuV8State>> {
        Some(self.state.clone())
    }

    fn on_node_reset(&mut self) {
        self.state.reset();
        self.mair = [0; 2];
    }

    fn snapshot(&self) -> serde_json::Value {
        let s = &self.state;
        serde_json::json!({
            "ctrl": s.ctrl(),
            "rnr": s.rnr.load(Ordering::Relaxed),
            "rbar": (0..MAX_REGIONS).map(|n| s.rbar(n)).collect::<Vec<_>>(),
            "rlar": (0..MAX_REGIONS).map(|n| s.rlar(n)).collect::<Vec<_>>(),
            "mair": self.mair,
        })
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        let s = &self.state;
        let u = |v: &serde_json::Value| v.as_u64().unwrap_or(0) as u32;
        if let Some(v) = state.get("ctrl") {
            s.ctrl.store(u(v), Ordering::Relaxed);
        }
        if let Some(v) = state.get("rnr") {
            s.rnr.store(u(v), Ordering::Relaxed);
        }
        for (key, regs) in [("rbar", &s.rbar), ("rlar", &s.rlar)] {
            if let Some(arr) = state.get(key).and_then(|v| v.as_array()) {
                for (n, v) in arr.iter().enumerate().take(MAX_REGIONS) {
                    regs[n].store(u(v), Ordering::Relaxed);
                }
            }
        }
        if let Some(arr) = state.get("mair").and_then(|v| v.as_array()) {
            for (n, v) in arr.iter().enumerate().take(2) {
                self.mair[n] = u(v);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(m: &mut Armv8mMpu, n: u32, base: u32, limit: u32, ap: u32) {
        m.write_u32(RNR, n).unwrap();
        m.write_u32(RBAR, base | (ap << 1)).unwrap();
        m.write_u32(RLAR, (limit & !0x1F) | RLAR_EN).unwrap();
    }

    #[test]
    fn region_file_is_per_region() {
        let mut m = Armv8mMpu::new(8);
        assert_eq!(m.read_reg(TYPE), 0x0800, "DREGION = 8");
        region(&mut m, 0, 0x0800_0000, 0x0807_FFFF, 0b11);
        region(&mut m, 1, 0x2000_0000, 0x2002_FFFF, 0b01);
        m.write_u32(RNR, 0).unwrap();
        assert_eq!(m.read_reg(RBAR), 0x0800_0006);
        assert_eq!(m.read_reg(RLAR), 0x0807_FFE1);
        m.write_u32(RNR, 1).unwrap();
        assert_eq!(m.read_reg(RBAR), 0x2000_0002);
        // Region 9 does not exist on an 8-region MPU.
        m.write_u32(RNR, 9).unwrap();
        m.write_u32(RBAR, 0x1234_5678).unwrap();
        assert_eq!(m.read_reg(RBAR), 0);
    }

    #[test]
    fn alias_registers_address_rnr_plus_n() {
        let mut m = Armv8mMpu::new(8);
        m.write_u32(RNR, 4).unwrap();
        m.write_u32(0x14, 0x1000_0000).unwrap(); // RBAR_A1 -> region 5
        m.write_u32(0x28, 0x1FFF_FFE1).unwrap(); // RLAR_A3 -> region 7
        assert_eq!(m.state.rbar(5), 0x1000_0000);
        assert_eq!(m.state.rlar(7), 0x1FFF_FFE1);
        // RNR[1:0] are ignored by the aliases.
        m.write_u32(RNR, 6).unwrap();
        assert_eq!(m.read_reg(0x14), 0x1000_0000, "A1 still region 5");
    }

    #[test]
    fn lookup_reports_the_single_matching_region() {
        let mut m = Armv8mMpu::new(8);
        region(&mut m, 0, 0x0800_0000, 0x0807_FFFF, 0b11);
        region(&mut m, 2, 0x2000_0000, 0x2002_FFFF, 0b00);
        // MPU disabled: no region.
        assert_eq!(m.state.lookup(0x2000_0100, true).region, None);
        m.write_u32(CTRL, CTRL_ENABLE).unwrap();
        assert_eq!(m.state.lookup(0x2000_0100, true).region, Some(2));
        assert_eq!(m.state.lookup(0x0800_0000, true).region, Some(0));
        assert_eq!(m.state.lookup(0x4000_0000, true).region, None);
        // Privileged-only RW: unprivileged gets neither R nor RW.
        let l = m.state.lookup(0x2000_0100, false);
        assert!(!l.r && !l.rw);
        // RO any.
        let l = m.state.lookup(0x0800_0000, false);
        assert!(l.r && !l.rw);
        // Overlap: no valid region.
        region(&mut m, 3, 0x2000_0000, 0x2000_0FFF, 0b01);
        assert_eq!(m.state.lookup(0x2000_0100, true).region, None);
        assert_eq!(m.state.lookup(0x2000_1000, true).region, Some(2));
    }

    #[test]
    fn snapshot_round_trips() {
        let mut m = Armv8mMpu::new(8);
        region(&mut m, 1, 0x2000_0000, 0x2000_FFFF, 0b01);
        m.write_u32(CTRL, 5).unwrap();
        m.write_u32(MAIR0, 0x44).unwrap();
        let s = m.snapshot();
        let mut n = Armv8mMpu::new(8);
        n.restore(s).unwrap();
        assert_eq!(n.state.rbar(1), 0x2000_0002);
        assert_eq!(n.read_reg(CTRL), 5);
        assert_eq!(n.read_reg(MAIR0), 0x44);
        n.on_node_reset();
        assert_eq!(n.state.rlar(1), 0);
    }
}
