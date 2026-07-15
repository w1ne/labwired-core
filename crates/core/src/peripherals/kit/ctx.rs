// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Bus-attachment context handed to each kit's `attach` method.
//!
//! Centralises the find-connection / downcast / config-parse boilerplate
//! that every hand-written `bus/mod.rs` arm used to repeat. A kit calls
//! `ctx.uart()?` / `ctx.spi()?` / `ctx.i2c()?` to acquire the typed
//! peripheral handle, and `ctx.config_str("apn")` etc. to read its YAML
//! config — same `serde_yaml::Value` accessors the legacy arms used.

use anyhow::{anyhow, Result};
use labwired_config::ExternalDevice;

use crate::bus::SystemBus;
use crate::peripherals::adc::Adc;
use crate::peripherals::i2c::{I2c, I2cDevice};
use crate::peripherals::spi::{Spi, SpiDevice};
use crate::peripherals::uart::Uart;

pub struct AttachCtx<'a> {
    pub bus: &'a mut SystemBus,
    pub ext: &'a ExternalDevice,
}

impl<'a> AttachCtx<'a> {
    pub fn new(bus: &'a mut SystemBus, ext: &'a ExternalDevice) -> Self {
        Self { bus, ext }
    }

    pub fn device_type(&self) -> &str {
        self.ext.r#type.as_str()
    }
    pub fn device_id(&self) -> &str {
        self.ext.id.as_str()
    }
    pub fn connection(&self) -> &str {
        self.ext.connection.as_str()
    }

    pub fn config_str(&self, key: &str) -> Option<&str> {
        self.ext.config.get(key).and_then(|v| v.as_str())
    }
    pub fn config_bool(&self, key: &str) -> Option<bool> {
        self.ext.config.get(key).and_then(|v| v.as_bool())
    }
    pub fn config_i64(&self, key: &str) -> Option<i64> {
        self.ext.config.get(key).and_then(|v| v.as_i64())
    }
    pub fn config_f64(&self, key: &str) -> Option<f64> {
        self.ext.config.get(key).and_then(|v| v.as_f64())
    }

    pub fn uart(&mut self) -> Result<&mut Uart> {
        let ext = self.ext;
        let idx = self
            .bus
            .find_peripheral_index_by_name(&ext.connection)
            .ok_or_else(|| missing_connection_err(ext))?;
        let any = self.bus.peripherals[idx]
            .dev
            .as_any_mut()
            .ok_or_else(|| downcast_err(ext))?;
        any.downcast_mut::<Uart>()
            .ok_or_else(|| wrong_transport_err(ext, "UART"))
    }

    pub fn spi(&mut self) -> Result<&mut Spi> {
        let ext = self.ext;
        let idx = self
            .bus
            .find_peripheral_index_by_name(&ext.connection)
            .ok_or_else(|| missing_connection_err(ext))?;
        let any = self.bus.peripherals[idx]
            .dev
            .as_any_mut()
            .ok_or_else(|| downcast_err(ext))?;
        any.downcast_mut::<Spi>()
            .ok_or_else(|| wrong_transport_err(ext, "SPI"))
    }

    pub fn i2c(&mut self) -> Result<&mut I2c> {
        let ext = self.ext;
        let idx = self
            .bus
            .find_peripheral_index_by_name(&ext.connection)
            .ok_or_else(|| missing_connection_err(ext))?;
        let any = self.bus.peripherals[idx]
            .dev
            .as_any_mut()
            .ok_or_else(|| downcast_err(ext))?;
        any.downcast_mut::<I2c>()
            .ok_or_else(|| wrong_transport_err(ext, "I2C"))
    }

    /// Attach an [`I2cDevice`] slave to whichever I²C controller the
    /// `connection:` field resolves to — the STM32 `I2c` enum *or* the
    /// ESP32-C3 `Esp32c3I2c` command-list controller. The two controllers
    /// expose different attach methods (`attach` vs `attach_slave`), so a kit
    /// that called `ctx.i2c()?.attach(...)` directly would only work on STM32
    /// buses. Going through this method lets one kit serve a sensor on either
    /// family without caring which bus the system.yaml wired it to.
    pub fn attach_i2c_device(&mut self, mut device: Box<dyn I2cDevice>) -> Result<()> {
        // Input devices get their system.yaml id stamped here (the ONE kit
        // attach path), so discovery and the stimulus resolver address them
        // by the name the author wrote (see crate::sim_input).
        if let Some(si) = device.as_sim_input_mut() {
            si.set_component_id(self.ext.id.clone());
        }
        // Funnel through the single bus choke point, which wraps the device in
        // the shared bus trace before handing it to whichever I²C controller the
        // `connection:` resolves to. There is no untraced attach path.
        let connection = self.ext.connection.clone();
        self.bus
            .attach_i2c_slave_with_route(&connection, device, Some(&self.ext.route))
            .map_err(|err| anyhow::anyhow!("{}: {err:#}", wrong_transport_err(self.ext, "I2C")))
    }

    /// Attach an [`SpiDevice`] to whichever SPI controller the `connection:`
    /// field resolves to: the generic STM32-style [`Spi`] or the ESP32-C3 GP-SPI
    /// model. This mirrors [`Self::attach_i2c_device`] for mixed-controller
    /// systems.
    pub fn attach_spi_device(&mut self, mut device: Box<dyn SpiDevice>) -> Result<()> {
        // Same identity stamp as `attach_i2c_device`.
        if let Some(si) = device.as_sim_input_mut() {
            si.set_component_id(self.ext.id.clone());
        }
        // Funnel through the single bus choke point (see `attach_i2c_device`).
        let connection = self.ext.connection.clone();
        self.bus
            .attach_spi_device(&connection, device)
            .map_err(|_| wrong_transport_err(self.ext, "SPI"))
    }

    /// Acquire the ADC peripheral declared in the system.yaml `connection:`
    /// field. Used by analog peripherals (e.g. NTC thermistor) that "seed"
    /// a channel rather than attach a stream/device.
    pub fn adc(&mut self) -> Result<&mut Adc> {
        let ext = self.ext;
        let idx = self
            .bus
            .find_peripheral_index_by_name(&ext.connection)
            .ok_or_else(|| missing_connection_err(ext))?;
        let any = self.bus.peripherals[idx]
            .dev
            .as_any_mut()
            .ok_or_else(|| downcast_err(ext))?;
        any.downcast_mut::<Adc>()
            .ok_or_else(|| wrong_transport_err(ext, "ADC"))
    }

    /// Resolve an STM32 pin label (e.g. `"PC7"`) to its `(ODR address, bit)`
    /// so a SPI display can sample the host's D/C line directly from the
    /// driving GPIO's output register. Returns None for unknown ports or
    /// pin labels.
    pub fn resolve_pin_odr(&self, pin: &str) -> Option<(u64, u8)> {
        SystemBus::resolve_pin_odr_pub(self.bus, pin)
    }

    /// Read the optional `i2c_address` config key, returning `default` when
    /// absent. Rejects non-integer values and any address outside the 7-bit
    /// range — same validation every legacy hand-written I2C bus arm did,
    /// hoisted here so every I2C kit gets it for free.
    pub fn i2c_address_or(&self, default: u8) -> Result<u8> {
        let Some(value) = self.ext.config.get("i2c_address") else {
            return Ok(default);
        };
        let Some(address) = value.as_u64() else {
            return Err(anyhow!(
                "External device '{}' type '{}' on connection '{}' has invalid i2c_address '{}'",
                self.ext.id,
                self.ext.r#type,
                self.ext.connection,
                serde_yaml::to_string(value)
                    .unwrap_or_else(|_| "<unprintable>".to_string())
                    .trim()
            ));
        };
        if address > 0x7f {
            return Err(anyhow!(
                "External device '{}' type '{}' on connection '{}' has out-of-range 7-bit i2c_address 0x{:x}",
                self.ext.id,
                self.ext.r#type,
                self.ext.connection,
                address
            ));
        }
        Ok(address as u8)
    }
}

fn missing_connection_err(ext: &ExternalDevice) -> anyhow::Error {
    anyhow!(
        "External device '{}' type '{}' references missing connection '{}'",
        ext.id,
        ext.r#type,
        ext.connection
    )
}
fn downcast_err(ext: &ExternalDevice) -> anyhow::Error {
    anyhow!(
        "External device '{}' type '{}' connection '{}' cannot be downcast",
        ext.id,
        ext.r#type,
        ext.connection
    )
}
fn wrong_transport_err(ext: &ExternalDevice, expected: &str) -> anyhow::Error {
    anyhow!(
        "External device '{}' type '{}' connection '{}' is not a {} peripheral",
        ext.id,
        ext.r#type,
        ext.connection,
        expected
    )
}
