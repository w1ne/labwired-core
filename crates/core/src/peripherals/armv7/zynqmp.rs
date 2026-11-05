//! Zynq UltraScale+ MPSoC RPU (Cortex-R5F) platform slice: the RPU GIC
//! (PL390, GICv1 register subset served by the GICv2 model), PS UART0/UART1
//! (Cadence) and TTC0. The map (bases, sizes, INTIDs, TTC input clock) is
//! owned by the chip descriptor's `native_rpu_peripherals`
//! (`configs/chips/xck26-rpu.yaml`). IPI mailboxes, the other TTCs, GPIO,
//! QSPI and the PMU firmware interface are not installed.
use super::{cadence_uart::CadenceUart, gic, ttc, Armv7Fabric};
use crate::{SimResult, SimulationError};
use std::sync::{Arc, Mutex};

/// The XCK26 RPU descriptor, for callers without a resolved chip YAML.
pub(crate) const XCK26_RPU_DESCRIPTOR: &str =
    include_str!("../../../../../configs/chips/xck26-rpu.yaml");

#[derive(serde::Deserialize)]
struct NativeRpuDescriptor {
    native_rpu_peripherals: Vec<labwired_config::PeripheralConfig>,
}

/// One validated RPU window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RpuWindow {
    pub base: u64,
    pub size: u64,
    pub intid: Option<u32>,
}

/// The validated RPU platform map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpuMap {
    pub gicd: RpuWindow,
    pub gicc: RpuWindow,
    pub uarts: Vec<(String, RpuWindow)>,
    pub ttc0: RpuWindow,
    pub ttc_input_hz: u64,
}

/// Parse and validate `native_rpu_peripherals` from a chip descriptor.
/// Every entry is checked before the caller touches the bus.
pub fn rpu_map(yaml: &str) -> SimResult<RpuMap> {
    let descriptor: NativeRpuDescriptor = serde_yaml::from_str(yaml)
        .map_err(|e| SimulationError::Other(format!("ZynqMP RPU descriptor: {e}")))?;
    let invalid = |why: &str| SimulationError::Other(format!("ZynqMP RPU map: {why}"));
    let mut gicd = None;
    let mut gicc = None;
    let mut ttc0 = None;
    let mut ttc_input_hz = None;
    let mut uarts = Vec::new();
    for entry in &descriptor.native_rpu_peripherals {
        let size = entry
            .size
            .as_deref()
            .and_then(|s| labwired_config::parse_size(s).ok())
            .filter(|&s| s == 0x1000)
            .ok_or_else(|| invalid(&format!("{} must declare a 4KiB window", entry.id)))?;
        let base = entry.base_address;
        if base == 0 || base % 0x1000 != 0 || base + size > 1 << 32 {
            return Err(invalid(&format!("{} base {base:#x}", entry.id)));
        }
        if entry.irq.is_some_and(|irq| !(32..1020).contains(&irq)) {
            return Err(invalid(&format!("{} INTID must be an SPI", entry.id)));
        }
        let window = RpuWindow {
            base,
            size,
            intid: entry.irq,
        };
        let has_irq = entry.irq.is_some();
        let slot = match entry.r#type.as_str() {
            "arm_gic_distributor" if !has_irq => &mut gicd,
            "arm_gic_cpu_interface" if !has_irq => &mut gicc,
            "cadence_ttc" if has_irq && entry.id == "ttc0" => {
                ttc_input_hz = entry
                    .config
                    .get("input_hz")
                    .and_then(|v| v.as_u64())
                    .filter(|&hz| hz > 0);
                &mut ttc0
            }
            "cadence_uart" if has_irq => {
                uarts.push((entry.id.clone(), window));
                continue;
            }
            other => {
                return Err(invalid(&format!(
                    "unsupported entry {} ({other})",
                    entry.id
                )))
            }
        };
        if slot.replace(window).is_some() {
            return Err(invalid(&format!("duplicate {}", entry.r#type)));
        }
    }
    let map = RpuMap {
        gicd: gicd.ok_or_else(|| invalid("missing GIC distributor"))?,
        gicc: gicc.ok_or_else(|| invalid("missing GIC CPU interface"))?,
        uarts,
        ttc0: ttc0.ok_or_else(|| invalid("missing ttc0"))?,
        ttc_input_hz: ttc_input_hz.ok_or_else(|| invalid("ttc0 needs config.input_hz"))?,
    };
    let mut windows: Vec<_> = [map.gicd, map.gicc, map.ttc0]
        .into_iter()
        .chain(map.uarts.iter().map(|(_, w)| *w))
        .collect();
    windows.sort_by_key(|w| w.base);
    if windows
        .windows(2)
        .any(|p| p[0].base + p[0].size > p[1].base)
    {
        return Err(invalid("overlapping windows"));
    }
    let mut ids: Vec<u32> = windows.iter().filter_map(|w| w.intid).collect();
    // TTC0 owns three consecutive SPIs, one per counter.
    let t = map.ttc0.intid.unwrap();
    ids.extend([t + 1, t + 2]);
    ids.sort_unstable();
    if ids.windows(2).any(|p| p[0] == p[1]) || t + 2 >= 1020 {
        return Err(invalid("INTIDs collide"));
    }
    Ok(map)
}

impl crate::bus::SystemBus {
    /// Install the XCK26 RPU platform from the in-tree descriptor.
    pub fn configure_armv7_zynqmp_rpu(&mut self) -> SimResult<()> {
        self.configure_armv7_zynqmp_rpu_from_descriptor(XCK26_RPU_DESCRIPTOR)
    }
    /// Install the RPU platform models from a chip descriptor's
    /// `native_rpu_peripherals`. The caller supplies `cpu_hz` and memory; this
    /// installs no boot image. A refused map leaves the bus unchanged.
    pub fn configure_armv7_zynqmp_rpu_from_descriptor(&mut self, yaml: &str) -> SimResult<()> {
        if self.cpu_hz == 0 {
            return Err(SimulationError::Other(
                "ZynqMP RPU requires nonzero cpu_hz".into(),
            ));
        }
        if self.irq_fabric.armv7.is_some() {
            return Err(SimulationError::Other(
                "ARMv7 fabric already installed".into(),
            ));
        }
        let map = rpu_map(yaml)?;
        let mut f = Armv7Fabric::default();
        let t = map.ttc0.intid.unwrap() as usize;
        let timer = Arc::new(Mutex::new(ttc::Ttc::new(
            f.gic.clone(),
            [t, t + 1, t + 2],
            self.cycle_clock.clone(),
            self.cpu_hz,
            map.ttc_input_hz,
        )?));
        f.ttc = Some(timer.clone());
        f.synchronize(self.current_cycle, self.cpu_hz);
        for (name, window, cpu) in [("gicd", map.gicd, false), ("gicc", map.gicc, true)] {
            self.add_peripheral(
                name,
                window.base,
                window.size,
                None,
                Box::new(gic::GicWindow {
                    controller: f.gic.clone(),
                    cpu,
                }),
            );
        }
        for (name, window) in &map.uarts {
            let intid = window.intid.unwrap();
            f.level_sources.push((self.peripherals.len(), intid));
            self.add_peripheral(
                name,
                window.base,
                window.size,
                None,
                Box::new(CadenceUart::new(f.gic.clone(), intid as usize)),
            );
        }
        self.add_peripheral(
            "ttc0",
            map.ttc0.base,
            map.ttc0.size,
            None,
            Box::new(ttc::TtcWindow(timer)),
        );
        self.irq_fabric.armv7 = Some(f);
        self.rebuild_peripheral_ranges();
        self.recompute_walk_deletable();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Bus;
    fn bus() -> crate::bus::SystemBus {
        let mut bus = crate::bus::SystemBus::new();
        bus.peripherals.clear();
        bus.cpu_hz = 500_000_000;
        bus
    }
    #[test]
    fn descriptor_map_matches_zephyr_kv260_dt() {
        let map = rpu_map(XCK26_RPU_DESCRIPTOR).unwrap();
        assert_eq!((map.gicd.base, map.gicc.base), (0xf900_0000, 0xf900_1000));
        assert_eq!(
            map.uarts,
            vec![
                (
                    "uart0".into(),
                    RpuWindow {
                        base: 0xff00_0000,
                        size: 0x1000,
                        intid: Some(53)
                    }
                ),
                (
                    "uart1".into(),
                    RpuWindow {
                        base: 0xff01_0000,
                        size: 0x1000,
                        intid: Some(54)
                    }
                ),
            ]
        );
        assert_eq!((map.ttc0.base, map.ttc0.intid), (0xff11_0000, Some(68)));
        assert_eq!(map.ttc_input_hz, 5_000_000);
    }
    #[test]
    fn invalid_maps_fail_before_bus_mutation() {
        let bad = [
            XCK26_RPU_DESCRIPTOR.replace("native_rpu_peripherals:", "missing:"),
            XCK26_RPU_DESCRIPTOR.replace("type: cadence_ttc", "type: gpio"),
            XCK26_RPU_DESCRIPTOR.replace("input_hz: 5000000", "input_hz: 0"),
            XCK26_RPU_DESCRIPTOR.replace("irq: 54", "irq: 53"),
            XCK26_RPU_DESCRIPTOR.replace("irq: 54", "irq: 69"),
            XCK26_RPU_DESCRIPTOR.replace("0xff010000", "0xff000000"),
            XCK26_RPU_DESCRIPTOR.replace(
                "base_address: 0xf9001000, size: 4KiB",
                "base_address: 0xf9001000, size: 256B",
            ),
            XCK26_RPU_DESCRIPTOR.replace("irq: 53", "irq: 21"),
        ];
        for yaml in bad {
            let mut b = bus();
            assert!(
                b.configure_armv7_zynqmp_rpu_from_descriptor(&yaml).is_err(),
                "{yaml}"
            );
            assert!(b.peripherals.is_empty());
            assert!(b.irq_fabric.armv7.is_none());
        }
    }
    #[test]
    fn rpu_map_installs_and_ttc_match_raises_gic_irq() {
        let mut bus = bus();
        bus.configure_armv7_zynqmp_rpu().unwrap();
        assert!(bus.configure_armv7_zynqmp_rpu().is_err());
        for name in ["gicd", "gicc", "uart0", "uart1", "ttc0"] {
            assert!(bus.peripherals.iter().any(|p| p.name == name), "{name}");
        }
        // UART1 reset CR through the bus.
        assert_eq!(bus.read_u32(0xff01_0000).unwrap(), 0x128);
        // Enable distributor + CPU interface, SPI 36 (INTID 68), PMR open.
        bus.write_u32(0xf900_0000, 1).unwrap();
        bus.write_u32(0xf900_1000, 1).unwrap();
        bus.write_u32(0xf900_1004, 0xf0).unwrap();
        bus.write_u32(0xf900_0100 + 8, 1 << 4).unwrap();
        // TTC0 counter 0: match 0 at 10 ticks, IER match 0, start.
        bus.write_u32(0xff11_0030, 10).unwrap();
        bus.write_u32(0xff11_0060, 2).unwrap();
        bus.write_u32(0xff11_000c, 0x08).unwrap();
        assert_eq!(bus.next_cpu_interrupt_deadline_cycle(), Some(1000));
        assert_eq!(bus.arm_irq_fiq_levels(), Some((false, false)));
        bus.set_current_cycle(1000);
        assert_eq!(bus.arm_irq_fiq_levels(), Some((true, false)));
        // Acknowledge via clear-on-read ISR: level drops.
        assert_eq!(bus.read_u32(0xff11_0054).unwrap(), 2);
        assert_eq!(bus.arm_irq_fiq_levels(), Some((false, false)));
    }
}
