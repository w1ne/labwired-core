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
        self.ext.config.get(key).and_then(|v| {
            v.as_f64()
                .or_else(|| v.as_i64().map(|i| i as f64))
                .or_else(|| v.as_u64().map(|u| u as f64))
                .or_else(|| v.as_str().and_then(|s| s.trim().parse::<f64>().ok()))
        })
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
    /// **Tier 2**: resolve a declarative part's `outputs:` roles to pads and
    /// record them on the bus, so the per-tick drain knows where to put what
    /// the part's rules queued.
    ///
    /// The binding is the same shape a `pins:` role uses — role → `config:` key
    /// → pad label — with `behavior.output_pins` supplying the key when it is
    /// not simply the role name. An `outputs:` role whose config key the
    /// placement does not set is SKIPPED, not an error: a board that leaves an
    /// interrupt line unconnected is an ordinary board, and the part must still
    /// work over the bus. An unresolvable pad label IS an error — that is a
    /// wiring mistake, not a choice.
    pub fn bind_output_pins(&mut self, desc: &labwired_config::DeviceDescriptor) -> Result<()> {
        for role in &desc.behavior.outputs {
            let key = desc
                .behavior
                .output_pins
                .get(role)
                .cloned()
                .unwrap_or_else(|| role.clone());
            let Some(label) = self.ext.config.get(&key).and_then(|v| {
                v.as_str()
                    .map(|s| s.to_string())
                    .or_else(|| v.as_i64().map(|n| n.to_string()))
                    .or_else(|| v.as_u64().map(|n| n.to_string()))
            }) else {
                continue;
            };
            let (addr, bit) =
                SystemBus::resolve_pin_idr_pub(self.bus, &label).ok_or_else(|| {
                    anyhow!(
                    "device '{}' output pin '{}' ('{}' = {}) could not be resolved to a GPIO input",
                    self.ext.id,
                    role,
                    key,
                    label
                )
                })?;
            self.bus.device_pin_pads.push(crate::bus::DevicePinPad {
                device_id: self.ext.id.clone(),
                role: role.clone(),
                addr,
                bit,
            });
        }
        Ok(())
    }

    pub fn attach_i2c_device(&mut self, mut device: Box<dyn I2cDevice>) -> Result<()> {
        // Input devices get their system.yaml id stamped here (the ONE kit
        // attach path), so discovery and the stimulus resolver address them
        // by the name the author wrote (see crate::sim_input).
        if let Some(si) = device.as_sim_input_mut() {
            si.set_component_id(self.ext.id.clone());
        }
        // ⚠️ SUPPLY GATE — the ONE home for "this I²C part has no power".
        //
        // A diagram can wire a sensor's or a panel's SDA/SCL and nothing else,
        // and the twin used to run it and report readings and painted pixels
        // for a chip that on a bench is dead. The compiler now says so
        // (`powered: false`, emitted only when the part's declared `power_in`
        // pins are on no net); this is where the engine acts on it.
        //
        // It sits HERE, not in each of the 34 I²C models, because the honest
        // behaviour is identical for all of them and is a bus fact: an
        // unpowered slave does not pull SDA low, so its address NACKs and a
        // scan finds nothing. Every I²C kit — including the declarative ones
        // built from `configs/devices/*.yaml`, which have no per-device Rust to
        // patch — reaches the bus through this method, so no kit can be
        // forgotten and no future kit has to remember.
        //
        // ⚠️ ABSENT MEANS POWERED. See `components::supply` for why that
        // asymmetry is load-bearing: the curated labs declare no rails at all.
        if !crate::peripherals::components::supply::powered_from_config(self) {
            device =
                Box::new(crate::peripherals::components::supply::UnpoweredI2cDevice::new(device));
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

    /// Attach a serial-audio device to the USART/I2S block the `connection:`
    /// field names.
    ///
    /// Separate from `attach_spi_device` because the unit differs: an I2S
    /// device answers in 32-bit channel slots, not bytes. On EFR32 the same
    /// physical block does both, which is exactly why the two doors must stay
    /// distinct -- a mic attached through the SPI door would be asked for
    /// bytes and would have no way to say which channel they came from.
    pub fn attach_i2s_device(
        &mut self,
        device: Box<dyn crate::peripherals::device::I2sDevice>,
    ) -> Result<()> {
        let ext = self.ext;
        let idx = self
            .bus
            .find_peripheral_index_by_name(&ext.connection)
            .ok_or_else(|| missing_connection_err(ext))?;
        let any = self.bus.peripherals[idx]
            .dev
            .as_any_mut()
            .ok_or_else(|| downcast_err(ext))?;
        let spi = any
            .downcast_mut::<crate::peripherals::spi::Spi>()
            .ok_or_else(|| wrong_transport_err(ext, "I2S"))?;
        spi.i2s_device = Some(device);
        Ok(())
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

    /// Attach an analog stimulus source (potentiometer wiper, thermistor
    /// divider) to `channel` of whichever ADC the `connection:` field names.
    ///
    /// The model is *retained* on the bus rather than being used once to
    /// compute a boot level and dropped — that retention is what makes the
    /// part drivable at runtime through `set_input`. The current level is
    /// seeded immediately so the firmware sees a correct value before any
    /// stimulus arrives.
    pub fn attach_analog_source(
        &mut self,
        channel: u8,
        mut source: Box<dyn crate::bus::sim_inputs::AnalogSource>,
    ) -> Result<()> {
        // Same identity stamp as `attach_i2c_device`.
        source.set_component_id(self.ext.id.clone());
        let connection = self.ext.connection.clone();
        let mv = source.output_mv();
        if !self.bus.seed_adc_channel(&connection, channel, mv) {
            return Err(wrong_transport_err(self.ext, "ADC"));
        }
        self.bus
            .analog_inputs
            .push(crate::bus::sim_inputs::AnalogInputSource {
                connection,
                channel,
                source,
            });
        Ok(())
    }

    /// Resolve an STM32 pin label (e.g. `"PC7"`) to its `(ODR address, bit)`
    /// so a SPI display can sample the host's D/C line directly from the
    /// driving GPIO's output register. Returns None for unknown ports or
    /// pin labels.
    pub fn resolve_pin_odr(&self, pin: &str) -> Option<(u64, u8)> {
        SystemBus::resolve_pin_odr_pub(self.bus, pin)
    }

    /// Parse a GPIO pad label into a pin number for bit-bang devices
    /// (`Transport::GpioGroup`). Accepts ESP32/S3 spellings (`GPIO15`, `IO4`,
    /// bare `15`) used by the ESP GPIO edge-observer path.
    pub fn parse_gpio_pin(&self, label: &str) -> Option<u8> {
        SystemBus::parse_esp32s3_gpio_pin(label).or_else(|| SystemBus::parse_esp32_gpio_pin(label))
    }

    /// Read a GPIO pin config key (or alternate key / default label) as a pad
    /// number. Shared by every `GpioGroup` kit so pin parsing is not re-copied
    /// per device.
    pub fn config_gpio_pin(&self, key: &str, alt_key: &str, default: &str) -> Result<u8> {
        let label = self
            .ext
            .config
            .get(key)
            .or_else(|| self.ext.config.get(alt_key))
            .and_then(|v| v.as_str())
            .unwrap_or(default);
        self.parse_gpio_pin(label).ok_or_else(|| {
            anyhow!(
                "{} '{}': pin '{}' (config {}/{}) is not a parseable GPIO pad label",
                self.device_type(),
                self.device_id(),
                label,
                key,
                alt_key
            )
        })
    }

    /// Subscribe `observer` to GPIO edge notifications on the bus GPIO block
    /// (classic ESP32 + ESP32-S3 today). Kits use this instead of a hand arm
    /// in `from_config` so bit-bang devices share one attach path.
    pub fn install_gpio_observer<T>(&mut self, observer: std::sync::Arc<T>)
    where
        T: crate::peripherals::device::GpioObserver + 'static,
    {
        SystemBus::install_gpio_observer(self.bus, observer);
    }

    /// Hold an MCU input pin at `level` — for device status lines the host
    /// polls but nothing else drives (an e-paper BUSY, a sensor DRDY).
    ///
    /// Resolution goes through `resolve_pin_idr`, which understands the chip
    /// pin-map, STM32/Nordic pad labels and ESP `GPIO`n alike, so a kit gets
    /// this on every supported MCU without knowing which one it is wired to.
    ///
    /// This must go through the GPIO peripheral rather than an MMIO write:
    /// input registers ignore stores (that is what makes them inputs), so a
    /// bus write would be silently dropped.
    ///
    /// A line left undriven reads whatever the input register happens to hold,
    /// and a driver that waits on it then blocks until its timeout — which at
    /// simulated speed is effectively forever. That is not a hang to debug; it
    /// is a peripheral nobody modelled.
    pub fn drive_pin_input(&mut self, pin: &str, level: bool) -> Result<()> {
        let (device_type, device_id) =
            (self.device_type().to_string(), self.device_id().to_string());
        if !SystemBus::drive_pin_input(self.bus, pin, level) {
            anyhow::bail!(
                "{device_type} '{device_id}': pin '{pin}' could not be driven as a \
                 GPIO input (unresolvable pin, or the GPIO block refused it)"
            );
        }
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use labwired_config::ExternalDevice;
    use std::collections::{BTreeMap, HashMap};

    fn ext_with_moisture(value: serde_yaml::Value) -> ExternalDevice {
        let mut config = HashMap::new();
        config.insert("moisture".into(), value);
        ExternalDevice {
            id: "soil".into(),
            r#type: "soil-moisture".into(),
            connection: "adc1".into(),
            channel: None,
            route: BTreeMap::new(),
            config,
        }
    }

    #[test]
    fn config_f64_parses_string_moisture_25() {
        // Scene compilers sometimes emit channel seeds as YAML strings ("25")
        // rather than bare numbers. config_f64 must accept both.
        let mut bus = SystemBus::new();
        let ext = ext_with_moisture(serde_yaml::Value::String("25".into()));
        let ctx = AttachCtx::new(&mut bus, &ext);
        assert_eq!(ctx.config_f64("moisture"), Some(25.0));

        let ext = ext_with_moisture(serde_yaml::Value::from(25));
        let ctx = AttachCtx::new(&mut bus, &ext);
        assert_eq!(ctx.config_f64("moisture"), Some(25.0));
    }
}
