//! 24 MHz architectural timer, derived solely from SystemBus modeled ticks.
//! One modeled instruction tick is not a claim about physical CPU timing.
use crate::{SimResult, SimulationError};
pub const FREQUENCY: u64 = 24_000_000;
pub const fn key(crn: u32, crm: u32, opc2: u32) -> u32 {
    (crn << 16) | (opc2 << 5) | crm
}
#[derive(Debug, Clone, Default)]
struct Compare {
    value: i128,
    control: u32,
}
impl Compare {
    fn status(&self, now: u128) -> bool {
        self.control & 1 != 0 && now as i128 >= self.value
    }
    fn firing(&self, now: u128) -> bool {
        self.control & 2 == 0 && self.status(now)
    }
    fn ctl(&self, now: u128) -> u64 {
        (self.control | ((self.status(now) as u32) << 2)) as u64
    }
}
#[derive(Debug, Clone, Default)]
pub struct ArchTimer {
    cycle: u64,
    cpu_hz: u64,
    count: u128,
    offset: u64,
    kctl: u32,
    physical: Compare,
    virtual_timer: Compare,
}
impl ArchTimer {
    pub fn synchronize(&mut self, cycle: u64, cpu_hz: u64) {
        self.cycle = cycle;
        self.cpu_hz = cpu_hz;
        self.count = if cpu_hz == 0 {
            0
        } else {
            u128::from(cycle) * u128::from(FREQUENCY) / u128::from(cpu_hz)
        };
    }
    fn virtual_count(&self) -> u128 {
        // A fixed epoch bias keeps subtraction nonnegative for every u64 offset.
        // The guest sees the low 64 bits, while TVAL compares and deadlines retain
        // continuity when the virtual count passes from u64::MAX to zero. Changing
        // CNTVOFF shifts this timeline without rebasing an already programmed CVAL.
        self.count + (1u128 << 64) - self.offset as u128
    }
    fn extend_compare(now: u128, value: u64) -> i128 {
        ((now & !u128::from(u64::MAX)) | u128::from(value)) as i128
    }
    pub fn levels(&self) -> (bool, bool) {
        (
            self.physical.firing(self.count),
            self.virtual_timer.firing(self.virtual_count()),
        )
    }
    pub fn next_deadline(&self) -> Option<u64> {
        if self.cpu_hz == 0 {
            return None;
        }
        let mut nearest = None;
        for (timer, now) in [
            (&self.physical, self.count),
            (&self.virtual_timer, self.virtual_count()),
        ] {
            if timer.control & 3 != 1 {
                continue;
            }
            if timer.status(now) {
                return Some(self.cycle);
            }
            let delta = (timer.value - now as i128) as u128;
            let Some(numerator) = self
                .count
                .checked_add(delta)
                .and_then(|target| target.checked_mul(self.cpu_hz as u128))
            else {
                // An unreachable compare must not hide another timer's valid deadline.
                continue;
            };
            let cycle = numerator.div_ceil(FREQUENCY as u128);
            if let Ok(cycle) = u64::try_from(cycle) {
                nearest = Some(nearest.map_or(cycle, |n: u64| n.min(cycle)));
            }
        }
        nearest
    }
    pub fn read(&self, k: u32, wide: bool) -> SimResult<u64> {
        if wide {
            return match k {
                0xe => Ok(self.count as u64),
                0x1e => Ok(self.virtual_count() as u64),
                0x2e => Ok(self.physical.value as u64),
                0x3e => Ok(self.virtual_timer.value as u64),
                0x4e => Ok(self.offset),
                _ => Err(unknown(k, wide)),
            };
        }
        match k {
            x if x == key(14, 0, 0) => Ok(FREQUENCY),
            x if x == key(14, 1, 0) => Ok(self.kctl as u64),
            x if x == key(14, 2, 0) => Ok((self.physical.value - self.count as i128) as u32 as u64),
            x if x == key(14, 3, 0) => {
                Ok((self.virtual_timer.value - self.virtual_count() as i128) as u32 as u64)
            }
            x if x == key(14, 2, 1) => Ok(self.physical.ctl(self.count)),
            x if x == key(14, 3, 1) => Ok(self.virtual_timer.ctl(self.virtual_count())),
            _ => Err(unknown(k, wide)),
        }
    }
    pub fn write(&mut self, k: u32, wide: bool, v: u64) -> SimResult<()> {
        if wide {
            match k {
                0x2e => self.physical.value = Self::extend_compare(self.count, v),
                0x3e => self.virtual_timer.value = Self::extend_compare(self.virtual_count(), v),
                0x4e => self.offset = v,
                _ => return Err(unknown(k, wide)),
            }
            return Ok(());
        }
        match k {
            x if x == key(14, 0, 0) => {
                if v != FREQUENCY {
                    return Err(SimulationError::NotImplemented(
                        "CNTFRQ differs from fixed 24 MHz oscillator".into(),
                    ));
                }
            }
            x if x == key(14, 1, 0) => {
                if v & 4 != 0 {
                    return Err(SimulationError::NotImplemented(
                        "CNTKCTL event stream".into(),
                    ));
                }
                self.kctl = v as u32 & 0x3ff;
            }
            x if x == key(14, 2, 0) => {
                self.physical.value = self.count as i128 + (v as i32 as i128)
            }
            x if x == key(14, 3, 0) => {
                self.virtual_timer.value = self.virtual_count() as i128 + (v as i32 as i128)
            }
            x if x == key(14, 2, 1) => self.physical.control = v as u32 & 3,
            x if x == key(14, 3, 1) => self.virtual_timer.control = v as u32 & 3,
            // Cortex-A7 native descriptors read ACTLR/PRRR/NMRR as fixed zero. These
            // Linux startup writes request cache/memory attributes; this functional
            // platform has no cache timing or memory-type side effects. Acceptance is
            // explicit and narrowly scoped, not fictitious native CP15 storage.
            0x10020 => {
                if v != 0x40 && v != 0 {
                    return Err(unknown(k, wide));
                }
            }
            0xa0002 => {
                if v != 0xff0a81a8 && v != 0 {
                    return Err(unknown(k, wide));
                }
            }
            0xa0022 => {
                if v != 0x40e040e0 && v != 0 {
                    return Err(unknown(k, wide));
                }
            }
            _ => return Err(unknown(k, wide)),
        }
        Ok(())
    }
}
fn unknown(k: u32, wide: bool) -> SimulationError {
    SimulationError::NotImplemented(format!("ARMv7 external CP15 {k:#x} (wide={wide})"))
}
