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
    pub(crate) uart4_index: Option<usize>,
    pub(crate) uart4_irq: Option<u32>,
    pub gic: Arc<Mutex<gic::Gic>>,
    pub timer: timer::ArchTimer,
}
impl Default for Armv7Fabric {
    fn default() -> Self {
        Self {
            identity: unique_id(),
            uart2_index: None,
            uart4_index: None,
            uart4_irq: None,
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
            uart4_index: None,
            uart4_irq: None,
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
/// Auxiliary chip metadata read only by the explicit native basket opt-in.
/// The generic descriptor still declares no peripheral models.
pub(crate) const RV1103_DESCRIPTOR: &str = include_str!("../../../../../configs/chips/rv1103.yaml");
#[derive(serde::Deserialize)]
struct NativeBasketDescriptor {
    native_basket_peripherals: Vec<labwired_config::PeripheralConfig>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NativeBasketWindow {
    pub base: u64,
    pub size: u64,
    pub irq: u32,
}
pub(crate) fn native_basket_windows(yaml: &str) -> SimResult<[NativeBasketWindow; 2]> {
    let descriptor: NativeBasketDescriptor = serde_yaml::from_str(yaml)
        .map_err(|e| SimulationError::Other(format!("RV1103 native basket descriptor: {e}")))?;
    let entries = descriptor.native_basket_peripherals;
    let invalid =
        || SimulationError::Other("RV1103 native basket peripheral map is invalid".into());
    if entries.len() != 2 {
        return Err(invalid());
    }
    let map = super::chip_map::ChipMap::new(&entries);
    let mut windows = Vec::new();
    for (id, kind, size) in [("i2c3", "rk3x_i2c", 0x1000), ("uart4", "dw_uart", 0x100)] {
        let entry = entries.iter().find(|p| p.id == id).ok_or_else(invalid)?;
        let base = map.base_of(id).ok_or_else(invalid)?;
        let declared_size = entry.size.as_deref().ok_or_else(invalid)?;
        let irq = entry.irq.ok_or_else(invalid)?;
        let end = base.checked_add(size).ok_or_else(invalid)?;
        if entry.r#type != kind
            || labwired_config::parse_size(declared_size).ok() != Some(size)
            || base == 0
            || base % 4 != 0
            || end > u64::from(u32::MAX) + 1
            || !(32..1020).contains(&irq)
            || entry.irq_controller.is_some()
            || entry.clock.is_some()
            || !entry.config.is_empty()
        {
            return Err(invalid());
        }
        windows.push(NativeBasketWindow { base, size, irq });
    }
    if windows[0].irq == windows[1].irq
        || (windows[0].base < windows[1].base + windows[1].size
            && windows[1].base < windows[0].base + windows[0].size)
    {
        return Err(invalid());
    }
    Ok([windows[0], windows[1]])
}
impl crate::bus::SystemBus {
    /// Opt in to the functional RV1103 I2C3 slice after installing ARMv7.
    /// This does not alter the foundation boot tree or configure any external part.
    pub fn configure_armv7_rv1103_i2c3(&mut self) -> SimResult<()> {
        self.configure_armv7_rv1103_i2c3_from_descriptor(RV1103_DESCRIPTOR)
    }
    fn configure_armv7_rv1103_i2c3_from_descriptor(&mut self, yaml: &str) -> SimResult<()> {
        let NativeBasketWindow { base, size, irq } = native_basket_windows(yaml)?[0];
        let fabric = self
            .irq_fabric
            .armv7
            .as_ref()
            .ok_or_else(|| SimulationError::Other("I2C3 requires ARMv7 fabric".into()))?;
        if self.peripherals.iter().any(|p| {
            p.name == "i2c3" || (p.base < base + size && p.base.saturating_add(p.size) > base)
        }) {
            return Err(SimulationError::Other(
                "I2C3 name or MMIO window already mapped".into(),
            ));
        }
        if std::iter::once(&self.flash)
            .chain(std::iter::once(&self.ram))
            .chain(self.extra_mem.iter())
            .any(|m| {
                m.base_addr < base + size && m.base_addr.saturating_add(m.data.len() as u64) > base
            })
        {
            return Err(SimulationError::Other(
                "I2C3 MMIO window overlaps CPU memory".into(),
            ));
        }
        let gic = fabric.gic.clone();
        self.add_peripheral(
            "i2c3",
            base,
            size,
            None,
            Box::new(rk3x_i2c::Rk3xI2c::new(gic, irq as usize)),
        );
        self.recompute_walk_deletable();
        Ok(())
    }
    /// Opt in to the functional RV1103 UART4 slice after installing ARMv7.
    /// This does not alter the foundation boot tree or configure any external part.
    pub fn configure_armv7_rv1103_uart4(&mut self) -> SimResult<()> {
        self.configure_armv7_rv1103_uart4_from_descriptor(RV1103_DESCRIPTOR)
    }
    fn configure_armv7_rv1103_uart4_from_descriptor(&mut self, yaml: &str) -> SimResult<()> {
        let NativeBasketWindow { base, size, irq } = native_basket_windows(yaml)?[1];
        let fabric = self
            .irq_fabric
            .armv7
            .as_ref()
            .ok_or_else(|| SimulationError::Other("UART4 requires ARMv7 fabric".into()))?;
        if self.peripherals.iter().any(|p| {
            p.name == "uart4" || (p.base < base + size && p.base.saturating_add(p.size) > base)
        }) {
            return Err(SimulationError::Other(
                "UART4 name or MMIO window already mapped".into(),
            ));
        }
        if std::iter::once(&self.flash)
            .chain(std::iter::once(&self.ram))
            .chain(self.extra_mem.iter())
            .any(|m| {
                m.base_addr < base + size && m.base_addr.saturating_add(m.data.len() as u64) > base
            })
        {
            return Err(SimulationError::Other(
                "UART4 MMIO window overlaps CPU memory".into(),
            ));
        }
        let gic = fabric.gic.clone();
        let index = self.peripherals.len();
        self.add_peripheral(
            "uart4",
            base,
            size,
            None,
            Box::new(dw_uart::DwUart::new(gic, irq as usize)),
        );
        let fabric = self.irq_fabric.armv7.as_mut().unwrap();
        fabric.uart4_index = Some(index);
        fabric.uart4_irq = Some(irq);
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

#[cfg(test)]
mod basket_map_tests {
    use super::*;
    #[test]
    fn native_map_is_opt_in_and_matches_reviewed_windows() {
        let descriptor: labwired_config::ChipDescriptor =
            serde_yaml::from_str(RV1103_DESCRIPTOR).unwrap();
        assert!(descriptor.peripherals.is_empty());
        let windows = native_basket_windows(RV1103_DESCRIPTOR).unwrap();
        assert_eq!(
            windows[0],
            NativeBasketWindow {
                base: 0xff460000,
                size: 0x1000,
                irq: 53
            }
        );
        assert_eq!(
            windows[1],
            NativeBasketWindow {
                base: 0xff4e0000,
                size: 0x100,
                irq: 61
            }
        );
        let mut bus = crate::bus::SystemBus::new();
        bus.cpu_hz = 24_000_000;
        bus.configure_armv7_rv1103().unwrap();
        assert!(!bus
            .peripherals
            .iter()
            .any(|p| p.name == "i2c3" || p.name == "uart4"));
        bus.configure_armv7_rv1103_i2c3().unwrap();
        bus.configure_armv7_rv1103_uart4().unwrap();
        for (id, window) in ["i2c3", "uart4"].into_iter().zip(windows) {
            let p = bus.peripherals.iter().find(|p| p.name == id).unwrap();
            assert_eq!((p.base, p.size, p.irq), (window.base, window.size, None));
        }
        assert_eq!(
            bus.irq_fabric.armv7.as_ref().unwrap().uart4_irq,
            Some(windows[1].irq)
        );
    }
    #[test]
    fn invalid_native_map_fails_before_bus_mutation() {
        let bad = [
            RV1103_DESCRIPTOR.replace("type: rk3x_i2c", "type: gpio"),
            RV1103_DESCRIPTOR.replace("irq: 53", "missing_irq: 53"),
            RV1103_DESCRIPTOR.replace("native_basket_peripherals:", "missing_native_map:"),
            RV1103_DESCRIPTOR.replace("0xff460000", "0xfffffffffffffff0"),
            RV1103_DESCRIPTOR.replace("size: 4KiB", "size: 256B"),
            RV1103_DESCRIPTOR.replace("0xff4e0000", "0xff460000"),
            RV1103_DESCRIPTOR.replace("irq: 61", "irq: 53"),
        ];
        for yaml in bad {
            let mut bus = crate::bus::SystemBus::new();
            bus.cpu_hz = 24_000_000;
            bus.configure_armv7_rv1103().unwrap();
            let before: Vec<_> = bus
                .peripherals
                .iter()
                .map(|p| (p.name.clone(), p.base, p.size, p.irq))
                .collect();
            assert!(bus
                .configure_armv7_rv1103_i2c3_from_descriptor(&yaml)
                .is_err());
            assert!(bus
                .configure_armv7_rv1103_uart4_from_descriptor(&yaml)
                .is_err());
            assert_eq!(
                before,
                bus.peripherals
                    .iter()
                    .map(|p| (p.name.clone(), p.base, p.size, p.irq))
                    .collect::<Vec<_>>()
            );
            let fabric = bus.irq_fabric.armv7.as_ref().unwrap();
            assert_eq!((fabric.uart4_index, fabric.uart4_irq), (None, None));
        }
    }
}
