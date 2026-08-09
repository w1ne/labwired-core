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
        } else if let Some(c) = any.downcast_mut::<crate::peripherals::nrf54l::twim::Nrf54lTwim>() {
            // Same kit → attach_i2c_device path as every other family. Without
            // this arm, smart-ring sensors could only reach the bus via the
            // nRF54L factory's build_i2c_tree loop — a second home for "what
            // does type X mean on this MCU".
            c.push_slave(wrapped);
        } else if let Some(c) =
            any.downcast_mut::<crate::peripherals::nrf52::serial_instance::Nrf52SerialInstance>()
        {
            // SPIM0/TWIM0 share one MMIO window; an I²C slave belongs to the
            // TWIM half. The nRF52 factory attaches manifest-declared externals
            // itself, but a programmatic attach to `i2c0` must land here too.
            c.attach_i2c(wrapped);
        } else if let Some(c) = any.downcast_mut::<crate::peripherals::rp2040::i2c::Rp2040I2c>() {
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

    /// Share the classic-ESP32 (LX6) I²C0 controller's live SCL/SDA levels with
    /// the classic GPIO port, so pads whose output matrix routes `I2CEXT0_SCL`
    /// (signal 29) / `I2CEXT0_SDA` (signal 30) read the real waveform through
    /// `read_gpio_pad` — which is what the in-engine logic analyzer samples.
    ///
    /// The S3 counterpart is [`Self::wire_esp32s3_i2c_pads`]; like it this is
    /// one-way, because the classic I²C model resolves its slaves by address
    /// rather than by physical pad route. Unlike it, `from_config` is not the
    /// only caller: `configure_xtensa_esp32` registers the classic peripheral
    /// bank in Rust and bypasses the chip YAML's peripheral list, so the classic
    /// call site lives there too. Both are no-ops unless both models are present.
    pub(crate) fn wire_esp32_i2c_pads(&mut self) {
        use crate::peripherals::esp32::gpio::Esp32Gpio;
        use crate::peripherals::esp32::i2c::Esp32I2c;

        let i2c_idx = self
            .peripherals
            .iter()
            .position(|p| p.dev.as_any().map(|a| a.is::<Esp32I2c>()).unwrap_or(false));
        let gpio_idx = self
            .peripherals
            .iter()
            .position(|p| p.dev.as_any().map(|a| a.is::<Esp32Gpio>()).unwrap_or(false));
        // Resolve BOTH before touching either: `pad_lines_arc` CREATES the wire
        // cell, and a controller owning a cell no GPIO port reaches would
        // narrate every transaction into something nothing reads.
        let (Some(i2c_idx), Some(gpio_idx)) = (i2c_idx, gpio_idx) else {
            return;
        };
        let Some(lines) = self.peripherals[i2c_idx]
            .dev
            .as_any_mut()
            .and_then(|a| a.downcast_mut::<Esp32I2c>())
            .map(Esp32I2c::pad_lines_arc)
        else {
            return;
        };
        if let Some(gpio) = self.peripherals[gpio_idx]
            .dev
            .as_any_mut()
            .and_then(|a| a.downcast_mut::<Esp32Gpio>())
        {
            gpio.set_i2c_lines(lines);
        }
    }

    /// Bind the RP2040 UARTs' TX/RX wires to the pads IO_BANK0 can route them
    /// to, so a probe on GP0 shows the serial waveform rather than the SIO
    /// output latch. No-op unless IO_BANK0, SIO and a UART are all on the bus.
    ///
    /// The pad map is transcribed from the RP2040 SVD's `GPIOn_CTRL.FUNCSEL`
    /// enumerations (`uart0_tx`, `uart1_tx`, …) rather than derived, because it
    /// is not derivable: GP0–GP7 alternate instance every four pads, but GP8/GP9
    /// are UART**1**, not UART0, and any parity rule silently mis-assigns them.
    ///
    /// TX ONLY. The SVD also names eight `uart*_rx` pads, and binding them was
    /// tempting and wrong: nothing in the engine ever drives the RX line, so a
    /// routed RX pad would report a confident constant idle-high — including
    /// while an attached GPS or modem was actually sending. That is worse than
    /// the SIO-latch fallback it would replace, because it looks authoritative.
    /// RX joins the table when something drives it, not before. CTS/RTS carry no
    /// narrated waveform either.
    pub(crate) fn wire_rp2040_uart_pads(&mut self) {
        use crate::peripherals::rp2040::io_bank0::{Rp2040IoBank0, GPIO_FUNC_UART};
        use crate::peripherals::rp2040::sio::Rp2040Sio;
        use crate::peripherals::uart::{Uart, LINE_TX};

        /// `(pad, uart instance, line, function name)` — straight from the SVD.
        const PADS: &[(u8, usize, usize, &str)] = &[
            (0, 0, LINE_TX, "UART0_TX"),
            (4, 1, LINE_TX, "UART1_TX"),
            (8, 1, LINE_TX, "UART1_TX"),
            (12, 0, LINE_TX, "UART0_TX"),
            (16, 0, LINE_TX, "UART0_TX"),
            (20, 1, LINE_TX, "UART1_TX"),
            (24, 1, LINE_TX, "UART1_TX"),
            (28, 0, LINE_TX, "UART0_TX"),
        ];

        let Some(functions) = self
            .peripherals
            .iter()
            .find_map(|p| {
                p.dev
                    .as_any()
                    .and_then(|a| a.downcast_ref::<Rp2040IoBank0>())
            })
            .map(Rp2040IoBank0::pad_functions)
        else {
            return;
        };
        // Resolve SIO BEFORE touching a UART: `pad_lines_arc` CREATES the pad
        // cell, and a UART that owns a cell no route reaches still buffers and
        // narrates on every transmitted byte, into a wire nothing reads.
        let Some(sio_idx) = self.find_peripheral_index_by_name("sio") else {
            return;
        };
        if self.peripherals[sio_idx]
            .dev
            .as_any_mut()
            .and_then(|a| a.downcast_mut::<Rp2040Sio>())
            .is_none()
        {
            return;
        }

        for (instance, name) in ["uart0", "uart1"].iter().enumerate() {
            let Some(idx) = self.find_peripheral_index_by_name(name) else {
                continue;
            };
            if !PADS.iter().any(|&(_, inst, _, _)| inst == instance) {
                continue;
            }
            let Some(lines) = self.peripherals[idx]
                .dev
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<Uart>())
                .map(Uart::pad_lines_arc)
            else {
                continue;
            };
            let Some(sio) = self.peripherals[sio_idx]
                .dev
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<Rp2040Sio>())
            else {
                return;
            };
            for &(pin, pad_instance, line, func) in PADS {
                if pad_instance != instance {
                    continue;
                }
                sio.bind_pad_route(functions.clone(), &lines, pin, GPIO_FUNC_UART, line, func);
            }
        }
    }

    /// Bind the RP2040 I²C controllers' wires to the pads IO_BANK0 can route
    /// them to, so `read_gpio_pad` — and the logic analyzer through it — sees
    /// the bus rather than the SIO output latch. No-op unless IO_BANK0, SIO and
    /// an I²C controller are all on the bus.
    ///
    /// Pad assignment is the fixed RP2040 map (datasheet Table 2-19): with
    /// `FUNCSEL = GPIO_FUNC_I2C`, an EVEN pad carries SDA and an odd pad SCL,
    /// and the instance alternates every two pads — GP0/GP1 are I2C0, GP2/GP3
    /// are I2C1, GP4/GP5 are I2C0 again, and so on. Nothing here is chosen by
    /// us: which pads exist is the datasheet's, and which one is live at any
    /// moment is FUNCSEL's.
    pub(crate) fn wire_rp2040_i2c_pads(&mut self) {
        use crate::peripherals::rp2040::i2c::{Rp2040I2c, LINE_SCL, LINE_SDA};
        use crate::peripherals::rp2040::io_bank0::{Rp2040IoBank0, GPIO_FUNC_I2C, PAD_COUNT};
        use crate::peripherals::rp2040::sio::Rp2040Sio;

        let Some(functions) = self
            .peripherals
            .iter()
            .find_map(|p| {
                p.dev
                    .as_any()
                    .and_then(|a| a.downcast_ref::<Rp2040IoBank0>())
            })
            .map(Rp2040IoBank0::pad_functions)
        else {
            return;
        };

        for (instance, name) in ["i2c0", "i2c1"].iter().enumerate() {
            let Some(idx) = self.find_peripheral_index_by_name(name) else {
                continue;
            };
            let Some(lines) = self.peripherals[idx]
                .dev
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<Rp2040I2c>())
                .map(Rp2040I2c::pad_lines_arc)
            else {
                continue;
            };
            let Some(sio_idx) = self.find_peripheral_index_by_name("sio") else {
                return;
            };
            let Some(sio) = self.peripherals[sio_idx]
                .dev
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<Rp2040Sio>())
            else {
                return;
            };
            for pin in 0..PAD_COUNT {
                if usize::from(pin / 2) % 2 != instance {
                    continue;
                }
                let (line, func) = if pin % 2 == 0 {
                    (
                        LINE_SDA,
                        if instance == 0 {
                            "I2C0_SDA"
                        } else {
                            "I2C1_SDA"
                        },
                    )
                } else {
                    (
                        LINE_SCL,
                        if instance == 0 {
                            "I2C0_SCL"
                        } else {
                            "I2C1_SCL"
                        },
                    )
                };
                sio.bind_pad_route(functions.clone(), &lines, pin, GPIO_FUNC_I2C, line, func);
            }
        }
    }

    /// Bind the RP2040 SPI controllers' SCK/MOSI/CSn wires to the pads IO_BANK0
    /// can route them to, so a probe on GP3 shows the shifted bytes rather than
    /// the SIO output latch. No-op unless IO_BANK0, SIO and an SPI controller
    /// are all on the bus.
    ///
    /// The pad map is transcribed from the RP2040 SVD's `GPIOn_CTRL.FUNCSEL`
    /// enumerations (`spi0_sclk`, `spi1_tx`, `spi0_ss_n`, …) rather than derived,
    /// because it is not derivable: the roles repeat every four pads
    /// (rx, ss_n, sclk, tx) but the INSTANCE flips every eight — GP0–7 spi0,
    /// GP8–15 spi1, GP16–23 spi0, GP24–29 spi1 — and any parity rule
    /// mis-assigns half the board. The group is also truncated at the top: GP28
    /// is `spi1_rx` and GP29 is `spi1_ss_n`, with no sclk/tx above GP27.
    ///
    /// SCK / MOSI / CSn ONLY. The SVD also names eight `spi*_rx` pads (GP0, 4,
    /// 8, 12, 16, 20, 24, 28) and binding them was tempting and wrong: nothing
    /// in the engine drives MISO — `Rp2040Spi` has no attached devices and
    /// clocks in the idle level `0x00` — so a routed RX pad would report a
    /// confident constant level, including while an attached flash or display
    /// was supposedly answering. That is worse than the SIO-latch fallback it
    /// would replace, because it looks authoritative. Same call as
    /// [`Self::wire_rp2040_uart_pads`]'s TX-only rationale. RX joins the table
    /// when something drives it, not before.
    ///
    /// CSn IS bound, because the SSP really does drive it whenever firmware
    /// hands the pad over — arduino-pico's `SPIClassRP2040::begin(true)` calls
    /// `gpio_set_function(_CS, GPIO_FUNC_SPI)`. Firmware that keeps chip select
    /// on SIO (the default) simply never makes the route live, and the pad keeps
    /// reading the GPIO latch, which is correct.
    pub(crate) fn wire_rp2040_spi_pads(&mut self) {
        use crate::peripherals::rp2040::io_bank0::{Rp2040IoBank0, GPIO_FUNC_SPI};
        use crate::peripherals::rp2040::sio::Rp2040Sio;
        use crate::peripherals::rp2040::spi::{Rp2040Spi, LINE_CSN, LINE_MOSI, LINE_SCK};

        /// `(pad, spi instance, line, function name)` — straight from the SVD.
        const PADS: &[(u8, usize, usize, &str)] = &[
            (1, 0, LINE_CSN, "SPI0_CSn"),
            (2, 0, LINE_SCK, "SPI0_SCK"),
            (3, 0, LINE_MOSI, "SPI0_TX"),
            (5, 0, LINE_CSN, "SPI0_CSn"),
            (6, 0, LINE_SCK, "SPI0_SCK"),
            (7, 0, LINE_MOSI, "SPI0_TX"),
            (9, 1, LINE_CSN, "SPI1_CSn"),
            (10, 1, LINE_SCK, "SPI1_SCK"),
            (11, 1, LINE_MOSI, "SPI1_TX"),
            (13, 1, LINE_CSN, "SPI1_CSn"),
            (14, 1, LINE_SCK, "SPI1_SCK"),
            (15, 1, LINE_MOSI, "SPI1_TX"),
            (17, 0, LINE_CSN, "SPI0_CSn"),
            (18, 0, LINE_SCK, "SPI0_SCK"),
            (19, 0, LINE_MOSI, "SPI0_TX"),
            (21, 0, LINE_CSN, "SPI0_CSn"),
            (22, 0, LINE_SCK, "SPI0_SCK"),
            (23, 0, LINE_MOSI, "SPI0_TX"),
            (25, 1, LINE_CSN, "SPI1_CSn"),
            (26, 1, LINE_SCK, "SPI1_SCK"),
            (27, 1, LINE_MOSI, "SPI1_TX"),
            (29, 1, LINE_CSN, "SPI1_CSn"),
        ];

        let Some(functions) = self
            .peripherals
            .iter()
            .find_map(|p| {
                p.dev
                    .as_any()
                    .and_then(|a| a.downcast_ref::<Rp2040IoBank0>())
            })
            .map(Rp2040IoBank0::pad_functions)
        else {
            return;
        };
        // ⚠️ Resolve SIO BEFORE touching an SPI: `pad_lines_arc` CREATES the pad
        // cell, and a controller that owns a cell no route reaches still buffers
        // every shifted word, arms a scheduler wakeup per burst, and narrates
        // into a wire nothing reads. Same ordering hazard as
        // `wire_rp2040_uart_pads`.
        let Some(sio_idx) = self.find_peripheral_index_by_name("sio") else {
            return;
        };
        if self.peripherals[sio_idx]
            .dev
            .as_any_mut()
            .and_then(|a| a.downcast_mut::<Rp2040Sio>())
            .is_none()
        {
            return;
        }

        for (instance, name) in ["spi0", "spi1"].iter().enumerate() {
            let Some(idx) = self.find_peripheral_index_by_name(name) else {
                continue;
            };
            if !PADS.iter().any(|&(_, inst, _, _)| inst == instance) {
                continue;
            }
            let Some(lines) = self.peripherals[idx]
                .dev
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<Rp2040Spi>())
                .map(Rp2040Spi::pad_lines_arc)
            else {
                continue;
            };
            let Some(sio) = self.peripherals[sio_idx]
                .dev
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<Rp2040Sio>())
            else {
                return;
            };
            for &(pin, pad_instance, line, func) in PADS {
                if pad_instance != instance {
                    continue;
                }
                sio.bind_pad_route(functions.clone(), &lines, pin, GPIO_FUNC_SPI, line, func);
            }
        }
    }

    /// Share the ESP32-S3 I²C0 controller's live SCL/SDA levels with S3 GPIO,
    /// so pads whose output matrix routes `I2CEXT0_SCL`/`SDA` read the real
    /// waveform through `read_gpio_pad` (which is what the in-engine logic
    /// analyzer samples). No-op unless both S3 models are on the bus.
    ///
    /// The C3 counterpart is [`Self::wire_esp32c3_i2c_pads`]; unlike the C3
    /// this direction is one-way, because the S3 I²C model resolves its slaves
    /// by address rather than by physical pad route.
    pub(crate) fn wire_esp32s3_i2c_pads(&mut self) {
        use crate::peripherals::esp32s3::gpio::Esp32s3Gpio;
        use crate::peripherals::esp32s3::i2c::Esp32s3I2c;

        let i2c_idx = self.peripherals.iter().position(|p| {
            p.dev
                .as_any()
                .map(|a| a.is::<Esp32s3I2c>())
                .unwrap_or(false)
        });
        let gpio_idx = self.peripherals.iter().position(|p| {
            p.dev
                .as_any()
                .map(|a| a.is::<Esp32s3Gpio>())
                .unwrap_or(false)
        });
        let (Some(i2c_idx), Some(gpio_idx)) = (i2c_idx, gpio_idx) else {
            return;
        };
        let lines = self.peripherals[i2c_idx]
            .dev
            .as_any_mut()
            .and_then(|a| a.downcast_mut::<Esp32s3I2c>())
            .map(|c| c.pad_lines_arc());
        if let (Some(lines), Some(gpio)) = (
            lines,
            self.peripherals[gpio_idx]
                .dev
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<Esp32s3Gpio>()),
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
                                gpio.add_pad_route(
                                    lines.pad_lines(),
                                    pin,
                                    Some(af),
                                    sig as usize,
                                    func,
                                );
                            }
                        }
                    }
                    GpioRegisterLayout::Stm32F1 => {
                        for &(spi, p, pin, sig, func) in F1 {
                            if spi == spi_name && p == port {
                                gpio.add_pad_route(
                                    lines.pad_lines(),
                                    pin,
                                    None,
                                    sig as usize,
                                    func,
                                );
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Route each STM32 I²C controller's SCL/SDA onto the GPIO pads that can
    /// carry them, so `read_gpio_pad` — and the logic analyzer through it —
    /// sees the wire this controller drives rather than the idle GPIO latch.
    ///
    /// The SPI counterpart is [`Self::wire_stm32_spi_pads`]; both install
    /// routes through the one `add_pad_route` mechanism, differing only in
    /// their AF table.
    ///
    /// ⚠️ TWO tables, keyed on the CONTROLLER's register generation, not on the
    /// GPIO's. "V2 GPIO registers" is not the same claim as "V2
    /// alternate-function map" — the gap the USART table below still carries
    /// for the L0 — and on I²C the two families genuinely disagree: DS10198
    /// Table 17 puts I2C3 on PA7/PB4 at AF4 on the L476, while DS10086 Rev 5
    /// Table 9 (pages 45-47) leaves AF4 on both of those pads UNASSIGNED on the
    /// F401 and puts I2C3 on PA8/PC9 instead. Routing one table to both would
    /// publish a live I²C waveform onto a pad the F4 silicon does not connect.
    pub(crate) fn wire_stm32_i2c_pads(&mut self) {
        use crate::peripherals::gpio::{GpioPort, GpioRegisterLayout};
        use crate::peripherals::i2c::{I2c, I2cRegisterLayout, LINE_SCL, LINE_SDA};

        /// `(i2c, port, pin, AF, line, func)`.
        type I2cPad = (&'static str, char, u8, u8, usize, &'static str);

        // Modern controller (L4/F7/G0/H5/G4/WB), V2 GPIO — STM32L476 datasheet
        // DS10198 Table 17: I2C1-3 on AF4 (I2C4 on AF5 where fitted, not
        // modelled here).
        const L4: &[I2cPad] = &[
            ("i2c1", 'b', 6, 4, LINE_SCL, "I2C1_SCL"),
            ("i2c1", 'b', 7, 4, LINE_SDA, "I2C1_SDA"),
            ("i2c1", 'b', 8, 4, LINE_SCL, "I2C1_SCL"),
            ("i2c1", 'b', 9, 4, LINE_SDA, "I2C1_SDA"),
            ("i2c2", 'b', 10, 4, LINE_SCL, "I2C2_SCL"),
            ("i2c2", 'b', 11, 4, LINE_SDA, "I2C2_SDA"),
            ("i2c2", 'b', 13, 4, LINE_SCL, "I2C2_SCL"),
            ("i2c2", 'b', 14, 4, LINE_SDA, "I2C2_SDA"),
            ("i2c2", 'f', 0, 4, LINE_SDA, "I2C2_SDA"),
            ("i2c2", 'f', 1, 4, LINE_SCL, "I2C2_SCL"),
            ("i2c3", 'a', 7, 4, LINE_SCL, "I2C3_SCL"),
            ("i2c3", 'b', 4, 4, LINE_SDA, "I2C3_SDA"),
            ("i2c3", 'c', 0, 4, LINE_SCL, "I2C3_SCL"),
            ("i2c3", 'c', 1, 4, LINE_SDA, "I2C3_SDA"),
            ("i2c3", 'c', 9, 4, LINE_SDA, "I2C3_SDA"),
        ];
        // Legacy controller (F1/F2/F4) on V2 GPIO, i.e. the F2/F4 parts — every
        // row read off STM32F401xD/xE datasheet DS10086 Rev 5, Table 9
        // "Alternate function mapping", column AF04 (`I2C1/I2C2/I2C3`):
        // page 45 (port A), page 46 (port B), page 47 (port C).
        //
        // DELIBERATELY ABSENT, and each absence is a fact from those pages:
        // * PB5/AF4 and PB12/AF4 are I2C1_SMBA and I2C2_SMBA — the SMBus alert
        //   line, not a data line, and nothing narrates it.
        // * PB3/AF4 and PB4/AF4 read `-`. Their I2C2_SDA / I2C3_SDA live on
        //   AF9 (`I2C2/I2C3`), a column no controller table here carries;
        //   adding them means adding AF9, not reusing AF4.
        // * PB13/PB14/AF4 and PC0/PC1/AF4 read `-` on the F4 and are I²C on the
        //   L4 — the exact rows that make the two tables non-mergeable.
        // * No port-F rows: the F401 has no port F, and the F405/F407 port-F
        //   assignment is not in this checkout's datasheet corpus, so it is
        //   unverified rather than absent.
        //
        // ⚠️ VERIFIED ON THE F401 ONLY. This one table also serves the F405,
        // F407 and F411 configs, whose datasheets (DS8626, DS8597, DS10314) are
        // NOT in this checkout's corpus — `labwired_datasheet` holds stm32f401,
        // stm32f103, stm32l476, stm32h563 and stm32h735 of the STM32s. Every
        // row here is the F4-series-wide AF4 assignment and the F401 pages are
        // the evidence for it; a row that turns out to differ on a larger part
        // belongs in a second table, not in this one. Adding pads only the
        // bigger parts have (PF0/PF1, PH4/PH5, PH7/PH8) requires reading those
        // documents first — the failure mode is silent, and it is the one
        // `wire_stm32_uart_pads` already carries for the L0.
        const F4: &[I2cPad] = &[
            ("i2c1", 'b', 6, 4, LINE_SCL, "I2C1_SCL"),
            ("i2c1", 'b', 7, 4, LINE_SDA, "I2C1_SDA"),
            ("i2c1", 'b', 8, 4, LINE_SCL, "I2C1_SCL"),
            ("i2c1", 'b', 9, 4, LINE_SDA, "I2C1_SDA"),
            ("i2c2", 'b', 10, 4, LINE_SCL, "I2C2_SCL"),
            ("i2c2", 'b', 11, 4, LINE_SDA, "I2C2_SDA"),
            ("i2c3", 'a', 8, 4, LINE_SCL, "I2C3_SCL"),
            ("i2c3", 'c', 9, 4, LINE_SDA, "I2C3_SDA"),
        ];

        for i2c_name in ["i2c1", "i2c2", "i2c3"] {
            let Some(i2c_idx) = self.find_peripheral_index_by_name(i2c_name) else {
                continue;
            };
            let Some(layout) = self.peripherals[i2c_idx]
                .dev
                .as_any()
                .and_then(|a| a.downcast_ref::<I2c>())
                .map(I2c::register_layout)
            else {
                continue;
            };
            let table = match layout {
                I2cRegisterLayout::Stm32L4 => L4,
                I2cRegisterLayout::Stm32F1 => F4,
                // Kinetis I²C has its own controller and pad model.
                I2cRegisterLayout::Kinetis => continue,
            };
            // ⚠️ Find the V2 ports this instance actually has rows for BEFORE
            // touching the controller: `pad_lines_arc` CREATES the pad cell, and
            // a controller owning a cell no route reaches still buffers and
            // narrates every transaction into a wire nothing reads. The legacy
            // table makes that reachable for the first time — the STM32F103
            // carries the same legacy controller behind F1-layout GPIO ports,
            // which are skipped below, so without this ordering the F103 would
            // switch the whole narration machinery on for nothing. Same hazard
            // `wire_stm32_uart_pads` documents.
            let ports: Vec<char> = ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h']
                .into_iter()
                .filter(|&port| {
                    if !table
                        .iter()
                        .any(|&(i2c, p, ..)| i2c == i2c_name && p == port)
                    {
                        return false;
                    }
                    self.find_peripheral_index_by_name(&format!("gpio{port}"))
                        .and_then(|idx| self.peripherals[idx].dev.as_any())
                        .and_then(|a| a.downcast_ref::<GpioPort>())
                        .is_some_and(|g| g.register_layout() == GpioRegisterLayout::Stm32V2)
                })
                .collect();
            if ports.is_empty() {
                continue;
            }
            let Some(lines) = self.peripherals[i2c_idx]
                .dev
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<I2c>())
                .and_then(|i2c| i2c.pad_lines_arc())
            else {
                continue;
            };
            for port in ports {
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
                for &(i2c, p, pin, af, line, func) in table {
                    if i2c == i2c_name && p == port {
                        gpio.add_pad_route(&lines, pin, Some(af), line, func);
                    }
                }
            }
        }
    }

    /// Route each STM32 USART's TX onto the GPIO pads that can carry it, so a
    /// probe shows the serial waveform rather than the idle GPIO latch.
    ///
    /// Same mechanism as [`Self::wire_stm32_i2c_pads`] and
    /// [`Self::wire_stm32_spi_pads`] — one `add_pad_route` per (pad, AF), and
    /// the AF nibble decides which is live. Only the table differs.
    ///
    /// TX ONLY, for the reason given on [`Self::wire_rp2040_uart_pads`]: nothing
    /// drives the RX line, so a routed RX pad would report an authoritative
    /// idle-high straight through incoming traffic.
    pub(crate) fn wire_stm32_uart_pads(&mut self) {
        use crate::peripherals::gpio::{GpioPort, GpioRegisterLayout};
        use crate::peripherals::uart::{Uart, LINE_TX};

        // (instance, port, pin, AF, line, func). USART1-3 on AF7.
        //
        // This ONE table is applied to every chip whose GPIO carries the V2
        // register layout — a dozen configs from the F4 through the H7 — so it
        // may hold only pads that mean the SAME thing on all of them. That is a
        // real constraint, not a cautious one: on the STM32H563 (DS14258) AF7
        // on PC4 is USART3_**RX**, while on the L476 (DS10198 Table 17) it is
        // USART3_TX, and PG9 is USART6_RX on the H5/H7 against USART1_TX on the
        // L4. Carrying those rows here would publish a controller's TX waveform
        // onto a pad the firmware correctly configured as somebody else's RX —
        // the wrong direction on the wrong peripheral, silently.
        //
        // So PC4/PC5 and PG9/PG10 are deliberately ABSENT. Every row below was
        // re-checked against DS10198 (L476), DS10086 (F401) and DS14258 (H563)
        // and means the same thing on all three.
        //
        // ⚠️ KNOWN GAP, shared with the I²C and SPI tables above: "V2 GPIO
        // registers" is not the same claim as "V2 alternate-function map". The
        // STM32L0 (stm32l073.yaml) carries stm32v2 GPIO but puts USART1/2 on
        // AF4, so these AF7 rows are wrong for it in both directions — the real
        // pads never route, and AF7 on PA2 is a comparator output that would
        // now carry USART2's waveform. Closing it needs a per-family AF map
        // keyed on something finer than the register layout; until then an L0
        // lab must not trust a serial probe.
        const V2: &[(u8, char, u8, u8, usize, &str)] = &[
            (1, 'a', 9, 7, LINE_TX, "USART1_TX"),
            (1, 'b', 6, 7, LINE_TX, "USART1_TX"),
            (2, 'a', 2, 7, LINE_TX, "USART2_TX"),
            (2, 'd', 5, 7, LINE_TX, "USART2_TX"),
            (3, 'b', 10, 7, LINE_TX, "USART3_TX"),
            (3, 'c', 10, 7, LINE_TX, "USART3_TX"),
            (3, 'd', 8, 7, LINE_TX, "USART3_TX"),
        ];

        for instance in 1u8..=3 {
            // Chip configs name these both ways — `uart2` on the L4/F1 configs,
            // `usart2` on the G4. Looking up both is what stops a rename in one
            // yaml silently un-routing that chip's serial pads.
            let Some(uart_idx) = self
                .find_peripheral_index_by_name(&format!("uart{instance}"))
                .or_else(|| self.find_peripheral_index_by_name(&format!("usart{instance}")))
            else {
                continue;
            };
            // Find the V2 ports this instance actually has rows for BEFORE
            // touching the UART: `pad_lines_arc` CREATES the pad cell, and a
            // UART owning a cell no route reaches still buffers and narrates on
            // every transmitted byte into a wire nothing reads. On an F1 chip —
            // whose GPIO is skipped below — that was the whole machinery
            // switched on for nothing.
            let ports: Vec<char> = ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h']
                .into_iter()
                .filter(|&port| {
                    if !V2
                        .iter()
                        .any(|&(inst, p, ..)| inst == instance && p == port)
                    {
                        return false;
                    }
                    self.find_peripheral_index_by_name(&format!("gpio{port}"))
                        .and_then(|idx| self.peripherals[idx].dev.as_any())
                        .and_then(|a| a.downcast_ref::<GpioPort>())
                        .is_some_and(|g| g.register_layout() == GpioRegisterLayout::Stm32V2)
                })
                .collect();
            if ports.is_empty() {
                continue;
            }
            let Some(lines) = self.peripherals[uart_idx]
                .dev
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<Uart>())
                .map(Uart::pad_lines_arc)
            else {
                continue;
            };
            for port in ports {
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
                for &(inst, p, pin, af, line, func) in V2 {
                    if inst == instance && p == port {
                        gpio.add_pad_route(&lines, pin, Some(af), line, func);
                    }
                }
            }
        }
    }

    /// Bind every nRF52 bus controller's wire to every pad its `PSEL` can name,
    /// so a probe shows the bus instead of the GPIO output latch.
    ///
    /// # Why this one looks nothing like the other four
    ///
    /// The other families mux at the PAD, so their wiring is a datasheet TABLE:
    /// "PB6 at AF4 is I2C1_SCL". A pad not in the table can never carry that
    /// signal, and [`PadRoutes`](crate::peripherals::pad_routing) matches a
    /// bound route against the pad's own function register.
    ///
    /// Nordic has no such register and no such table. `PSEL.SCL` is five bits
    /// of pin plus one of port: ANY pad can carry ANY signal, chosen at runtime
    /// (nRF52840 PS v1.11 §6.31.7.19, p798). So the "table" here is the full
    /// cross product — every pad × every signal — and which single route is
    /// live at each instant comes from the shared claim table the peripherals
    /// publish their `PSEL` into
    /// ([`crate::peripherals::nrf52::pin_select`]). Same seam, opposite
    /// direction, and re-pointing a `PSEL` mid-run follows immediately because
    /// the answer is read live rather than baked in here.
    ///
    /// # Which chips this touches, and which it deliberately does not
    ///
    /// THREE independent gates, all structural rather than a chip-name list.
    /// They overlap, and that overlap is measured, not assumed: a mutation that
    /// deletes the `window_offset` gate alone SURVIVES the bus-visibility board
    /// because the instance-address table below already excludes the nRF5340
    /// (its UARTE0 is at 0x5000_8000, not 0x4000_2000). Deleting BOTH is what
    /// the board catches. So the address table is the load-bearing exclusion
    /// today and the `window_offset` check is the belt to its braces — which is
    /// worth keeping, because a future nRF53 yaml that happened to map a
    /// peripheral at an nRF52 address would otherwise be wired on a `PSEL`
    /// encoding nothing in this repo has verified.
    ///
    /// * The GPIO port must carry the nRF52 register layout AND start its MMIO
    ///   window at the block base (`window_offset == 0`). From the nRF5340 on,
    ///   Nordic re-bases a port at `OUT` and the chip yaml says
    ///   `reg_offset: 0x500` — see [`GpioPort::window_offset`]. That is the
    ///   marker of the nRF53/nRF54 generation, whose `PSEL` field layout is NOT
    ///   verified here: no nRF5340 datasheet is in this checkout's corpus
    ///   (`labwired_datasheet` holds nrf52840 and nrf54l15 of the Nordics), so
    ///   nrf5340.yaml is left unwired rather than routed on the assumption that
    ///   a part with a different GPIO base kept the same pin-select encoding.
    ///   Closing that gap needs the nRF5340 PS, not a sibling's.
    /// * The controller must be one of the nRF52 models. The nRF54L15 carries
    ///   dedicated `Nrf54lUarte` / `Nrf54lTwim` models (EasyDMA moved into a
    ///   `DMA.{RX,TX}` cluster), so it falls out of every arm below without
    ///   needing to be named.
    ///
    /// * The instance must sit at an address the PS instance tables name.
    ///   Instance NAMES come from the base address, not the chip yaml's `id`:
    ///   `TWIM0` is "the instance at 0x40003000" per PS §6.31.7, and calling it
    ///   that regardless of whether a yaml spells the id `i2c0`, `twim0` or
    ///   `arduino_i2c` is what stops a rename from silently relabelling a
    ///   waveform. It is also what keeps a part with an nRF52 peripheral MODEL
    ///   at a different address — the nRF5340's UARTE0 — out of the table.
    ///
    /// TX / SCL / SDA / SCK / MOSI ONLY. `PSEL.RXD`, `PSEL.MISO`, `PSEL.CTS`,
    /// `PSEL.RTS` and `PSEL.CSN` are tracked registers that nothing in this
    /// engine DRIVES, so a pad routed to one would report a confident constant
    /// idle level straight through real traffic — worse than the GPIO-latch
    /// fallback it replaced, because it looks authoritative. Same call as
    /// [`Self::wire_rp2040_uart_pads`].
    pub(crate) fn wire_nrf52_pads(&mut self) {
        use crate::peripherals::gpio::{GpioPort, GpioRegisterLayout};
        use crate::peripherals::nrf52::pin_select::NrfPinClaims;
        use crate::peripherals::nrf52::serial_instance::Nrf52SerialInstance;
        use crate::peripherals::nrf52::twim::{LINE_SCL, LINE_SDA};
        use crate::peripherals::nrf52::uarte::{Nrf52Uarte, LINE_TXD};
        use crate::peripherals::spi::{Spi, SpiSignal};
        use std::sync::Arc;

        /// UARTE instances by base address (PS v1.11 §6.34.9, p836).
        const UARTE: &[(u64, &str)] = &[(0x4000_2000, "UARTE0_TX"), (0x4002_8000, "UARTE1_TX")];
        /// TWIM instances by base address (PS v1.11 §6.31.7, p790).
        const TWIM: &[(u64, &str, &str)] = &[
            (0x4000_3000, "TWIM0_SCL", "TWIM0_SDA"),
            (0x4000_4000, "TWIM1_SCL", "TWIM1_SDA"),
        ];
        /// SPIM instances by base address (PS v1.11 §6.25.6, p727). SPIM3
        /// (0x4002_F000) is listed for completeness of the address decode; no
        /// chip yaml in this checkout maps it.
        const SPIM: &[(u64, &str, &str)] = &[
            (0x4000_3000, "SPIM0_SCK", "SPIM0_MOSI"),
            (0x4000_4000, "SPIM1_SCK", "SPIM1_MOSI"),
            (0x4002_3000, "SPIM2_SCK", "SPIM2_MOSI"),
            (0x4002_F000, "SPIM3_SCK", "SPIM3_MOSI"),
        ];

        // ⚠️ Resolve the GPIO ports FIRST and bail if there are none.
        // `pad_lines_arc` CREATES the wire cell, and a controller that owns a
        // cell no route reaches still buffers every byte of every transfer and
        // narrates it into something nothing reads — the ordering hazard
        // `wire_rp2040_uart_pads` documents, and the reason nothing below
        // touches a controller until this vector is known non-empty.
        let ports: Vec<(usize, u8)> = (0u8..2)
            .filter_map(|port| {
                let idx = self.find_peripheral_index_by_name(&format!("gpio{port}"))?;
                let gpio = self.peripherals[idx]
                    .dev
                    .as_any()
                    .and_then(|a| a.downcast_ref::<GpioPort>())?;
                (gpio.register_layout() == GpioRegisterLayout::Nrf52 && gpio.window_offset() == 0)
                    .then_some((idx, port))
            })
            .collect();
        if ports.is_empty() {
            return;
        }

        let claims = Arc::new(NrfPinClaims::new());
        for &(idx, port) in &ports {
            if let Some(gpio) = self.peripherals[idx]
                .dev
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<GpioPort>())
            {
                gpio.set_nrf_pin_claims(claims.clone(), port);
            }
        }

        // Claim tokens are handed out from 0 and are unique per (instance,
        // signal) across the whole chip. The SAME value is installed in the
        // controller and bound into every port's routing table — that identity
        // IS the routing, so it is minted in one place and never derived twice.
        let mut next_token: u32 = 0;
        // `(pad-line cell, [(claim token, line index, signal name)])` for every
        // controller found, collected before any binding so the borrow of
        // `self.peripherals` is over.
        type Bindings = (
            std::sync::Arc<crate::peripherals::pad_lines::PadLines>,
            Vec<(u32, usize, &'static str)>,
        );
        let mut wired: Vec<Bindings> = Vec::new();

        for entry_idx in 0..self.peripherals.len() {
            let base = self.peripherals[entry_idx].base;
            let Some(any) = self.peripherals[entry_idx].dev.as_any_mut() else {
                continue;
            };

            if let Some(uarte) = any.downcast_mut::<Nrf52Uarte>() {
                let Some(&(_, func)) = UARTE.iter().find(|&&(addr, _)| addr == base) else {
                    // A UARTE at an address the PS instance table does not name
                    // is not one this engine can label, and an unlabelled
                    // binding would land on the bus-visibility board as an
                    // unclassified name — a hard failure there, by design.
                    continue;
                };
                let token = next_token;
                next_token += 1;
                let lines = uarte.pad_lines_arc();
                uarte.install_pin_claims(&claims, token);
                wired.push((lines, vec![(token, LINE_TXD, func)]));
                continue;
            }

            if let Some(mux) = any.downcast_mut::<Nrf52SerialInstance>() {
                // One MMIO window, two personalities, two independent wires:
                // ENABLE picks which is driving, and each half claims its pads
                // only while it is the selected one.
                if let Some(&(_, scl, sda)) = TWIM.iter().find(|&&(addr, ..)| addr == base) {
                    let (scl_token, sda_token) = (next_token, next_token + 1);
                    next_token += 2;
                    let lines = mux.twim_pad_lines_arc();
                    mux.install_twim_pin_claims(&claims, scl_token, sda_token);
                    wired.push((
                        lines,
                        vec![(scl_token, LINE_SCL, scl), (sda_token, LINE_SDA, sda)],
                    ));
                }
                if let Some(&(_, sck, mosi)) = SPIM.iter().find(|&&(addr, ..)| addr == base) {
                    let (sck_token, mosi_token) = (next_token, next_token + 1);
                    next_token += 2;
                    let lines = mux.spim_pad_lines_arc();
                    mux.install_spim_pin_claims(&claims, sck_token, mosi_token);
                    wired.push((
                        lines,
                        vec![
                            (sck_token, SpiSignal::Sck as usize, sck),
                            (mosi_token, SpiSignal::Mosi as usize, mosi),
                        ],
                    ));
                }
                continue;
            }

            if let Some(twim) = any.downcast_mut::<crate::peripherals::nrf52::twim::Nrf52Twim>() {
                let Some(&(_, scl, sda)) = TWIM.iter().find(|&&(addr, ..)| addr == base) else {
                    continue;
                };
                let (scl_token, sda_token) = (next_token, next_token + 1);
                next_token += 2;
                let lines = twim.pad_lines_arc();
                twim.install_pin_claims(&claims, scl_token, sda_token);
                wired.push((
                    lines,
                    vec![(scl_token, LINE_SCL, scl), (sda_token, LINE_SDA, sda)],
                ));
                continue;
            }

            if let Some(spi) = any.downcast_mut::<Spi>() {
                if !spi.is_nrf_wire_layout() {
                    continue;
                }
                let Some(&(_, sck, mosi)) = SPIM.iter().find(|&&(addr, ..)| addr == base) else {
                    continue;
                };
                let (sck_token, mosi_token) = (next_token, next_token + 1);
                next_token += 2;
                let lines = spi.line_levels_arc().pad_lines().clone();
                spi.install_nrf_pin_claims(&claims, sck_token, mosi_token);
                wired.push((
                    lines,
                    vec![
                        (sck_token, SpiSignal::Sck as usize, sck),
                        (mosi_token, SpiSignal::Mosi as usize, mosi),
                    ],
                ));
            }
        }

        // Every pad × every signal. `PSEL.PIN` is five bits wide on both ports,
        // so 32 routes per port is the exact span the register can name — a
        // port that physically bonds out fewer (P1 has 16 on the nRF52840) is
        // simply never claimed above its pin count, and `read_gpio_pad` already
        // refuses pins ≥ 32.
        for &(gpio_idx, _) in &ports {
            let Some(gpio) = self.peripherals[gpio_idx]
                .dev
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<GpioPort>())
            else {
                continue;
            };
            for (lines, signals) in &wired {
                for &(token, line, func) in signals {
                    for pin in 0..32u8 {
                        gpio.add_pad_route_selector(lines, pin, Some(token), line, func);
                    }
                }
            }
        }
    }

    /// Every peripheral signal name that any pad on this bus is BOUND to carry,
    /// deduplicated and sorted — `["I2C1_SCL", "I2C1_SDA", "USART2_TX", …]`.
    ///
    /// This is the ONE read-only window onto what the `wire_*_pads` functions
    /// above actually achieved, and it exists for exactly one caller:
    /// `crates/core/tests/bus_visibility.rs`, which builds every chip in
    /// `configs/chips/` and derives the bus-visibility scoreboard from this
    /// list. Nothing in the engine reads it.
    ///
    /// Why it lives here and not in the test: the routes are held privately by
    /// five different GPIO-ish models (STM32 `GpioPort`, RP2040 `Sio`, and the
    /// C3/S3/classic ESP32 GPIO ports), each reached by a different downcast.
    /// Publishing five downcasts to make the scoreboard derivable would be a
    /// far larger surface than publishing the one question it asks.
    ///
    /// ⚠️ The names are the DERIVED truth. A `wire_*_pads` function that
    /// silently stops binding — a renamed peripheral in a chip yaml, a family
    /// whose GPIO model changed, an early `return` on a missing model — empties
    /// this list for that chip, and the scoreboard ratchet fails. Do not add a
    /// fallback that synthesises names from anything other than live bindings:
    /// that is precisely the guarantee being sold.
    pub fn bound_pad_functions(&self) -> Vec<&'static str> {
        use crate::peripherals::esp32::gpio::Esp32Gpio;
        use crate::peripherals::esp32c3::gpio::Esp32c3Gpio;
        use crate::peripherals::esp32s3::gpio::Esp32s3Gpio;
        use crate::peripherals::gpio::GpioPort;
        use crate::peripherals::rp2040::sio::Rp2040Sio;

        let mut out: Vec<&'static str> = Vec::new();
        for entry in &self.peripherals {
            let Some(any) = entry.dev.as_any() else {
                continue;
            };
            let funcs = if let Some(g) = any.downcast_ref::<GpioPort>() {
                g.bound_pad_functions()
            } else if let Some(g) = any.downcast_ref::<Rp2040Sio>() {
                g.bound_pad_functions()
            } else if let Some(g) = any.downcast_ref::<Esp32c3Gpio>() {
                g.bound_pad_functions()
            } else if let Some(g) = any.downcast_ref::<Esp32s3Gpio>() {
                g.bound_pad_functions()
            } else if let Some(g) = any.downcast_ref::<Esp32Gpio>() {
                g.bound_pad_functions()
            } else {
                continue;
            };
            for f in funcs {
                if !out.contains(&f) {
                    out.push(f);
                }
            }
        }
        out.sort_unstable();
        out
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
        } else if let Some(c) =
            any.downcast_mut::<crate::peripherals::nrf52::serial_instance::Nrf52SerialInstance>()
        {
            // The SPIM half of the shared SPIM0/TWIM0 window.
            c.attach_spi(wrapped);
        } else {
            anyhow::bail!("attach_spi_device: '{controller}' is not a SPI controller");
        }
        Ok(())
    }
}
