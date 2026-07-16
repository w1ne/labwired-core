// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! I2C/SPI attach funnels and chip pad wiring.

use super::*;

impl SystemBus {
    /// Attach an I²C slave without a physical route. This remains suitable for
    /// fixed-pin controllers and low-level test fixtures; ESP32-C3 rejects it
    /// because C3's GPIO matrix makes a controller-only binding ambiguous.
    pub fn attach_i2c_slave(
        &mut self,
        controller: &str,
        dev: Box<dyn crate::peripherals::i2c::I2cDevice>,
    ) -> anyhow::Result<()> {
        self.attach_i2c_slave_with_route(controller, dev, None)
    }

    /// The single funnel through which every manifest-backed I²C slave reaches
    /// a controller. `route` is a target-neutral signal map (`sda`/`scl` for
    /// I²C); ESP32-C3 lowers it to real GPIO-matrix pads and rejects missing,
    /// unsupported, or ambiguous routes instead of silently attaching by bus
    /// name alone. Other controller families preserve the generic shape for
    /// forward-compatible physical routing while retaining their fixed-pin
    /// behavior today.
    pub fn attach_i2c_slave_with_route(
        &mut self,
        controller: &str,
        dev: Box<dyn crate::peripherals::i2c::I2cDevice>,
        route: Option<&std::collections::BTreeMap<String, String>>,
    ) -> anyhow::Result<()> {
        let wrapped = bus_trace::wrap_i2c(controller, &self.bus_trace, dev);
        let idx = self
            .find_peripheral_index_by_name(controller)
            .ok_or_else(|| anyhow::anyhow!("attach_i2c_slave: no peripheral '{controller}'"))?;
        let any = self.peripherals[idx].dev.as_any_mut().ok_or_else(|| {
            anyhow::anyhow!("attach_i2c_slave: '{controller}' is not downcastable")
        })?;
        if let Some(c) = any.downcast_mut::<crate::peripherals::i2c::I2c>() {
            c.push_slave(wrapped);
        } else if let Some(c) = any.downcast_mut::<crate::peripherals::esp32c3::i2c::Esp32c3I2c>() {
            let route = route.ok_or_else(|| {
                anyhow::anyhow!(
                    "ESP32-C3 I2C external device on '{controller}' requires both route.sda and route.scl"
                )
            })?;
            let route =
                crate::peripherals::esp32c3::i2c::C3I2cPadRoute::from_manifest_route(route)?;
            c.push_slave_with_route(wrapped, route);
        } else if let Some(c) = any.downcast_mut::<crate::peripherals::esp32s3::i2c::Esp32s3I2c>() {
            c.push_slave(wrapped);
        } else if let Some(c) = any.downcast_mut::<crate::peripherals::esp32::i2c::Esp32I2c>() {
            c.push_slave(wrapped);
        } else if let Some(c) = any.downcast_mut::<crate::peripherals::nrf52::twim::Nrf52Twim>() {
            c.push_slave(wrapped);
        } else {
            anyhow::bail!("attach_i2c_slave: '{controller}' is not an I2C controller");
        }
        Ok(())
    }

    /// Wire the ESP32-C3 I²C0 bit engine to C3 GPIO in both directions: GPIO
    /// reads the live SDA/SCL waveform, while I²C reads GPIO's live input/output
    /// matrix state before allowing a physically routed slave to acknowledge.
    /// No-op unless both C3 models are on the bus.
    pub(crate) fn wire_esp32c3_i2c_pads(&mut self) {
        use crate::peripherals::esp32c3::gpio::Esp32c3Gpio;
        use crate::peripherals::esp32c3::i2c::Esp32c3I2c;
        let i2c_idx = self.peripherals.iter().position(|p| {
            p.dev
                .as_any()
                .map(|a| a.is::<Esp32c3I2c>())
                .unwrap_or(false)
        });
        let gpio_idx = self.peripherals.iter().position(|p| {
            p.dev
                .as_any()
                .map(|a| a.is::<Esp32c3Gpio>())
                .unwrap_or(false)
        });
        let (Some(i2c_idx), Some(gpio_idx)) = (i2c_idx, gpio_idx) else {
            return;
        };
        let matrix_route = self.peripherals[gpio_idx]
            .dev
            .as_any()
            .and_then(|a| a.downcast_ref::<Esp32c3Gpio>())
            .map(|g| g.i2c_matrix_route_state());
        let lines = self.peripherals[i2c_idx]
            .dev
            .as_any_mut()
            .and_then(|a| a.downcast_mut::<Esp32c3I2c>())
            .and_then(|c| {
                matrix_route.map(|route| {
                    c.set_matrix_route_state(route);
                    c.line_levels_arc()
                })
            });
        if let (Some(lines), Some(gpio)) = (
            lines,
            self.peripherals[gpio_idx]
                .dev
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<Esp32c3Gpio>()),
        ) {
            gpio.set_i2c_lines(lines);
        }
    }

    /// Wire C3 IO_MUX per-pad controls into C3 GPIO after both models have
    /// been constructed. The IO_MUX owns the shared register bank; GPIO reads
    /// `FUN_WPU` from it to model Arduino `INPUT_PULLUP`. No-op on any bus
    /// without both C3 peripherals.
    pub(crate) fn wire_esp32c3_pad_controls(&mut self) {
        use crate::peripherals::esp32c3::gpio::Esp32c3Gpio;
        use crate::peripherals::esp32c3::io_mux::Esp32c3IoMux;

        let io_mux_idx = self.peripherals.iter().position(|p| {
            p.dev
                .as_any()
                .map(|any| any.is::<Esp32c3IoMux>())
                .unwrap_or(false)
        });
        let gpio_idx = self.peripherals.iter().position(|p| {
            p.dev
                .as_any()
                .map(|any| any.is::<Esp32c3Gpio>())
                .unwrap_or(false)
        });
        let (Some(io_mux_idx), Some(gpio_idx)) = (io_mux_idx, gpio_idx) else {
            return;
        };

        let controls = self.peripherals[io_mux_idx]
            .dev
            .as_any()
            .and_then(|any| any.downcast_ref::<Esp32c3IoMux>())
            .map(Esp32c3IoMux::pad_controls);
        if let (Some(controls), Some(gpio)) = (
            controls,
            self.peripherals[gpio_idx]
                .dev
                .as_any_mut()
                .and_then(|any| any.downcast_mut::<Esp32c3Gpio>()),
        ) {
            gpio.set_pad_controls(controls);
        }
    }

    /// Bracket a C3 IO_MUX write with GPIO push-capture sampling. A `FUN_WPU`
    /// write changes an input pad electrically even though the GPIO register
    /// block itself is not written, so the usual GPIO-local write hooks would
    /// otherwise miss the edge. The returned GPIO index is passed to
    /// [`Self::finish_esp32c3_io_mux_write`] after the MMIO write succeeds.
    pub(crate) fn begin_esp32c3_io_mux_write(&mut self, io_mux_idx: usize) -> Option<usize> {
        use crate::peripherals::esp32c3::gpio::Esp32c3Gpio;
        use crate::peripherals::esp32c3::io_mux::Esp32c3IoMux;

        if !self.peripherals.get(io_mux_idx).is_some_and(|p| {
            p.dev
                .as_any()
                .map(|any| any.is::<Esp32c3IoMux>())
                .unwrap_or(false)
        }) {
            return None;
        }
        let gpio_idx = self.peripherals.iter().position(|p| {
            p.dev
                .as_any()
                .map(|any| any.is::<Esp32c3Gpio>())
                .unwrap_or(false)
        })?;
        self.peripherals[gpio_idx]
            .dev
            .as_any_mut()
            .and_then(|any| any.downcast_mut::<Esp32c3Gpio>())?
            .tap_snapshot();
        Some(gpio_idx)
    }

    /// Complete a successful C3 IO_MUX write started by
    /// [`Self::begin_esp32c3_io_mux_write`], pushing any changed pad level to
    /// the in-engine logic tap.
    pub(crate) fn finish_esp32c3_io_mux_write(&mut self, gpio_idx: Option<usize>) {
        let Some(gpio_idx) = gpio_idx else {
            return;
        };
        if let Some(gpio) = self.peripherals[gpio_idx]
            .dev
            .as_any_mut()
            .and_then(|any| any.downcast_mut::<crate::peripherals::esp32c3::gpio::Esp32c3Gpio>())
        {
            gpio.tap_report();
        }
    }

    /// Wire the STM32 SPI bit engines' live SCK/MOSI/MISO levels into the
    /// STM32 GPIO ports, so pads whose MODER/AFR (V2) or CRL/CRH CNF (F1)
    /// route an SPI alternate function read the real waveform through
    /// `read_gpio_pad` (which is what the in-engine logic analyzer samples).
    /// The SPI counterpart of [`Self::wire_esp32c3_i2c_pads`]; no-op on buses
    /// without a classic/FIFO STM32 SPI.
    ///
    /// Signal mapping comes from static per-family AF tables sourced from the
    /// datasheet alternate-function maps:
    /// * L4 (FIFO SPI + V2 GPIO): STM32L476 datasheet DS10198 Table 17 —
    ///   SPI1/SPI2 on AF5, SPI3 on AF6.
    /// * F4 (classic SPI + V2 GPIO): STM32F407 datasheet DS8626 Table 9 —
    ///   SPI1/SPI2 on AF5.
    /// * F1 (classic SPI + F1 GPIO): RM0008 §9.3 default pinout, no AFIO
    ///   remap (remap is not modeled). F1 MISO pads are input-mode on real
    ///   silicon and are intentionally not routed (see `GpioPort` docs).
    pub(crate) fn wire_stm32_spi_pads(&mut self) {
        use crate::peripherals::gpio::{GpioPort, GpioRegisterLayout};
        use crate::peripherals::spi::{Spi, SpiSignal};
        use SpiSignal::{Miso, Mosi, Sck};

        // (spi, port, pin, AF, signal, func) — V2 ports, L4 parts (DS10198
        // Table 17: SPI1-3).
        const L4: &[(&str, char, u8, u8, SpiSignal, &str)] = &[
            ("spi1", 'a', 5, 5, Sck, "SPI1_SCK"),
            ("spi1", 'a', 6, 5, Miso, "SPI1_MISO"),
            ("spi1", 'a', 7, 5, Mosi, "SPI1_MOSI"),
            ("spi1", 'b', 3, 5, Sck, "SPI1_SCK"),
            ("spi1", 'b', 4, 5, Miso, "SPI1_MISO"),
            ("spi1", 'b', 5, 5, Mosi, "SPI1_MOSI"),
            ("spi1", 'e', 13, 5, Sck, "SPI1_SCK"),
            ("spi1", 'e', 14, 5, Miso, "SPI1_MISO"),
            ("spi1", 'e', 15, 5, Mosi, "SPI1_MOSI"),
            ("spi2", 'b', 10, 5, Sck, "SPI2_SCK"),
            ("spi2", 'b', 13, 5, Sck, "SPI2_SCK"),
            ("spi2", 'b', 14, 5, Miso, "SPI2_MISO"),
            ("spi2", 'b', 15, 5, Mosi, "SPI2_MOSI"),
            ("spi2", 'c', 2, 5, Miso, "SPI2_MISO"),
            ("spi2", 'c', 3, 5, Mosi, "SPI2_MOSI"),
            ("spi2", 'd', 1, 5, Sck, "SPI2_SCK"),
            ("spi2", 'd', 3, 5, Miso, "SPI2_MISO"),
            ("spi2", 'd', 4, 5, Mosi, "SPI2_MOSI"),
            ("spi3", 'b', 3, 6, Sck, "SPI3_SCK"),
            ("spi3", 'b', 4, 6, Miso, "SPI3_MISO"),
            ("spi3", 'b', 5, 6, Mosi, "SPI3_MOSI"),
            ("spi3", 'c', 10, 6, Sck, "SPI3_SCK"),
            ("spi3", 'c', 11, 6, Miso, "SPI3_MISO"),
            ("spi3", 'c', 12, 6, Mosi, "SPI3_MOSI"),
        ];
        // V2 ports, F4 parts (DS8626 Table 9: SPI1-2).
        const F4: &[(&str, char, u8, u8, SpiSignal, &str)] = &[
            ("spi1", 'a', 5, 5, Sck, "SPI1_SCK"),
            ("spi1", 'a', 6, 5, Miso, "SPI1_MISO"),
            ("spi1", 'a', 7, 5, Mosi, "SPI1_MOSI"),
            ("spi1", 'b', 3, 5, Sck, "SPI1_SCK"),
            ("spi1", 'b', 4, 5, Miso, "SPI1_MISO"),
            ("spi1", 'b', 5, 5, Mosi, "SPI1_MOSI"),
            ("spi2", 'b', 10, 5, Sck, "SPI2_SCK"),
            ("spi2", 'b', 13, 5, Sck, "SPI2_SCK"),
            ("spi2", 'b', 14, 5, Miso, "SPI2_MISO"),
            ("spi2", 'b', 15, 5, Mosi, "SPI2_MOSI"),
            ("spi2", 'c', 2, 5, Miso, "SPI2_MISO"),
            ("spi2", 'c', 3, 5, Mosi, "SPI2_MOSI"),
        ];
        // F1 ports (RM0008 §9.3 default mapping, SPI1-2, SCK/MOSI only).
        const F1: &[(&str, char, u8, SpiSignal, &str)] = &[
            ("spi1", 'a', 5, Sck, "SPI1_SCK"),
            ("spi1", 'a', 7, Mosi, "SPI1_MOSI"),
            ("spi2", 'b', 13, Sck, "SPI2_SCK"),
            ("spi2", 'b', 15, Mosi, "SPI2_MOSI"),
        ];

        for spi_name in ["spi1", "spi2", "spi3"] {
            let Some(spi_idx) = self.find_peripheral_index_by_name(spi_name) else {
                continue;
            };
            let Some((fifo, lines)) = self.peripherals[spi_idx]
                .dev
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<Spi>())
                .filter(|s| s.is_stm32_wire_layout())
                .map(|s| (s.is_fifo_layout(), s.line_levels_arc()))
            else {
                continue;
            };
            for port in ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h'] {
                let Some(gpio_idx) = self.find_peripheral_index_by_name(&format!("gpio{port}"))
                else {
                    continue;
                };
                let Some(gpio) = self.peripherals[gpio_idx]
                    .dev
                    .as_any_mut()
                    .and_then(|a| a.downcast_mut::<GpioPort>())
                else {
                    continue;
                };
                match gpio.register_layout() {
                    GpioRegisterLayout::Stm32V2 => {
                        let table = if fifo { L4 } else { F4 };
                        for &(spi, p, pin, af, sig, func) in table {
                            if spi == spi_name && p == port {
                                gpio.add_spi_pad_route(&lines, pin, Some(af), sig, func);
                            }
                        }
                    }
                    GpioRegisterLayout::Stm32F1 => {
                        for &(spi, p, pin, sig, func) in F1 {
                            if spi == spi_name && p == port {
                                gpio.add_spi_pad_route(&lines, pin, None, sig, func);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// The single funnel through which every SPI device reaches a controller —
    /// the SPI counterpart of [`Self::attach_i2c_slave`]. Wraps then dispatches.
    pub fn attach_spi_device(
        &mut self,
        controller: &str,
        dev: Box<dyn crate::peripherals::spi::SpiDevice>,
    ) -> anyhow::Result<()> {
        let wrapped = bus_trace::wrap_spi(controller, &self.bus_trace, dev);
        let idx = self
            .find_peripheral_index_by_name(controller)
            .ok_or_else(|| anyhow::anyhow!("attach_spi_device: no peripheral '{controller}'"))?;
        let any = self.peripherals[idx].dev.as_any_mut().ok_or_else(|| {
            anyhow::anyhow!("attach_spi_device: '{controller}' is not downcastable")
        })?;
        if let Some(c) = any.downcast_mut::<crate::peripherals::spi::Spi>() {
            c.push_device(wrapped);
        } else if let Some(c) = any.downcast_mut::<crate::peripherals::esp32c3::spi::Esp32c3Spi>() {
            c.push_device(wrapped);
        } else if let Some(c) = any.downcast_mut::<crate::peripherals::esp32::spi::Esp32Spi>() {
            c.push_device(wrapped);
        } else if let Some(c) = any.downcast_mut::<crate::peripherals::esp32s3::gpspi::Esp32s3Spi>()
        {
            c.push_device(wrapped);
        } else {
            anyhow::bail!("attach_spi_device: '{controller}' is not a SPI controller");
        }
        Ok(())
    }
}
