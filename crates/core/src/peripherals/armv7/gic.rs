//! A single-CPU, 256-INTID GICv2. No virtualization or multicore interfaces.
use crate::{Peripheral, SimResult, SimulationError};
use std::sync::{Arc, Mutex};
/// Canonical interrupt banks, not a cache of another representation.
#[derive(Debug, Clone, Copy, Default)]
struct Bits([u64; 4]);
impl Bits {
    #[inline]
    fn get(&self, id: usize) -> bool {
        self.0[id / 64] & (1u64 << (id % 64)) != 0
    }
    #[inline]
    fn set(&mut self, id: usize, value: bool) {
        let bit = 1u64 << (id % 64);
        if value {
            self.0[id / 64] |= bit;
        } else {
            self.0[id / 64] &= !bit;
        }
    }
    fn iter(self) -> BitIds {
        BitIds {
            words: self.0,
            word: 0,
        }
    }
}
struct BitIds {
    words: [u64; 4],
    word: usize,
}
impl Iterator for BitIds {
    type Item = usize;
    fn next(&mut self) -> Option<usize> {
        while self.word < 4 {
            let bits = self.words[self.word];
            if bits == 0 {
                self.word += 1;
                continue;
            }
            self.words[self.word] &= bits - 1;
            return Some(self.word * 64 + bits.trailing_zeros() as usize);
        }
        None
    }
}
#[derive(Debug, Clone)]
pub struct Gic {
    distributor: u32,
    cpu: u32,
    pmr: u8,
    bpr: u8,
    group: Bits,
    enabled: Bits,
    pending: Bits,
    active: Bits,
    level: Bits,
    priority: [u8; 256],
    targets: [u8; 256],
    edge: Bits,
}
impl Default for Gic {
    fn default() -> Self {
        Self {
            distributor: 0,
            cpu: 0,
            pmr: 0,
            bpr: 0,
            group: Bits::default(),
            // The GIC-400 profile has permanently enabled SGIs (RAO/WI).
            enabled: Bits([0xffff, 0, 0, 0]),
            pending: Bits::default(),
            active: Bits::default(),
            level: Bits::default(),
            priority: [0xa0; 256],
            targets: [1; 256],
            edge: Bits([0xffff, 0, 0, 0]),
        }
    }
}
impl Gic {
    pub fn set_level(&mut self, id: usize, high: bool) {
        if id < 256 {
            if self.edge.get(id) && high && !self.level.get(id) {
                self.pending.set(id, true);
            }
            self.level.set(id, high);
        }
    }
    fn running(&self) -> u8 {
        self.active
            .iter()
            .map(|i| self.priority[i])
            .min()
            .unwrap_or(255)
    }
    fn candidate(&self, group: Option<bool>) -> Option<usize> {
        let g0 = self.distributor & self.cpu & 1 != 0 && group != Some(true);
        let g1 = self.distributor & self.cpu & 2 != 0 && group != Some(false);
        if (!g0 && !g1) || self.pmr == 0 {
            return None;
        }
        let run = self.running();
        let mut selected = None;
        for word in 0..4 {
            let pending = self.pending.0[word] | (self.level.0[word] & !self.edge.0[word]);
            let groups = (if g0 { !self.group.0[word] } else { 0 })
                | (if g1 { self.group.0[word] } else { 0 });
            let mut eligible = self.enabled.0[word] & pending & !self.active.0[word] & groups;
            while eligible != 0 {
                let i = word * 64 + eligible.trailing_zeros() as usize;
                eligible &= eligible - 1;
                if self.targets[i] & 1 == 0 || self.priority[i] >= self.pmr {
                    continue;
                }
                if run != 255
                    && (self.priority[i] as u16 >> (self.bpr + 1)) >= (run as u16 >> (self.bpr + 1))
                {
                    continue;
                }
                if selected
                    .is_none_or(|old: usize| (self.priority[i], i) < (self.priority[old], old))
                {
                    selected = Some(i);
                }
            }
        }
        selected
    }
    pub fn levels(&self) -> (bool, bool) {
        let g0 = self.candidate(Some(false)).is_some();
        let g1 = self.candidate(Some(true)).is_some();
        (g1 || (g0 && self.cpu & 8 == 0), g0 && self.cpu & 8 != 0)
    }
    fn bitmap(&self, base: usize, kind: u64) -> u32 {
        let word = base / 64;
        let bits = match kind {
            0x80 => self.group.0[word],
            0x100 => self.enabled.0[word],
            0x200 => self.pending.0[word] | (self.level.0[word] & !self.edge.0[word]),
            0x300 => self.active.0[word],
            _ => 0,
        };
        (bits >> (base % 64)) as u32
    }
    pub fn dist_read(&self, o: u64, w: u32) -> SimResult<u32> {
        if matches!(w, 1 | 2 | 4)
            && o % w as u64 == 0
            && ((0x400..0x500).contains(&o) || (0x800..0x900).contains(&o))
            && o + w as u64 <= if o < 0x500 { 0x500 } else { 0x900 }
        {
            let a = if o < 0x500 {
                &self.priority
            } else {
                &self.targets
            };
            let n = (o & 255) as usize;
            return Ok((0..w as usize).fold(0, |v, i| v | ((a[n + i] as u32) << (i * 8))));
        }
        if w != 4 || o % 4 != 0 {
            return Err(access("GICD", o, w));
        }
        Ok(match o {
            0 => self.distributor,
            4 => 7,
            8 => 0x0102043b,
            0x80..=0x9c => self.bitmap(((o - 0x80) / 4 * 32) as usize, 0x80),
            0x100..=0x11c | 0x180..=0x19c => self.bitmap(((o & 0x7f) / 4 * 32) as usize, 0x100),
            0x200..=0x21c | 0x280..=0x29c => self.bitmap(((o & 0x7f) / 4 * 32) as usize, 0x200),
            0x300..=0x31c | 0x380..=0x39c => self.bitmap(((o & 0x7f) / 4 * 32) as usize, 0x300),
            0xc00..=0xc3c => (0..16).fold(0, |v, j| {
                v | ((self.edge.get(((o - 0xc00) / 4 * 16) as usize + j) as u32) << (2 * j + 1))
            }),
            0xf10..=0xf1c | 0xf20..=0xf2c => {
                let base = ((o & 0xf) / 4 * 4) as usize;
                (0..4).fold(0, |v, j| {
                    v | ((self.pending.get(base + j) as u32) << (8 * j))
                })
            }
            0xfe8 => 0x20,
            0xfec => 0,
            0xff0 => 0x0d,
            0xff4 => 0xf0,
            0xff8 => 5,
            0xffc => 0xb1,
            0xfd0..=0xfe4 => 0,
            _ => return Err(access("GICD", o, w)),
        })
    }
    pub fn dist_write(&mut self, o: u64, w: u32, v: u32) -> SimResult<()> {
        if matches!(w, 1 | 2 | 4)
            && o % w as u64 == 0
            && ((0x400..0x500).contains(&o) || (0x800..0x900).contains(&o))
            && o + w as u64 <= if o < 0x500 { 0x500 } else { 0x900 }
        {
            let n = (o & 255) as usize;
            for i in 0..w as usize {
                if o < 0x500 {
                    self.priority[n + i] = (v >> (i * 8)) as u8;
                } else if n + i >= 32 {
                    self.targets[n + i] = (v >> (i * 8)) as u8 & 1;
                }
            }
            return Ok(());
        }
        if w != 4 || o % 4 != 0 {
            return Err(access("GICD", o, w));
        }
        match o {
            0 => self.distributor = v & 3,
            4 | 8 => {}
            0x80..=0x9c => {
                for j in 0..32 {
                    self.group
                        .set(((o - 0x80) / 4 * 32) as usize + j, v & (1 << j) != 0);
                }
            }
            0x100..=0x11c
            | 0x180..=0x19c
            | 0x200..=0x21c
            | 0x280..=0x29c
            | 0x300..=0x31c
            | 0x380..=0x39c => {
                let base = ((o & 0x7f) / 4 * 32) as usize;
                let set = o & 0x80 == 0;
                for j in 0..32 {
                    if v & (1 << j) != 0 {
                        match o & 0x300 {
                            0x100 if base + j >= 16 => self.enabled.set(base + j, set),
                            0x200 => self.pending.set(base + j, set),
                            0x300 => self.active.set(base + j, set),
                            _ => {}
                        }
                    }
                }
            }
            0xc00..=0xc3c => {
                for j in 0..16 {
                    let i = ((o - 0xc00) / 4 * 16) as usize + j;
                    if i >= 16 {
                        self.edge.set(i, v & (2 << (2 * j)) != 0);
                    }
                }
            }
            0xf00 => {
                let filter = (v >> 24) & 3;
                if filter == 2 || (filter == 0 && v & (1 << 16) != 0) {
                    self.pending.set((v & 15) as usize, true);
                }
            }
            0xd00..=0xd1c | 0xe00..=0xe1c => {}
            0xf10..=0xf1c | 0xf20..=0xf2c => {
                let base = ((o & 0xf) / 4 * 4) as usize;
                for j in 0..4 {
                    // Only source CPU0 exists; the other source bits are RAZ/WI.
                    if v & (1 << (8 * j)) != 0 {
                        self.pending.set(base + j, o >= 0xf20);
                    }
                }
            }
            0xfd0..=0xffc => {}
            _ => return Err(access("GICD", o, w)),
        }
        Ok(())
    }
    pub fn cpu_read(&mut self, o: u64, w: u32) -> SimResult<u32> {
        if w != 4 || o % 4 != 0 {
            return Err(access("GICC", o, w));
        }
        Ok(match o {
            0 => self.cpu,
            4 => self.pmr as u32,
            8 => self.bpr as u32,
            0xc => {
                if let Some(i) = self.candidate(None) {
                    self.pending.set(i, false);
                    self.active.set(i, true);
                    i as u32
                } else {
                    1023
                }
            }
            0x14 => self.running() as u32,
            0x18 => self.candidate(None).map_or(1023, |i| i as u32),
            0x1c => self.bpr as u32,
            0xd0..=0xdc => self
                .active
                .iter()
                .filter(|&i| self.priority[i] as u64 / 64 == (o - 0xd0) / 4)
                .fold(0, |v, i| v | (1 << (self.priority[i] % 64 / 2))),
            0xfc => 0x0002043b,
            0xfd0..=0xfe4 => 0,
            0xfe8 => 0x20,
            0xfec => 0,
            0xff0 => 0x0d,
            0xff4 => 0xf0,
            0xff8 => 5,
            0xffc => 0xb1,
            _ => return Err(access("GICC", o, w)),
        })
    }
    pub fn cpu_write(&mut self, o: u64, w: u32, v: u32) -> SimResult<()> {
        if w != 4 || o % 4 != 0 {
            return Err(access("GICC", o, w));
        }
        match o {
            0 => {
                if v & ((1 << 9) | (1 << 10)) != 0 {
                    return Err(SimulationError::NotImplemented("GIC EOImode1".into()));
                }
                self.cpu = v & 0x1f;
            }
            4 => self.pmr = v as u8,
            8 | 0x1c => self.bpr = (v & 7) as u8,
            0x10 => {
                let id = (v & 1023) as usize;
                if id < 256 && self.active.get(id) && self.priority[id] == self.running() {
                    self.active.set(id, false);
                }
            }
            0xd0..=0xdc => {
                if v != 0 {
                    return Err(SimulationError::NotImplemented(
                        "software GIC active-priority injection".into(),
                    ));
                }
                for i in self.active.iter() {
                    if self.priority[i] as u64 / 64 == (o - 0xd0) / 4 {
                        self.active.set(i, false);
                    }
                }
            }
            0xfc => {}
            _ => return Err(access("GICC", o, w)),
        }
        Ok(())
    }
}
fn access(name: &str, o: u64, w: u32) -> SimulationError {
    SimulationError::NotImplemented(format!("{name} register {o:#x}, width {w}"))
}
#[derive(Debug)]
pub struct GicWindow {
    pub controller: Arc<Mutex<Gic>>,
    pub cpu: bool,
}
impl GicWindow {
    fn read_width(&self, o: u64, w: u32) -> SimResult<u32> {
        let mut g = self.controller.lock().unwrap();
        if self.cpu {
            g.cpu_read(o, w)
        } else {
            g.dist_read(o, w)
        }
    }
    fn write_width(&mut self, o: u64, w: u32, v: u32) -> SimResult<()> {
        let mut g = self.controller.lock().unwrap();
        if self.cpu {
            g.cpu_write(o, w, v)
        } else {
            g.dist_write(o, w, v)
        }
    }
}
impl Peripheral for GicWindow {
    fn read(&self, o: u64) -> SimResult<u8> {
        Ok(self.read_width(o, 1)? as u8)
    }
    fn write(&mut self, o: u64, v: u8) -> SimResult<()> {
        self.write_width(o, 1, v as u32)
    }
    fn read_u16(&self, o: u64) -> SimResult<u16> {
        Ok(self.read_width(o, 2)? as u16)
    }
    fn read_u32(&self, o: u64) -> SimResult<u32> {
        self.read_width(o, 4)
    }
    fn write_u16(&mut self, o: u64, v: u16) -> SimResult<()> {
        self.write_width(o, 2, v as u32)
    }
    fn write_u32(&mut self, o: u64, v: u32) -> SimResult<()> {
        self.write_width(o, 4, v)
    }
    fn needs_legacy_walk(&self) -> bool {
        false
    }
}
