//! RV1103 initial functional platform: one CPU, GICv2, fixed 24 MHz timer,
//! DW UART2. TrustZone banks, virtualization, DMA and serial/cache timing are
//! outside this slice; the native CPU continues owning MMU and execution.
pub mod dw_uart;
pub mod gic;
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
