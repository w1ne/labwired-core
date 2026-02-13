// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::memory::LinearMemory;
use crate::peripherals::gpio::GpioRegisterLayout;
use crate::peripherals::nvic::NvicState;
use crate::peripherals::rcc::RccRegisterLayout;
use crate::peripherals::uart::Uart;
use crate::peripherals::uart::UartRegisterLayout;
use crate::{Bus, DmaRequest, Peripheral, SimResult, SimulationError};
use anyhow::Context;
use labwired_config::{parse_size, ChipDescriptor, PeripheralConfig, SystemManifest};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;

pub struct PeripheralEntry {
    pub name: String,
    pub base: u64,
    pub size: u64,
    pub irq: Option<u32>,
    pub dev: Box<dyn Peripheral>,
}

pub struct SystemBus {
    pub flash: LinearMemory,
    pub ram: LinearMemory,
    pub peripherals: Vec<PeripheralEntry>,
    pub nvic: Option<Arc<NvicState>>,
    pub observers: Vec<Arc<dyn crate::SimulationObserver>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeripheralTickCost {
    pub index: usize,
    pub cycles: u32,
}

impl Default for SystemBus {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemBus {
    fn profile_name<'a>(p_cfg: &'a PeripheralConfig) -> anyhow::Result<Option<&'a str>> {
        if let Some(value) = p_cfg.config.get("profile") {
            return value.as_str().map(Some).ok_or_else(|| {
                anyhow::anyhow!("Peripheral '{}' config.profile must be a string", p_cfg.id)
            });
        }
        if let Some(value) = p_cfg.config.get("register_layout") {
            return value.as_str().map(Some).ok_or_else(|| {
                anyhow::anyhow!(
                    "Peripheral '{}' config.register_layout must be a string",
                    p_cfg.id
                )
            });
        }
        Ok(None)
    }

    fn parse_profile_or_default<T>(
        p_cfg: &PeripheralConfig,
        peripheral_kind: &str,
    ) -> anyhow::Result<T>
    where
        T: FromStr<Err = String> + Default,
    {
        let Some(profile_name) = Self::profile_name(p_cfg)? else {
            return Ok(T::default());
        };
        T::from_str(profile_name).map_err(|e| {
            anyhow::anyhow!(
                "Peripheral '{}' has invalid {} profile '{}': {}",
                p_cfg.id,
                peripheral_kind,
                profile_name,
                e
            )
        })
    }

    fn resolve_peripheral_path(manifest: &SystemManifest, descriptor_path: &str) -> PathBuf {
        let raw = PathBuf::from(descriptor_path);
        if raw.is_absolute() || raw.exists() {
            return raw;
        }

        let chip_path = Path::new(&manifest.chip);
        let chip_dir = chip_path.parent().unwrap_or_else(|| Path::new("."));
        chip_dir.join(descriptor_path)
    }

    pub fn new() -> Self {
        // Default initialization for tests
        Self {
            flash: LinearMemory::new(1024 * 1024, 0x0),
            ram: LinearMemory::new(1024 * 1024, 0x2000_0000),
            peripherals: vec![
                PeripheralEntry {
                    name: "dma1".to_string(),
                    base: 0x4002_0000,
                    size: 0x400,
                    irq: None,
                    dev: Box::new(crate::peripherals::dma::Dma1::new()),
                },
                PeripheralEntry {
                    name: "afio".to_string(),
                    base: 0x4001_0000,
                    size: 0x400,
                    irq: None,
                    dev: Box::new(crate::peripherals::afio::Afio::new()),
                },
                PeripheralEntry {
                    name: "exti".to_string(),
                    base: 0x4001_0400,
                    size: 0x400,
                    irq: None,
                    dev: Box::new(crate::peripherals::exti::Exti::new()),
                },
                PeripheralEntry {
                    name: "systick".to_string(),
                    base: 0xE000_E010,
                    size: 0x10,
                    irq: Some(15),
                    dev: Box::new(crate::peripherals::systick::Systick::new()),
                },
                PeripheralEntry {
                    name: "uart1".to_string(),
                    base: 0x4000_C000,
                    size: 0x1000,
                    irq: None,
                    dev: Box::new(crate::peripherals::uart::Uart::new()),
                },
                PeripheralEntry {
                    name: "gpioa".to_string(),
                    base: 0x4001_0800,
                    size: 0x400,
                    irq: None,
                    dev: Box::new(crate::peripherals::gpio::GpioPort::new()),
                },
                PeripheralEntry {
                    name: "gpiob".to_string(),
                    base: 0x4001_0C00,
                    size: 0x400,
                    irq: None,
                    dev: Box::new(crate::peripherals::gpio::GpioPort::new()),
                },
                PeripheralEntry {
                    name: "gpioc".to_string(),
                    base: 0x4001_1000,
                    size: 0x400,
                    irq: None,
                    dev: Box::new(crate::peripherals::gpio::GpioPort::new()),
                },
                PeripheralEntry {
                    name: "rcc".to_string(),
                    base: 0x4002_1000,
                    size: 0x400,
                    irq: None,
                    dev: Box::new(crate::peripherals::rcc::Rcc::new()),
                },
                PeripheralEntry {
                    name: "tim2".to_string(),
                    base: 0x4000_0000,
                    size: 0x400,
                    irq: Some(28),
                    dev: Box::new(crate::peripherals::timer::Timer::new()),
                },
                PeripheralEntry {
                    name: "tim3".to_string(),
                    base: 0x4000_0400,
                    size: 0x400,
                    irq: Some(29),
                    dev: Box::new(crate::peripherals::timer::Timer::new()),
                },
                PeripheralEntry {
                    name: "i2c1".to_string(),
                    base: 0x4000_5400,
                    size: 0x400,
                    irq: Some(31),
                    dev: Box::new(crate::peripherals::i2c::I2c::new()),
                },
                PeripheralEntry {
                    name: "i2c2".to_string(),
                    base: 0x4000_5800,
                    size: 0x400,
                    irq: Some(33),
                    dev: Box::new(crate::peripherals::i2c::I2c::new()),
                },
                PeripheralEntry {
                    name: "spi1".to_string(),
                    base: 0x4001_3000,
                    size: 0x400,
                    irq: Some(35),
                    dev: Box::new(crate::peripherals::spi::Spi::new()),
                },
                PeripheralEntry {
                    name: "spi2".to_string(),
                    base: 0x4000_3800,
                    size: 0x400,
                    irq: Some(36),
                    dev: Box::new(crate::peripherals::spi::Spi::new()),
                },
            ],
            nvic: None,
            observers: Vec::new(),
        }
    }

    /// Attach a UART TX capture sink to any UART peripherals on this bus.
    ///
    /// When `echo_stdout` is false, UART writes will no longer be printed to stdout.
    pub fn attach_uart_tx_sink(&mut self, sink: Arc<Mutex<Vec<u8>>>, echo_stdout: bool) {
        for p in &mut self.peripherals {
            let Some(any) = p.dev.as_any_mut() else {
                continue;
            };
            let Some(uart) = any.downcast_mut::<Uart>() else {
                continue;
            };
            uart.set_sink(Some(sink.clone()), echo_stdout);
        }
    }

    pub fn from_config(chip: &ChipDescriptor, manifest: &SystemManifest) -> anyhow::Result<Self> {
        let flash_size = parse_size(&chip.flash.size)?;
        let ram_size = parse_size(&chip.ram.size)?;

        let mut bus = Self {
            flash: LinearMemory::new(flash_size as usize, chip.flash.base),
            ram: LinearMemory::new(ram_size as usize, chip.ram.base),
            peripherals: Vec::new(),
            nvic: None,
            observers: Vec::new(),
        };

        for p_cfg in &chip.peripherals {
            let dev: Box<dyn Peripheral> = match p_cfg.r#type.as_str() {
                "uart" => {
                    let layout: UartRegisterLayout = Self::parse_profile_or_default(p_cfg, "UART")?;
                    Box::new(crate::peripherals::uart::Uart::new_with_layout(layout))
                }
                "systick" => Box::new(crate::peripherals::systick::Systick::new()),
                "gpio" => {
                    let layout: GpioRegisterLayout = Self::parse_profile_or_default(p_cfg, "GPIO")?;
                    Box::new(crate::peripherals::gpio::GpioPort::new_with_layout(layout))
                }
                "rcc" => {
                    let layout: RccRegisterLayout = Self::parse_profile_or_default(p_cfg, "RCC")?;
                    Box::new(crate::peripherals::rcc::Rcc::new_with_layout(layout))
                }
                "timer" => Box::new(crate::peripherals::timer::Timer::new()),
                "i2c" => Box::new(crate::peripherals::i2c::I2c::new()),
                "spi" => Box::new(crate::peripherals::spi::Spi::new()),
                "exti" => Box::new(crate::peripherals::exti::Exti::new()),
                "afio" => Box::new(crate::peripherals::afio::Afio::new()),
                "dma" => Box::new(crate::peripherals::dma::Dma1::new()),
                "adc" => Box::new(crate::peripherals::adc::Adc::new()),
                "declarative" => {
                    let descriptor_path = p_cfg
                        .config
                        .get("path")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "Field 'path' is required in 'config' for declarative peripheral '{}'",
                                p_cfg.id
                            )
                        })?;

                    let resolved_path = Self::resolve_peripheral_path(manifest, descriptor_path);
                    let desc = labwired_config::PeripheralDescriptor::from_file(&resolved_path)
                        .with_context(|| {
                            format!(
                                "Failed to load declarative descriptor for '{}' from '{}' (resolved to '{}')",
                                p_cfg.id,
                                descriptor_path,
                                resolved_path.display()
                            )
                        })?;

                    Box::new(crate::peripherals::declarative::GenericPeripheral::new(
                        desc,
                    ))
                }
                "strict_ir" => {
                    let descriptor_path = p_cfg
                        .config
                        .get("path")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "Field 'path' is required in 'config' for strict_ir peripheral '{}'",
                                p_cfg.id
                            )
                        })?;

                    let resolved_path = Self::resolve_peripheral_path(manifest, descriptor_path);
                    let content = std::fs::read_to_string(&resolved_path).with_context(|| {
                        format!(
                            "Failed to read IR file '{}' (resolved to '{}')",
                            descriptor_path,
                            resolved_path.display()
                        )
                    })?;
                    let ir_peripheral = match serde_json::from_str::<labwired_ir::IrPeripheral>(
                        &content,
                    ) {
                        Ok(peripheral) => peripheral,
                        Err(peripheral_err) => {
                            let device: labwired_ir::IrDevice = serde_json::from_str(&content)
                                .with_context(|| {
                                    format!(
                                        "Failed to parse Strict IR from {} as IrPeripheral ({}) or IrDevice",
                                        resolved_path.display(),
                                        peripheral_err
                                    )
                                })?;

                            if let Some(peripheral) = device.peripherals.get(&p_cfg.id) {
                                peripheral.clone()
                            } else if device.peripherals.len() == 1 {
                                device
                                    .peripherals
                                    .into_values()
                                    .next()
                                    .expect("len() checked above")
                            } else {
                                let available = device
                                    .peripherals
                                    .keys()
                                    .cloned()
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                return Err(anyhow::anyhow!(
                                    "Strict IR '{}' contains multiple peripherals [{}]; no match for id '{}'",
                                    resolved_path.display(),
                                    available,
                                    p_cfg.id
                                ));
                            }
                        }
                    };

                    let desc: labwired_config::PeripheralDescriptor = ir_peripheral.into();

                    Box::new(crate::peripherals::declarative::GenericPeripheral::new(
                        desc,
                    ))
                }
                "strict_ir_internal" => {
                    let val = p_cfg.config.get("internal_ir_peripheral").ok_or_else(|| {
                        anyhow::anyhow!("Missing internal_ir_peripheral config for converted IR")
                    })?;
                    // Convert yaml Value (which was serde_yaml::to_value(p)) back to IrPeripheral
                    let ir_peripheral: labwired_ir::IrPeripheral =
                        serde_yaml::from_value(val.clone())?;
                    let desc: labwired_config::PeripheralDescriptor = ir_peripheral.into();

                    Box::new(crate::peripherals::declarative::GenericPeripheral::new(
                        desc,
                    ))
                }
                other => {
                    tracing::warn!(
                        "Unsupported peripheral type '{}' for id '{}'; skipping",
                        other,
                        p_cfg.id
                    );
                    continue;
                }
            };

            let dev = dev;
            // Stubbing out peripherals with external devices is deprecated.
            // For now, we keep the original peripheral.
            /*
            for ext in &_manifest.external_devices {
                if ext.connection == p_cfg.id {
                    tracing::info!("Stubbing {} on {}", ext.id, p_cfg.id);
                    dev = Box::new(crate::peripherals::stub::StubPeripheral::new(0x42));
                }
            }
            */

            // Map peripheral window size + IRQ from descriptor when provided.
            // Defaults keep older descriptors working.
            let size = if let Some(size) = &p_cfg.size {
                parse_size(size)?
            } else {
                0x1000 // Default 4KB page
            };

            let irq = if let Some(irq) = p_cfg.irq {
                Some(irq)
            } else if p_cfg.id == "systick" {
                Some(15)
            } else {
                None
            };

            bus.peripherals.push(PeripheralEntry {
                name: p_cfg.id.clone(),
                base: p_cfg.base_address,
                size,
                irq,
                dev,
            });
        }

        Ok(bus)
    }

    pub fn signal_nvic_irq(&self, irq: u32) {
        if let Some(nvic) = &self.nvic {
            if irq >= 16 {
                let idx = ((irq - 16) / 32) as usize;
                let bit = (irq - 16) % 32;
                if idx < 8 {
                    nvic.ispr[idx].fetch_or(1 << bit, Ordering::SeqCst);
                }
            } else {
                // Core exceptions are handled differently if needed,
                // but signal_nvic_irq is mostly for external IRQs.
                tracing::warn!("signal_nvic_irq called for core exception {}", irq);
            }
        }
    }

    pub fn read_u32(&self, addr: u64) -> SimResult<u32> {
        let b0 = self.read_u8(addr)? as u32;
        let b1 = self.read_u8(addr + 1)? as u32;
        let b2 = self.read_u8(addr + 2)? as u32;
        let b3 = self.read_u8(addr + 3)? as u32;
        Ok(b0 | (b1 << 8) | (b2 << 16) | (b3 << 24))
    }

    pub fn write_u32(&mut self, addr: u64, value: u32) -> SimResult<()> {
        self.write_u8(addr, (value & 0xFF) as u8)?;
        self.write_u8(addr + 1, ((value >> 8) & 0xFF) as u8)?;
        self.write_u8(addr + 2, ((value >> 16) & 0xFF) as u8)?;
        self.write_u8(addr + 3, ((value >> 24) & 0xFF) as u8)?;
        Ok(())
    }

    pub fn read_u16(&self, addr: u64) -> SimResult<u16> {
        let b0 = self.read_u8(addr)? as u16;
        let b1 = self.read_u8(addr + 1)? as u16;
        Ok(b0 | (b1 << 8))
    }

    pub fn write_u16(&mut self, addr: u64, value: u16) -> SimResult<()> {
        self.write_u8(addr, (value & 0xFF) as u8)?;
        self.write_u8(addr + 1, ((value >> 8) & 0xFF) as u8)?;
        Ok(())
    }

    fn tick_peripherals_phase1(&mut self) -> (Vec<u32>, Vec<PeripheralTickCost>, Vec<DmaRequest>) {
        let mut interrupts = Vec::new();
        let mut costs = Vec::new();
        let mut dma_requests = Vec::new();

        for (peripheral_index, p) in self.peripherals.iter_mut().enumerate() {
            let res = p.dev.tick();
            if res.cycles > 0 {
                costs.push(PeripheralTickCost {
                    index: peripheral_index,
                    cycles: res.cycles,
                });
            }

            if !res.dma_requests.is_empty() {
                dma_requests.extend(res.dma_requests);
            }

            if res.irq {
                if let Some(irq) = p.irq {
                    if irq >= 16 {
                        if let Some(nvic) = &self.nvic {
                            let idx = ((irq - 16) / 32) as usize;
                            let bit = (irq - 16) % 32;
                            if idx < 8 {
                                nvic.ispr[idx].fetch_or(1 << bit, Ordering::SeqCst);
                            }
                        } else {
                            interrupts.push(irq);
                        }
                    } else {
                        interrupts.push(irq);
                    }
                }
            }

            for irq in res.explicit_irqs {
                if let Some(nvic) = &self.nvic {
                    if irq >= 16 {
                        let idx = ((irq - 16) / 32) as usize;
                        let bit = (irq - 16) % 32;
                        if idx < 8 {
                            nvic.ispr[idx].fetch_or(1 << bit, Ordering::SeqCst);
                        }
                    } else {
                        interrupts.push(irq);
                    }
                } else {
                    interrupts.push(irq);
                }
            }
        }

        (interrupts, costs, dma_requests)
    }

    fn collect_enabled_nvic_interrupts(&self, interrupts: &mut Vec<u32>) {
        if let Some(nvic) = &self.nvic {
            for idx in 0..8 {
                let mask =
                    nvic.iser[idx].load(Ordering::SeqCst) & nvic.ispr[idx].load(Ordering::SeqCst);
                if mask != 0 {
                    for bit in 0..32 {
                        if (mask & (1 << bit)) != 0 {
                            let irq = 16 + (idx as u32 * 32) + bit;
                            interrupts.push(irq);
                        }
                    }
                }
            }
        }
    }

    pub fn tick_peripherals_with_costs(
        &mut self,
    ) -> (Vec<u32>, Vec<PeripheralTickCost>, Vec<DmaRequest>) {
        let (mut interrupts, costs, dma_requests) = self.tick_peripherals_phase1();
        self.collect_enabled_nvic_interrupts(&mut interrupts);

        (interrupts, costs, dma_requests)
    }

    pub fn tick_peripherals_fully(&mut self) -> (Vec<u32>, Vec<PeripheralTickCost>) {
        let (mut interrupts, costs, pending_dma) = self.tick_peripherals_phase1();

        // Phase 2: Execute DMA requests (this now has access to self.flash/ram via write_u8)
        for req in pending_dma {
            match req.direction {
                crate::DmaDirection::Read => {
                    // DMA Read from source
                    if let Ok(val) = self.read_u8(req.addr) {
                        // In a real DMA transfer, this value would be written somewhere else.
                        // For a generic DmaRequest, we might need a way to pass the value back to the peripheral.
                        // Let's refine DmaRequest later if needed.
                        tracing::trace!("DMA Read: {:#x} -> {:#x}", req.addr, val);
                    }
                }
                crate::DmaDirection::Write => {
                    // DMA Write to destination
                    let _ = self.write_u8(req.addr, req.val);
                    tracing::trace!("DMA Write: {:#x} <- {:#x}", req.addr, req.val);
                }
            }
        }

        // Phase 2.5: EXTI Logic Removed - moved to Exti peripheral via explicit_irqs.

        // Phase 3: Scan NVIC
        self.collect_enabled_nvic_interrupts(&mut interrupts);

        (interrupts, costs)
    }
}

impl crate::Bus for SystemBus {
    fn read_u8(&self, addr: u64) -> SimResult<u8> {
        if let Some(val) = self.ram.read_u8(addr) {
            return Ok(val);
        }
        if let Some(val) = self.flash.read_u8(addr) {
            return Ok(val);
        }
        // Cortex-M boot alias: address 0x0000_0000 mirrors flash start on many STM32 parts.
        // This lets reset-vector fetch work when flash is configured at 0x0800_0000.
        if self.flash.base_addr != 0 {
            let alias_end = self.flash.data.len() as u64;
            if addr < alias_end {
                if let Some(val) = self.flash.read_u8(self.flash.base_addr + addr) {
                    return Ok(val);
                }
            }
        }

        // Dynamic Peripherals
        for p in &self.peripherals {
            if addr >= p.base && addr < p.base + p.size {
                return p.dev.read(addr - p.base);
            }
        }

        Err(SimulationError::MemoryViolation(addr))
    }

    fn write_u8(&mut self, addr: u64, value: u8) -> SimResult<()> {
        let flash_alias_old = if self.flash.base_addr != 0 && addr < self.flash.data.len() as u64 {
            self.flash.read_u8(self.flash.base_addr + addr)
        } else {
            None
        };

        // Avoid calling `read_u8` here since peripheral reads may carry side effects.
        let old_value = self
            .ram
            .read_u8(addr)
            .or_else(|| self.flash.read_u8(addr))
            .or(flash_alias_old)
            .or_else(|| {
                self.peripherals
                    .iter()
                    .find(|p| addr >= p.base && addr < p.base + p.size)
                    .and_then(|p| p.dev.peek(addr - p.base))
            })
            .unwrap_or(0);

        let flash_alias_write = self.flash.base_addr != 0
            && addr < self.flash.data.len() as u64
            && self.flash.write_u8(self.flash.base_addr + addr, value);

        let res = if self.ram.write_u8(addr, value)
            || self.flash.write_u8(addr, value)
            || flash_alias_write
        {
            Ok(())
        } else {
            // Dynamic Peripherals
            let mut found = false;
            let mut p_res = Ok(());
            for p in &mut self.peripherals {
                if addr >= p.base && addr < p.base + p.size {
                    p_res = p.dev.write(addr - p.base, value);
                    found = true;
                    break;
                }
            }
            if found {
                p_res
            } else {
                Err(SimulationError::MemoryViolation(addr))
            }
        };

        if res.is_ok() {
            // Trigger observers
            for observer in &self.observers {
                observer.on_memory_write(addr, old_value, value);
            }
        }

        res
    }

    fn tick_peripherals(&mut self) -> Vec<u32> {
        let (interrupts, _costs, dma_requests) = self.tick_peripherals_with_costs();

        // Execute DMA requests
        if !dma_requests.is_empty() {
            let _ = self.execute_dma(&dma_requests);
        }

        interrupts
    }

    fn execute_dma(&mut self, requests: &[DmaRequest]) -> SimResult<()> {
        for req in requests {
            match req.direction {
                crate::DmaDirection::Read => {
                    // Note: In a real system, the DMA controller reads into its internal register.
                    // Here we just verify the read is valid for now, or we could pass the value back.
                    // For STM32 DMA, it's usually memory-to-peripheral or peripheral-to-memory.
                    let _ = self.read_u8(req.addr)?;
                }
                crate::DmaDirection::Write => {
                    self.write_u8(req.addr, req.val)?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use labwired_config::{ChipDescriptor, SystemManifest};
    use std::path::PathBuf;

    #[test]
    fn test_system_bus_from_config_declarative() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let chip_path = root.join("tests/fixtures/test_chip_declarative.yaml");
        let manifest_path = root.join("tests/fixtures/test_system_declarative.yaml");

        let chip = ChipDescriptor::from_file(&chip_path).unwrap();
        let manifest = SystemManifest::from_file(&manifest_path).unwrap();

        let bus =
            SystemBus::from_config(&chip, &manifest).expect("Failed to create bus from config");

        // Verify TIMER1 is present at 0x40001000
        let found = bus
            .peripherals
            .iter()
            .find(|p| p.name == "TIMER1")
            .expect("TIMER1 not found");
        assert_eq!(found.base, 0x40001000);
        assert_eq!(found.size, 1024);

        // Verify we can read/write to it through the bus
        // Address 0x40001000 + 0x00 = CTRL register (reset value 0)
        let ctrl_val = bus.read_u32(0x40001000).unwrap();
        assert_eq!(ctrl_val, 0);

        // Address 0x40001000 + 0x04 = COUNT register
        let mut bus = bus;
        bus.write_u32(0x40001004, 0x12345678).unwrap();
        let count_val = bus.read_u32(0x40001004).unwrap();
        assert_eq!(count_val, 0x12345678);
    }

    #[test]
    fn test_system_bus_resolves_descriptor_path_relative_to_chip_file() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let chip_path = root.join("tests/fixtures/test_chip_declarative.yaml");
        let manifest_path = root.join("tests/fixtures/test_system_declarative.yaml");

        let mut chip = ChipDescriptor::from_file(&chip_path).unwrap();
        let mut manifest = SystemManifest::from_file(&manifest_path).unwrap();

        // Simulate a descriptor path that is relative to chip.yaml location.
        if let Some(path) = chip.peripherals[0].config.get_mut("path") {
            *path = serde_yaml::Value::String("test_timer_descriptor.yaml".to_string());
        }
        manifest.chip = chip_path.to_string_lossy().into_owned();

        let bus =
            SystemBus::from_config(&chip, &manifest).expect("Failed to create bus from config");

        let found = bus
            .peripherals
            .iter()
            .find(|p| p.name == "TIMER1")
            .expect("TIMER1 not found");
        assert_eq!(found.base, 0x40001000);
    }

    #[test]
    fn test_system_bus_memory_observer() {
        use std::sync::Arc;
        use std::sync::Mutex;

        #[derive(Debug)]
        struct MockObserver {
            writes: Arc<Mutex<Vec<(u64, u8, u8)>>>,
        }

        impl crate::SimulationObserver for MockObserver {
            fn on_memory_write(&self, addr: u64, old: u8, new: u8) {
                self.writes.lock().unwrap().push((addr, old, new));
            }
        }

        let writes = Arc::new(Mutex::new(Vec::new()));
        let mut bus = SystemBus::new();
        bus.observers.push(Arc::new(MockObserver {
            writes: writes.clone(),
        }));

        // Write to RAM (e.g., 0x20000000)
        bus.write_u8(0x20000000, 0xAA).unwrap();
        {
            let w = writes.lock().unwrap();
            assert_eq!(w.len(), 1);
            assert_eq!(w[0], (0x20000000, 0, 0xAA));
        }

        // Write to Peripheral (e.g., UART at 0x4000C000)
        bus.write_u8(0x4000C000, 0xBB).unwrap();
        {
            let w = writes.lock().unwrap();
            assert_eq!(w.len(), 2);
            assert_eq!(w[1], (0x4000C000, 0xC0, 0xBB));
        }
    }

    #[test]
    fn test_flash_boot_alias_read_and_write() {
        let mut bus = SystemBus {
            flash: LinearMemory::new(256, 0x0800_0000),
            ram: LinearMemory::new(256, 0x2000_0000),
            peripherals: Vec::new(),
            nvic: None,
            observers: Vec::new(),
        };

        bus.flash.write_u8(0x0800_0000, 0x12);
        bus.flash.write_u8(0x0800_0001, 0x34);

        // Read through aliased 0x0000_0000 boot window.
        assert_eq!(bus.read_u8(0x0000_0000).unwrap(), 0x12);
        assert_eq!(bus.read_u8(0x0000_0001).unwrap(), 0x34);

        // Write through alias and verify backing flash changed.
        bus.write_u8(0x0000_0001, 0xAB).unwrap();
        assert_eq!(bus.flash.read_u8(0x0800_0001), Some(0xAB));
    }
}
