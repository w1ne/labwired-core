//! GPIO chip select → STM32 SPI devices.
//!
//! Firmware selects an SPI slave by driving a plain GPIO low around its
//! transfer. Before this route existed the STM32 SPI clocked every byte
//! through every attached device and the last one's answer won, so two
//! slaves on one bus corrupted each other, and a declarative device never saw
//! the CS↑ that closes its frame.
//!
//! The route runs from the same MMIO write choke as the analog mux
//! (`maybe_service_edge_driven_gpio_devices`): a store to a GPIO port that
//! owns a device's `cs_pin` makes the controller re-read its CS levels and
//! deliver the edges. A label that does not resolve to a GPIO output leaves
//! that device on the old pass-through (see `Spi::stm32_cs`). Empty by
//! default → one length check per write.

use super::SystemBus;

/// One SPI controller that routes GPIO chip-select edges.
#[derive(Debug, Clone)]
pub(crate) struct SpiCsWatch {
    /// Peripheral index of the controller.
    pub(crate) spi: usize,
    /// Peripheral indices of the GPIO ports owning its CS pins. Resolved on
    /// the first write after attach, once every port is on the bus.
    pub(crate) gpios: Option<Vec<usize>>,
}

impl SystemBus {
    /// Register `spi` (a peripheral index) as a CS-routing controller if it
    /// reports chip-select pins. Called from the SPI attach funnel.
    pub(crate) fn note_spi_cs_watch(&mut self, spi: usize) {
        if self.peripherals[spi].dev.gpio_chip_select_pins().is_empty() {
            return;
        }
        match self.spi_cs_watches.iter_mut().find(|w| w.spi == spi) {
            // A new device may name a pin on another port: resolve again.
            Some(w) => w.gpios = None,
            None => self.spi_cs_watches.push(SpiCsWatch { spi, gpios: None }),
        }
    }

    pub(crate) fn route_spi_cs_on_write(&mut self, idx: usize) {
        if self.spi_cs_watches.is_empty() {
            return;
        }
        self.route_spi_cs_on_write_cold(idx);
    }

    #[inline(never)]
    fn route_spi_cs_on_write_cold(&mut self, idx: usize) {
        for w in 0..self.spi_cs_watches.len() {
            if self.spi_cs_watches[w].gpios.is_none() {
                let gpios = self.resolve_spi_cs_ports(self.spi_cs_watches[w].spi);
                self.spi_cs_watches[w].gpios = Some(gpios);
            }
            let hit = self.spi_cs_watches[w]
                .gpios
                .as_ref()
                .is_some_and(|g| g.contains(&idx));
            if hit {
                self.sync_spi_cs(self.spi_cs_watches[w].spi);
            }
        }
    }

    /// The GPIO ports that own controller `spi`'s CS labels. A label that
    /// does not resolve contributes nothing: its device stays pass-through.
    fn resolve_spi_cs_ports(&self, spi: usize) -> Vec<usize> {
        let pins = self.peripherals[spi].dev.gpio_chip_select_pins();
        let mut ports: Vec<usize> = pins
            .iter()
            .filter_map(|pin| Self::resolve_pin_odr(self, pin))
            .filter_map(|(addr, _)| self.find_peripheral_index(addr))
            .collect();
        ports.sort_unstable();
        ports.dedup();
        ports
    }

    /// Hand controller `spi` the bus so it can re-read its CS GPIOs. The
    /// controller is lent out under a placeholder, the same swap
    /// `prepare_dma_tx` uses; the GPIO ports it reads are other peripherals.
    fn sync_spi_cs(&mut self, spi: usize) {
        let placeholder: Box<dyn crate::Peripheral> =
            Box::new(crate::peripherals::stub::StubPeripheral::new(0));
        let mut dev = std::mem::replace(&mut self.peripherals[spi].dev, placeholder);
        dev.sync_gpio_chip_selects(self);
        self.peripherals[spi].dev = dev;
    }
}

#[cfg(test)]
#[path = "spi_cs_tests.rs"]
mod tests;
