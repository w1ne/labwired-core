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
use crate::peripherals::i2c::I2c;
use crate::peripherals::spi::Spi;
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
