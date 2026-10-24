// SPDX-License-Identifier: MIT
//! Port-aware GPIO controls for an opted-in declarative serial display.
//! No controller downcast, peripheral clock dependency or per-cycle polling.
use std::sync::{Arc, Mutex};

use super::declarative_display::GenericDisplay;
use crate::bus::{BusResidentDevice, DevicePins};
use crate::peripherals::spi::SpiDevice;
use crate::sim_input::{InputChannel, SimInput, SimInputError};

#[derive(Clone, Copy, Debug)]
pub struct DisplayControlPin {
    pub addr: u64,
    pub bit: u8,
    pub active_level: bool,
}

/// SPI owns this handle; the GPIO observer shares the same actual frame RAM.
pub struct DisplaySpiHandle {
    display: Arc<Mutex<GenericDisplay>>,
    cs_pin: String,
    dc_pin: Option<String>,
    dc_source: Option<(u64, u8)>,
    component_id: Option<String>,
}

#[derive(Debug)]
pub struct DisplayGpioObserver {
    display: Arc<Mutex<GenericDisplay>>,
    reset: Option<DisplayControlPin>,
    backlight: Option<DisplayControlPin>,
    addrs: Vec<u64>,
    id: String,
}

impl DisplaySpiHandle {
    pub(super) fn bind(
        mut display: GenericDisplay,
        reset: Option<DisplayControlPin>,
        backlight: Option<DisplayControlPin>,
    ) -> (Self, DisplayGpioObserver) {
        let cs_pin = SpiDevice::cs_pin(&display).to_owned();
        let dc_pin = SpiDevice::dc_pin(&display).map(str::to_owned);
        let dc_source = SpiDevice::dc_source(&display);
        let component_id = SpiDevice::component_id(&display).map(str::to_owned);
        display.configure_gpio_controls(reset.is_some(), backlight.is_some());
        let display = Arc::new(Mutex::new(display));
        let mut addrs: Vec<u64> = [reset, backlight]
            .into_iter()
            .flatten()
            .map(|pin| pin.addr)
            .collect();
        addrs.sort_unstable();
        addrs.dedup();
        let observer = DisplayGpioObserver {
            display: display.clone(),
            reset,
            backlight,
            addrs,
            id: format!(
                "{}:gpio-control",
                component_id.as_deref().unwrap_or("display")
            ),
        };
        (
            Self {
                display,
                cs_pin,
                dc_pin,
                dc_source,
                component_id,
            },
            observer,
        )
    }
}

impl SpiDevice for DisplaySpiHandle {
    fn cs_pin(&self) -> &str {
        &self.cs_pin
    }
    fn dc_pin(&self) -> Option<&str> {
        self.dc_pin.as_deref()
    }
    fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }
    fn dc_source(&self) -> Option<(u64, u8)> {
        self.dc_source
    }
    fn set_dc_source(&mut self, addr: u64, bit: u8) {
        self.dc_source = Some((addr, bit));
        self.display.lock().unwrap().set_dc_source(addr, bit);
    }
    fn set_dc_level(&mut self, level: bool) {
        self.display.lock().unwrap().set_dc_level(level);
    }
    fn cs_select(&mut self) {
        self.display.lock().unwrap().cs_select();
    }
    fn cs_release(&mut self) {
        self.display.lock().unwrap().cs_release();
    }
    fn transfer(&mut self, byte: u8) -> u8 {
        self.display.lock().unwrap().transfer(byte)
    }
    fn artifacts(
        &self,
        id: &str,
        opts: &crate::inspect::InspectOpts,
    ) -> Vec<crate::inspect::Artifact> {
        SpiDevice::artifacts(&*self.display.lock().unwrap(), id, opts)
    }
    fn advance_time_us(&mut self, us: u64) {
        self.display.lock().unwrap().advance_time_us(us);
    }
    fn runtime_snapshot(&self) -> Vec<u8> {
        self.display.lock().unwrap().runtime_snapshot()
    }
    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> crate::SimResult<()> {
        self.display.lock().unwrap().restore_runtime_snapshot(bytes)
    }
}

impl BusResidentDevice for DisplayGpioObserver {
    // No elapsed-time behavior: only the synchronous GPIO write hook samples.
    fn service(&mut self, _pins: &mut dyn DevicePins, _now: u64) {}
    fn service_edge(&mut self, pins: &mut dyn DevicePins, _now: u64) {
        let sample = |pin: Option<DisplayControlPin>| {
            pin.and_then(|pin| {
                pins.known_pad_bit(pin.addr, pin.bit)
                    .map(|value| value == pin.active_level)
            })
        };
        // Sample both wires before changing display state: one port store can
        // change reset and light together, and neither may see a stale peer.
        let (reset, backlight) = (sample(self.reset), sample(self.backlight));
        self.display
            .lock()
            .unwrap()
            .update_gpio_controls(reset, backlight);
    }
    fn needs_per_cycle_service(&self) -> bool {
        false
    }
    fn edge_service_addrs(&self) -> &[u64] {
        &self.addrs
    }
    fn as_sim_input(&mut self) -> &mut dyn SimInput {
        self
    }
    fn id(&self) -> &str {
        &self.id
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl SimInput for DisplayGpioObserver {
    fn input_channels(&self) -> &[InputChannel] {
        &[]
    }
    fn set_input(&mut self, key: &str, _value: f64) -> Result<(), SimInputError> {
        Err(SimInputError::UnknownChannel(key.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, collections::BTreeMap};

    #[derive(Default)]
    struct Pins {
        levels: BTreeMap<(u64, u8), bool>,
        reads: Cell<usize>,
    }
    impl DevicePins for Pins {
        fn output_bit(&self, _addr: u64, _bit: u8) -> Option<bool> {
            panic!("must sample known pads, not output latches")
        }
        fn known_pad_bit(&self, addr: u64, bit: u8) -> Option<bool> {
            self.reads.set(self.reads.get() + 1);
            self.levels.get(&(addr, bit)).copied()
        }
        fn drive_idr_bit(&mut self, _addr: u64, _bit: u8, _high: bool) {
            panic!("observer never drives pins")
        }
        fn drive_input_bit(&mut self, _addr: u64, _bit: u8, _high: bool) -> bool {
            panic!("observer never drives pins")
        }
    }
    const RESET: u64 = 0x41008010;
    const LIGHT: u64 = 0x41008090; // same bit, different port: no PA0/PB0 alias
    fn panel() -> (DisplaySpiHandle, DisplayGpioObserver) {
        let mut panel =
            GenericDisplay::from_yaml(include_str!("../../../tests/fixtures/st7735r-memory.yaml"))
                .unwrap();
        panel.set_glass_window(super::super::declarative_display::GlassWindow {
            col_offset: 0,
            row_offset: 0,
            cols: 1,
            rows: 1,
        });
        panel
            .bind_gpio_controls(
                Some(DisplayControlPin {
                    addr: RESET,
                    bit: 0,
                    active_level: false,
                }),
                Some(DisplayControlPin {
                    addr: LIGHT,
                    bit: 0,
                    active_level: true,
                }),
            )
            .unwrap()
    }
    fn command(panel: &mut DisplaySpiHandle, op: u8, args: &[u8]) {
        panel.set_dc_level(false);
        panel.transfer(op);
        panel.set_dc_level(true);
        for &byte in args {
            panel.transfer(byte);
        }
    }
    fn frame(panel: &DisplaySpiHandle) -> crate::inspect::Artifact {
        panel
            .artifacts(
                "panel",
                &crate::inspect::InspectOpts {
                    include_bytes: true,
                    peripheral: None,
                },
            )
            .remove(0)
    }

    #[test]
    fn display_gpio_unknown_reset_blocks_data_and_tick_does_not_poll() {
        let (mut panel, mut observer) = panel();
        let mut pins = Pins::default();
        pins.levels.insert((RESET, 0), true);
        pins.levels.insert((LIGHT, 0), true);
        assert!(!observer.needs_per_cycle_service());
        assert_eq!(observer.edge_service_addrs(), &[RESET, LIGHT]);
        for cycle in 0..10 {
            observer.service(&mut pins, cycle);
        }
        assert_eq!(pins.reads.get(), 0);
        command(&mut panel, 0x2c, &[0xfc, 0, 0]);
        assert!(frame(&panel).bytes.is_none());
        assert!(frame(&panel).meta["reset_asserted"].is_null());
        observer.service_edge(&mut pins, 10);
        command(&mut panel, 0x2c, &[0xfc, 0, 0]);
        assert_eq!(frame(&panel).bytes, Some(vec![255, 0, 0]));
        assert_eq!(frame(&panel).meta["reset_asserted"], false);
    }

    #[test]
    fn display_gpio_backlight_is_visibility_only_and_port_identity_is_preserved() {
        let (mut panel, mut observer) = panel();
        let mut pins = Pins::default();
        pins.levels.insert((RESET, 0), true);
        pins.levels.insert((LIGHT, 0), false);
        observer.service_edge(&mut pins, 0);
        command(&mut panel, 0x29, &[]);
        command(&mut panel, 0x11, &[]);
        command(&mut panel, 0x2c, &[0, 0xfc, 0]);
        assert_eq!(frame(&panel).bytes, Some(vec![0, 255, 0]));
        assert_eq!(frame(&panel).meta["lit"], false);
        pins.levels.insert((LIGHT, 0), true);
        observer.service_edge(&mut pins, 1);
        assert_eq!(frame(&panel).meta["lit"], true);
        assert_eq!(frame(&panel).meta["reset_asserted"], false);
        pins.levels.remove(&(LIGHT, 0));
        observer.service_edge(&mut pins, 2);
        assert!(frame(&panel).meta["backlight_on"].is_null());
        assert_eq!(frame(&panel).meta["lit"], false);
    }

    #[test]
    fn display_gpio_reset_retains_ram_cancels_fragments_and_invalidates_lut() {
        let (mut panel, mut observer) = panel();
        let mut pins = Pins::default();
        pins.levels.insert((RESET, 0), true);
        pins.levels.insert((LIGHT, 0), true);
        observer.service_edge(&mut pins, 0);
        command(&mut panel, 0x3a, &[5]);
        command(&mut panel, 0x2d, &[63; 128]);
        command(&mut panel, 0x2c, &[0, 0]);
        let saved = panel.runtime_snapshot();
        assert_eq!(frame(&panel).bytes, Some(vec![255; 3]));
        pins.levels.insert((RESET, 0), false);
        observer.service_edge(&mut pins, 1);
        assert_eq!(frame(&panel).meta["colmod"], 6);
        assert_eq!(frame(&panel).meta["reset_asserted"], true);
        command(&mut panel, 0x2c, &[0, 0, 0]);
        assert_eq!(
            frame(&panel).bytes,
            Some(vec![255; 3]),
            "held reset refuses RAM writes"
        );
        pins.levels.insert((RESET, 0), true);
        observer.service_edge(&mut pins, 2);
        command(&mut panel, 0x3a, &[5]);
        command(&mut panel, 0x2c, &[0, 0]);
        assert!(
            frame(&panel).bytes.is_none(),
            "hardware reset invalidated LUT"
        );
        panel.restore_runtime_snapshot(&saved).unwrap();
        assert_eq!(frame(&panel).bytes, Some(vec![255; 3]));
    }

    #[test]
    fn display_gpio_unknown_reset_cannot_resume_a_partial_pixel_on_release() {
        let (mut panel, mut observer) = panel();
        let mut pins = Pins::default();
        pins.levels.insert((RESET, 0), true);
        pins.levels.insert((LIGHT, 0), true);
        observer.service_edge(&mut pins, 0);
        command(&mut panel, 0x2c, &[0xfc, 0]);
        pins.levels.remove(&(RESET, 0));
        observer.service_edge(&mut pins, 1);
        panel.transfer(0);
        pins.levels.insert((RESET, 0), true);
        observer.service_edge(&mut pins, 2);
        panel.transfer(0);
        assert!(frame(&panel).bytes.is_none());
        command(&mut panel, 0x2c, &[0, 0, 0xfc]);
        assert_eq!(frame(&panel).bytes, Some(vec![0, 0, 255]));
    }
}
