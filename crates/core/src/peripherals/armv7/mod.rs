//! RV1103 initial functional platform: one CPU, GICv2, fixed 24 MHz timer,
//! DW UART2. TrustZone banks, virtualization, DMA and serial/cache timing are
//! outside this slice; the native CPU continues owning MMU and execution.
pub mod dw_uart;
pub mod gic;
pub mod rk3x_i2c;
pub mod timer;
use crate::{SimResult, SimulationError};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
fn unique_id() -> u64 {
    NEXT_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1))
        .expect("ARM bus identity space exhausted")
}
#[derive(Debug)]
pub struct Armv7Fabric {
    identity: u64,
    pub(crate) uart2_index: Option<usize>,
    pub gic: Arc<Mutex<gic::Gic>>,
    pub timer: timer::ArchTimer,
}
impl Default for Armv7Fabric {
    fn default() -> Self {
        Self {
            identity: unique_id(),
            uart2_index: None,
            gic: Arc::new(Mutex::new(gic::Gic::default())),
            timer: timer::ArchTimer::default(),
        }
    }
}
impl Clone for Armv7Fabric {
    fn clone(&self) -> Self {
        Self {
            identity: unique_id(),
            uart2_index: None,
            gic: Arc::new(Mutex::new(self.gic.lock().unwrap().clone())),
            timer: self.timer.clone(),
        }
    }
}
impl Armv7Fabric {
    pub fn code_identity(&self) -> u64 {
        self.identity
    }
    pub fn synchronize(&mut self, cycle: u64, cpu_hz: u64) {
        self.timer.synchronize(cycle, cpu_hz);
        self.sync_levels();
    }
    pub fn sync_levels(&self) {
        let (p, v) = self.timer.levels();
        let mut g = self.gic.lock().unwrap();
        g.set_level(29, p);
        g.set_level(27, v);
    }
}
impl crate::bus::SystemBus {
    /// Opt in to the functional RV1103 I2C3 slice after installing ARMv7.
    /// This does not alter the foundation boot tree or configure any external part.
    pub fn configure_armv7_rv1103_i2c3(&mut self) -> SimResult<()> {
        const BASE: u64 = 0xff460000;
        const SIZE: u64 = 0x1000;
        let fabric = self
            .irq_fabric
            .armv7
            .as_ref()
            .ok_or_else(|| SimulationError::Other("I2C3 requires ARMv7 fabric".into()))?;
        if self.peripherals.iter().any(|p| {
            p.name == "i2c3" || (p.base < BASE + SIZE && p.base.saturating_add(p.size) > BASE)
        }) {
            return Err(SimulationError::Other(
                "I2C3 name or MMIO window already mapped".into(),
            ));
        }
        if std::iter::once(&self.flash)
            .chain(std::iter::once(&self.ram))
            .chain(self.extra_mem.iter())
            .any(|m| {
                m.base_addr < BASE + SIZE && m.base_addr.saturating_add(m.data.len() as u64) > BASE
            })
        {
            return Err(SimulationError::Other(
                "I2C3 MMIO window overlaps CPU memory".into(),
            ));
        }
        let gic = fabric.gic.clone();
        self.add_peripheral(
            "i2c3",
            BASE,
            SIZE,
            None,
            Box::new(rk3x_i2c::Rk3xI2c::new(gic, 53)),
        );
        self.recompute_walk_deletable();
        Ok(())
    }
    /// Install RV1103 platform models through the existing bus dispatch. The
    /// caller supplies CPU Hz and RAM; this installs no board/boot image.
    pub fn configure_armv7_rv1103(&mut self) -> SimResult<()> {
        if self.cpu_hz == 0 {
            return Err(SimulationError::Other(
                "ARMv7 requires nonzero cpu_hz".into(),
            ));
        }
        if self.irq_fabric.armv7.is_some() {
            return Err(SimulationError::Other(
                "ARMv7 fabric already installed".into(),
            ));
        }
        let mut f = Armv7Fabric::default();
        f.synchronize(self.current_cycle, self.cpu_hz);
        self.add_peripheral(
            "gicd",
            0xff1f1000,
            0x1000,
            None,
            Box::new(gic::GicWindow {
                controller: f.gic.clone(),
                cpu: false,
            }),
        );
        self.add_peripheral(
            "gicc",
            0xff1f2000,
            0x2000,
            None,
            Box::new(gic::GicWindow {
                controller: f.gic.clone(),
                cpu: true,
            }),
        );
        f.uart2_index = Some(self.peripherals.len());
        self.add_peripheral(
            "uart2",
            0xff4c0000,
            0x100,
            None,
            Box::new(dw_uart::DwUart::new(f.gic.clone(), 59)),
        );
        self.irq_fabric.armv7 = Some(f);
        self.rebuild_peripheral_ranges();
        self.recompute_walk_deletable();
        Ok(())
    }
}
