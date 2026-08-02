// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C3 IO_MUX pad-control peripheral.
//!
//! The C3 keeps `PIN_CTRL` at offset `0x00`; its per-pad registers begin at
//! `0x04` (`GPIO0`) rather than at the base address. GPIO shares this register
//! bank so the pad-level `FUN_WPU` control used by Arduino `INPUT_PULLUP` is
//! visible outside this MMIO model.

use crate::{Peripheral, SimResult};
use labwired_config::PeripheralDescriptor;
use std::sync::{Arc, OnceLock, RwLock};

const PIN_CTRL: u64 = 0x00;
const GPIO0: u64 = 0x04;
const DATE: u64 = 0xfc;
/// ESP32-C3 IO_MUX has registers for GPIO0 through GPIO21. GPIO22 through
/// GPIO25 exist in the separate 26-bit GPIO block but have no IO_MUX pad word.
const PAD_COUNT: usize = 22;
const PIN_CTRL_RESET: u32 = 0x07ff;
const PAD_RESET: u32 = 0x0000_0b00;
const DATE_RESET: u32 = 0x0200_6050;
const FUN_WPU: u32 = 1 << 8;

/// The behavioral model replaces the declarative peripheral, but debugger and
/// inspect clients still need its original register contract. Keep the schema
/// sourced from the same checked-in descriptor rather than duplicating it by
/// hand in Rust.
const IO_MUX_DESCRIPTOR_YAML: &str =
    include_str!("../../../../../configs/peripherals/esp32c3/io_mux.yaml");

fn io_mux_descriptor() -> &'static PeripheralDescriptor {
    static DESCRIPTOR: OnceLock<PeripheralDescriptor> = OnceLock::new();
    DESCRIPTOR.get_or_init(|| {
        PeripheralDescriptor::from_yaml(IO_MUX_DESCRIPTOR_YAML)
            .expect("embedded ESP32-C3 IO_MUX descriptor is valid")
    })
}

pub(crate) type PadControls = Arc<RwLock<[u32; PAD_COUNT]>>;

#[derive(Debug)]
pub struct Esp32c3IoMux {
    pin_ctrl: u32,
    pads: PadControls,
    date: u32,
}

impl Esp32c3IoMux {
    pub fn new() -> Self {
        Self {
            pin_ctrl: PIN_CTRL_RESET,
            // Keep the descriptor's raw cold-reset state observable. In
            // particular, FUN_WPU is already set at reset, so GPIO's electrical
            // input view must see the raw pull-up directly rather than a
            // simulator-only "written" activation convention.
            pads: Arc::new(RwLock::new([PAD_RESET; PAD_COUNT])),
            date: DATE_RESET,
        }
    }

    pub(crate) fn pad_controls(&self) -> PadControls {
        Arc::clone(&self.pads)
    }

    pub fn fun_pull_up(&self, pin: u8) -> bool {
        self.pads
            .read()
            .expect("ESP32-C3 IO_MUX pad controls poisoned")
            .get(pin as usize)
            .map(|word| word & FUN_WPU != 0)
            .unwrap_or(false)
    }

    fn pad_index(word_off: u64) -> Option<usize> {
        if (GPIO0..GPIO0 + (PAD_COUNT as u64) * 4).contains(&word_off) {
            Some(((word_off - GPIO0) / 4) as usize)
        } else {
            None
        }
    }

    fn read_word(&self, word_off: u64) -> u32 {
        if word_off == PIN_CTRL {
            self.pin_ctrl
        } else if word_off == DATE {
            self.date
        } else if let Some(pin) = Self::pad_index(word_off) {
            self.pads
                .read()
                .expect("ESP32-C3 IO_MUX pad controls poisoned")[pin]
        } else {
            0
        }
    }

    fn write_word(&mut self, word_off: u64, value: u32) {
        if word_off == PIN_CTRL {
            self.pin_ctrl = value;
        } else if word_off == DATE {
            self.date = value;
        } else if let Some(pin) = Self::pad_index(word_off) {
            self.pads
                .write()
                .expect("ESP32-C3 IO_MUX pad controls poisoned")[pin] = value;
        }
    }

    fn runtime_state(&self) -> IoMuxSnapshot {
        IoMuxSnapshot {
            pin_ctrl: self.pin_ctrl,
            pads: *self
                .pads
                .read()
                .expect("ESP32-C3 IO_MUX pad controls poisoned"),
            date: self.date,
        }
    }

    fn apply_runtime_state(&mut self, state: IoMuxSnapshot) {
        self.pin_ctrl = state.pin_ctrl;
        *self
            .pads
            .write()
            .expect("ESP32-C3 IO_MUX pad controls poisoned") = state.pads;
        self.date = state.date;
    }
}

impl Default for Esp32c3IoMux {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct IoMuxSnapshot {
    pin_ctrl: u32,
    pads: [u32; PAD_COUNT],
    date: u32,
}

impl Peripheral for Esp32c3IoMux {
    // Inert walk: IO_MUX is a register bank whose pad controls take effect at
    // the MMIO write. It has no time-based state, IRQ, DMA, or event output.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_word(offset & !3);
        Ok(((word >> ((offset & 3) * 8)) & 0xff) as u8)
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        let word = self.read_word(offset & !3);
        Some(((word >> ((offset & 3) * 8)) & 0xff) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let shift = (offset & 3) * 8;
        let mut word = self.read_word(word_off);
        word &= !(0xff << shift);
        word |= (value as u32) << shift;
        self.write_word(word_off, word);
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset & 3 == 0 {
            self.write_word(offset, value);
            Ok(())
        } else {
            for byte in 0..4 {
                self.write(offset + byte, ((value >> (byte * 8)) & 0xff) as u8)?;
            }
            Ok(())
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self.runtime_state()).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        if let Ok(state) = serde_json::from_value::<IoMuxSnapshot>(state) {
            self.apply_runtime_state(state);
        }
        Ok(())
    }

    fn peripheral_descriptor(&self) -> Option<PeripheralDescriptor> {
        Some(io_mux_descriptor().clone())
    }

    fn describe_registers(&self) -> Option<Vec<crate::inspect::RegisterSchema>> {
        Some(
            io_mux_descriptor()
                .registers
                .iter()
                .map(|reg| crate::inspect::RegisterSchema {
                    name: reg.id.clone(),
                    offset: reg.address_offset,
                    size: reg.size,
                    access: match reg.access {
                        labwired_config::Access::ReadWrite => "rw",
                        labwired_config::Access::ReadOnly => "ro",
                        labwired_config::Access::WriteOnly => "wo",
                    },
                    fields: reg
                        .fields
                        .iter()
                        .map(|field| crate::inspect::FieldSchema {
                            name: field.name.clone(),
                            bits: field.bit_range,
                        })
                        .collect(),
                })
                .collect(),
        )
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(&self.runtime_state()).expect("bincode serialize Esp32c3IoMux")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let state: IoMuxSnapshot = bincode::deserialize(bytes).map_err(|error| {
            crate::SimulationError::NotImplemented(format!("Esp32c3IoMux snapshot decode: {error}"))
        })?;
        self.apply_runtime_state(state);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SystemBus;
    use crate::{Bus, DebugControl, SimulationObserver};
    use labwired_config::{ChipDescriptor, SystemManifest};
    use std::sync::{Arc, Mutex};

    fn c3_bus() -> SystemBus {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let chip = ChipDescriptor::from_file(root.join("../../configs/chips/esp32c3.yaml"))
            .expect("read esp32c3 chip yaml");
        let manifest: SystemManifest = serde_yaml::from_str(
            r#"
name: "esp32c3-io-mux-debug-surface-test"
chip: "../chips/esp32c3.yaml"
"#,
        )
        .expect("parse system yaml");
        SystemBus::from_config(&chip, &manifest).expect("construct C3 bus")
    }

    #[test]
    fn cold_descriptor_surface_has_twenty_two_pads_date_and_no_synthetic_registers() {
        let mut io_mux = Esp32c3IoMux::new();

        assert_eq!(io_mux.read_u32(PIN_CTRL).unwrap(), PIN_CTRL_RESET);
        for pin in 0..22u64 {
            assert_eq!(
                io_mux.read_u32(GPIO0 + pin * 4).unwrap(),
                0x0000_0b00,
                "GPIO{pin} must retain the descriptor cold reset"
            );
        }
        for pin in 22..26u64 {
            assert_eq!(
                io_mux.read_u32(GPIO0 + pin * 4).unwrap(),
                0,
                "GPIO{pin} is outside the ESP32-C3 IO_MUX pad bank"
            );
        }
        assert_eq!(io_mux.read_u32(0xfc).unwrap(), 0x0200_6050);

        // The declarative descriptor exposes these RW words raw: keep high
        // reserved bits too, rather than introducing a simulator write mask.
        io_mux.write_u32(PIN_CTRL, 0xa5a5_f123).unwrap();
        io_mux.write_u32(GPIO0 + 4 * 4, 0xfeed_1b02).unwrap();
        io_mux.write_u32(0xfc, 0xdead_beef).unwrap();
        for pin in 22..26u64 {
            io_mux.write_u32(GPIO0 + pin * 4, 0xfeed_face).unwrap();
        }

        assert_eq!(io_mux.read_u32(PIN_CTRL).unwrap(), 0xa5a5_f123);
        assert_eq!(io_mux.read_u32(GPIO0 + 4 * 4).unwrap(), 0xfeed_1b02);
        assert_eq!(io_mux.read_u32(0xfc).unwrap(), 0xdead_beef);
        for pin in 22..26u64 {
            assert_eq!(io_mux.read_u32(GPIO0 + pin * 4).unwrap(), 0);
        }
        assert!(io_mux.fun_pull_up(4));
        assert!(!io_mux.fun_pull_up(22));
    }

    #[test]
    fn c3_io_mux_is_registered_as_a_canonical_behavioral_model() {
        assert!(crate::peripherals::generic_factory::is_canonical_model_type("esp32c3_io_mux"));
    }

    #[test]
    fn c3_io_mux_keeps_its_descriptor_for_machine_debuggers() {
        let mut bus = c3_bus();
        let cpu = crate::system::riscv::configure_riscv(&mut bus);
        let machine = crate::Machine::new(cpu, bus);

        let descriptor = machine
            .get_peripheral_descriptor("io_mux")
            .expect("C3 IO_MUX retains its register descriptor");
        assert!(
            descriptor.registers.iter().any(|reg| reg.id == "PIN_CTRL"),
            "debugger descriptor includes PIN_CTRL"
        );
        assert!(
            descriptor.registers.iter().any(|reg| reg.id == "GPIO0"),
            "debugger descriptor includes GPIO0"
        );

        let inspected = machine.inspect(Some("io_mux"), &crate::inspect::InspectOpts::default());
        let registers = &inspected.peripherals[0].registers;
        assert!(
            registers.iter().any(|reg| reg.name == "PIN_CTRL"),
            "inspect schema includes PIN_CTRL"
        );
        assert!(
            registers.iter().any(|reg| reg.name == "GPIO0"),
            "inspect schema includes GPIO0"
        );
    }

    #[derive(Debug)]
    struct MemoryWriteObserver {
        writes: Arc<Mutex<Vec<(u64, u8, u8)>>>,
    }

    impl SimulationObserver for MemoryWriteObserver {
        fn on_step_end(&self, _cycles: u32, _registers: &[u32]) {}

        fn on_memory_write(&self, addr: u64, old: u8, new: u8) {
            self.writes.lock().unwrap().push((addr, old, new));
        }
    }

    #[test]
    fn io_mux_byte_write_observer_sees_the_programmed_prior_byte() {
        const IO_MUX_BASE: u64 = 0x6000_9000;
        const GPIO4: u64 = IO_MUX_BASE + GPIO0 + 4 * 4;

        let mut bus = c3_bus();
        bus.write_u32(GPIO4, 0xfeed_1b02)
            .expect("program GPIO4 IO_MUX word");

        let writes = Arc::new(Mutex::new(Vec::new()));
        bus.observers.push(Arc::new(MemoryWriteObserver {
            writes: Arc::clone(&writes),
        }));

        bus.write_u8(GPIO4 + 2, 0xa5)
            .expect("write GPIO4's third byte");

        assert_eq!(
            *writes.lock().unwrap(),
            vec![(GPIO4 + 2, 0xed, 0xa5)],
            "observer gets the raw prior byte, not a synthetic zero"
        );
    }

    #[test]
    fn io_mux_is_inert_for_the_legacy_tick_walk() {
        assert!(
            !Esp32c3IoMux::new().needs_legacy_walk(),
            "IO_MUX only changes at MMIO write time and must not block C3 walk deletion"
        );
    }

    #[test]
    fn runtime_snapshot_keeps_pin_ctrl_date_and_pullup_released_for_gpio() {
        let mut original = Esp32c3IoMux::new();
        original.write_u32(PIN_CTRL, 0x321).unwrap();
        original.write_u32(GPIO0 + 4 * 4, 0x0000_1b02).unwrap();
        original.write_u32(0xfc, 0x0bad_c0de).unwrap();
        let snapshot = original.runtime_snapshot();

        let mut resumed = Esp32c3IoMux::new();
        resumed.restore_runtime_snapshot(&snapshot).unwrap();
        let mut gpio = crate::peripherals::esp32c3::gpio::Esp32c3Gpio::new();
        gpio.set_pad_controls(resumed.pad_controls());

        assert!(resumed.fun_pull_up(4));
        assert_eq!(resumed.read_u32(PIN_CTRL).unwrap(), 0x321);
        assert_eq!(resumed.read_u32(0xfc).unwrap(), 0x0bad_c0de);
        assert_eq!(gpio.read_gpio_input(4), Some(true));
    }
}
