// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Per-chip REACHABILITY gate for the stimulus surface.
//!
//! The bug this exists to prevent: `SystemBus::for_each_sim_input` was a
//! downcast chain over three hardcoded controller types (`I2c` / `Spi` /
//! `Uart`). Every controller added afterwards — ESP32-C3 I²C, ESP32-C3 SPI,
//! ESP32-S3 I²C — hosted devices that no `list_inputs` query reported and no
//! `set_input` call could drive. Nothing was red. The component unit tests
//! passed (they exercise the model directly), the controller tests passed
//! (they exercise register behavior), and the stimulus tests passed (they only
//! ever ran on chips whose controller happened to be in the chain). The whole
//! class of failure was "the device works, and the bus cannot see it".
//!
//! So this file asserts the one thing none of those did: that on a REAL board
//! built from config, an attached input device is reachable through the PUBLIC
//! surface an agent actually calls — `list_inputs` and `set_input` — and that
//! a set moves what the device reports over its own protocol.
//!
//! It is table-driven over chips ([`CASES`]) so that onboarding a chip means
//! adding a row. A new controller family that forgets
//! `Peripheral::for_each_attached_sim_input` fails here rather than shipping a
//! board whose sensors are silently undrivable.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::components::{Adxl345, GenericSpiDevice};
use labwired_core::peripherals::i2c::I2cDevice;
use labwired_core::peripherals::spi::SpiDevice;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Build a bus from a `configs/systems/*.yaml`, resolving its relative `chip:`.
fn bus_from_system(system_rel: &str) -> SystemBus {
    let sys_path = workspace_root().join("configs/systems").join(system_rel);
    let mut manifest =
        SystemManifest::from_file(&sys_path).unwrap_or_else(|e| panic!("load {system_rel}: {e}"));
    let chip_path = sys_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load chip for {system_rel}: {e}"));
    // Descriptors nested inside the chip (`../peripherals/**.yaml`) resolve
    // relative to `manifest.chip`, so it must be absolute or the build fails
    // wherever the test binary happens to run from.
    manifest.chip = chip_path.to_str().expect("utf-8 chip path").to_string();
    SystemBus::from_config(&chip, &manifest).unwrap_or_else(|e| panic!("build {system_rel}: {e}"))
}

/// How the case's input device hangs off the controller.
enum Attach {
    /// ADXL345 accelerometer at `address`. `route` carries the target-neutral
    /// signal map ESP32-C3 requires (it rejects controller-name-only binds).
    I2cAdxl345 {
        address: u8,
        route: Option<&'static [(&'static str, &'static str)]>,
    },
    /// MAX31855 thermocouple amplifier on `cs_pin`.
    SpiMax31855 { cs_pin: &'static str },
}

struct Case {
    /// Human label used in failure messages.
    chip: &'static str,
    /// `configs/systems/*.yaml` to build the board from.
    system: &'static str,
    /// Controller id within that chip to attach to.
    controller: &'static str,
    attach: Attach,
}

/// One row per (chip, controller family) that can host an input device.
///
/// Adding a chip with an I²C or SPI controller means adding a row. Leaving it
/// out is the failure mode this table exists to make expensive: the chip ships,
/// its sensors attach, and nothing tells you they cannot be driven.
const CASES: &[Case] = &[
    // Generic STM32 controllers — the family that always worked, kept as the
    // control: if this row breaks, the seam itself is wrong, not one chip.
    Case {
        chip: "stm32f407 (generic I2c)",
        system: "nucleo-f407.yaml",
        controller: "i2c1",
        attach: Attach::I2cAdxl345 {
            address: 0x53,
            route: None,
        },
    },
    Case {
        chip: "stm32f407 (generic Spi)",
        system: "nucleo-f407.yaml",
        controller: "spi1",
        attach: Attach::SpiMax31855 { cs_pin: "PA4" },
    },
    // The three that were unreachable before the seam.
    Case {
        chip: "esp32c3 (Esp32c3I2c)",
        system: "esp32c3-devkit.yaml",
        controller: "i2c0",
        attach: Attach::I2cAdxl345 {
            address: 0x53,
            route: Some(&[("sda", "GPIO4"), ("scl", "GPIO5")]),
        },
    },
    Case {
        chip: "esp32s3 (Esp32s3I2c)",
        system: "esp32s3-zero.yaml",
        controller: "i2c0",
        attach: Attach::I2cAdxl345 {
            address: 0x53,
            route: None,
        },
    },
    Case {
        chip: "esp32c3 (Esp32c3Spi)",
        system: "esp32c3-devkit.yaml",
        controller: "spi2",
        attach: Attach::SpiMax31855 { cs_pin: "GPIO7" },
    },
    // nRF52840 — the family that could not attach at all. `Nrf52Twim`'s
    // `Peripheral` impl declared no `as_any_mut`, so `attach_i2c_slave`'s
    // downcast could never match it and every attach to a TWIM controller
    // failed loudly. `twi1` (chip type `nrf52840_i2c`, canonical
    // `nrf52840_twim`) is the standalone TWIM every nRF52840 board carries.
    Case {
        chip: "nrf52840 (Nrf52Twim)",
        system: "nrf52840-dk.yaml",
        controller: "twi1",
        attach: Attach::I2cAdxl345 {
            address: 0x53,
            route: None,
        },
    },
    // `i2c0` is the SPIM0/TWIM0 mux sharing one MMIO window. It was reachable
    // by neither attach funnel (absent from both dispatch chains) AND
    // implemented no `for_each_attached_sim_input`, so devices the nRF52
    // factory attached into it from a manifest were SILENTLY undrivable —
    // the worse of the two holes, since attach appeared to succeed.
    Case {
        chip: "nrf52840 (Nrf52SerialInstance, TWIM half)",
        system: "nrf52840-dk.yaml",
        controller: "i2c0",
        attach: Attach::I2cAdxl345 {
            address: 0x53,
            route: None,
        },
    },
    Case {
        chip: "nrf52840 (Nrf52SerialInstance, SPIM half)",
        system: "nrf52840-dk.yaml",
        controller: "i2c0",
        attach: Attach::SpiMax31855 { cs_pin: "P0.31" },
    },
];

impl Case {
    /// Build the board and attach the case's device through the PRODUCTION
    /// attach funnel (`attach_i2c_slave_with_route` / `attach_spi_device`), so
    /// the device sits on the controller exactly as a manifest-declared one
    /// would — trace wrapper and all.
    fn build(&self) -> SystemBus {
        let mut bus = bus_from_system(self.system);
        match &self.attach {
            Attach::I2cAdxl345 { address, route } => {
                let route: Option<BTreeMap<String, String>> = route.map(|pairs| {
                    pairs
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect()
                });
                bus.attach_i2c_slave_with_route(
                    self.controller,
                    Box::new(Adxl345::new(*address)),
                    route.as_ref(),
                )
                .unwrap_or_else(|e| panic!("{}: attach ADXL345: {e}", self.chip));
            }
            Attach::SpiMax31855 { cs_pin } => {
                let yaml = labwired_config::embedded_device_yaml("max31855")
                    .expect("max31855 descriptor embedded");
                let dev = GenericSpiDevice::from_yaml(yaml, cs_pin)
                    .expect("max31855.yaml is a valid declarative spi descriptor");
                bus.attach_spi_device(self.controller, Box::new(dev))
                    .unwrap_or_else(|e| panic!("{}: attach MAX31855: {e}", self.chip));
            }
        }
        bus
    }

    /// The channel a set is asserted through, and a value distinct from the
    /// model's boot state so a no-op set cannot pass.
    fn probe(&self) -> (&'static str, f64) {
        match self.attach {
            // Boot rest sample is z = +1 g; drive it to an unmistakable -1 g.
            Attach::I2cAdxl345 { .. } => ("z", -1.0),
            // Boot is 0 °C; 500 °C is well clear of it and in K-type range.
            Attach::SpiMax31855 { .. } => ("temperature", 500.0),
        }
    }

    /// Read the driven quantity back out of the attached model over its own
    /// device protocol (I²C register reads / SPI word shift-out).
    ///
    /// This deliberately reaches the device by a path INDEPENDENT of
    /// `for_each_sim_input` — the walk under test cannot be its own witness.
    fn read_back(&self, bus: &mut SystemBus) -> f64 {
        match self.attach {
            Attach::I2cAdxl345 { .. } => with_adxl345(bus, |dev| {
                // DATAZ0/DATAZ1 (0x36/0x37), little-endian, 256 LSB/g in the
                // full-resolution default the model boots in.
                dev.stop(); // fresh transaction: reset the register-pointer phase
                dev.write(0x36);
                let lo = dev.read() as u16;
                let hi = dev.read() as u16;
                (((hi << 8) | lo) as i16) as f64 / 256.0
            }),
            Attach::SpiMax31855 { .. } => with_max31855(bus, |dev| {
                dev.cs_select();
                let mut word: u32 = 0;
                for _ in 0..4 {
                    word = (word << 8) | dev.transfer(0x00) as u32;
                }
                dev.cs_release();
                // Bits 31..18: signed 14-bit thermocouple temperature, 0.25 °C/LSB.
                let raw = (word >> 18) as u16;
                let signed = ((raw << 2) as i16) >> 2; // sign-extend from 14 bits
                signed as f64 / 4.0
            }),
        }
    }
}

/// Enumerate every attached I²C slave across all controller families and hand
/// the first ADXL345 to `f`. Panics when none is found — a silent miss here
/// would make every assertion below vacuous.
fn with_adxl345<R>(bus: &mut SystemBus, f: impl FnOnce(&mut Adxl345) -> R) -> R {
    use labwired_core::peripherals::esp32c3::i2c::Esp32c3I2c;
    use labwired_core::peripherals::esp32s3::i2c::Esp32s3I2c;
    use labwired_core::peripherals::i2c::I2c;
    use labwired_core::peripherals::nrf52::serial_instance::Nrf52SerialInstance;
    use labwired_core::peripherals::nrf52::twim::Nrf52Twim;

    for entry in bus.peripherals.iter_mut() {
        let Some(any) = entry.dev.as_any_mut() else {
            continue;
        };
        if let Some(c) = any.downcast_ref::<I2c>() {
            for cell in c.attached_devices() {
                let mut dev = cell.borrow_mut();
                if let Some(a) = dev.as_any_mut().and_then(|a| a.downcast_mut::<Adxl345>()) {
                    return f(a);
                }
            }
        } else if let Some(c) = any.downcast_mut::<Esp32c3I2c>() {
            for slave in c.attached_slaves_mut() {
                if let Some(a) = slave.as_any_mut().and_then(|a| a.downcast_mut::<Adxl345>()) {
                    return f(a);
                }
            }
        } else if let Some(c) = any.downcast_mut::<Esp32s3I2c>() {
            for slave in c.attached_slaves_mut() {
                if let Some(a) = slave.as_any_mut().and_then(|a| a.downcast_mut::<Adxl345>()) {
                    return f(a);
                }
            }
        } else if let Some(c) = any.downcast_ref::<Nrf52Twim>() {
            for cell in c.attached_devices() {
                let mut dev = cell.borrow_mut();
                if let Some(a) = dev.as_any_mut().and_then(|a| a.downcast_mut::<Adxl345>()) {
                    return f(a);
                }
            }
        } else if let Some(c) = any.downcast_ref::<Nrf52SerialInstance>() {
            for cell in c.attached_i2c_devices() {
                let mut dev = cell.borrow_mut();
                if let Some(a) = dev.as_any_mut().and_then(|a| a.downcast_mut::<Adxl345>()) {
                    return f(a);
                }
            }
        }
    }
    panic!("no ADXL345 found on the bus — the test's own readback path is broken");
}

/// SPI counterpart of [`with_adxl345`].
fn with_max31855<R>(bus: &mut SystemBus, f: impl FnOnce(&mut GenericSpiDevice) -> R) -> R {
    use labwired_core::peripherals::esp32c3::spi::Esp32c3Spi;
    use labwired_core::peripherals::nrf52::serial_instance::Nrf52SerialInstance;
    use labwired_core::peripherals::spi::Spi;

    for entry in bus.peripherals.iter_mut() {
        let Some(any) = entry.dev.as_any_mut() else {
            continue;
        };
        if let Some(c) = any.downcast_mut::<Spi>() {
            for dev in c.attached_devices.iter_mut() {
                if let Some(m) = dev
                    .as_any_mut()
                    .and_then(|a| a.downcast_mut::<GenericSpiDevice>())
                {
                    return f(m);
                }
            }
        } else if let Some(c) = any.downcast_mut::<Esp32c3Spi>() {
            for dev in c.attached_devices_mut() {
                if let Some(m) = dev
                    .as_any_mut()
                    .and_then(|a| a.downcast_mut::<GenericSpiDevice>())
                {
                    return f(m);
                }
            }
        } else if let Some(c) = any.downcast_mut::<Nrf52SerialInstance>() {
            for dev in c.attached_spi_devices_mut() {
                if let Some(m) = dev
                    .as_any_mut()
                    .and_then(|a| a.downcast_mut::<GenericSpiDevice>())
                {
                    return f(m);
                }
            }
        }
    }
    panic!("no MAX31855 found on the bus — the test's own readback path is broken");
}

/// Every case's device must be DISCOVERABLE: its channels appear in
/// `list_inputs`, the query behind the MCP stimulus surface and the browser
/// panel. Before the seam this was empty on all three ESP controllers.
#[test]
fn every_chip_discovers_its_attached_input_device() {
    for case in CASES {
        let mut bus = case.build();
        let (channel, _) = case.probe();
        let inputs = bus.list_inputs();
        assert!(
            inputs.iter().any(|(_, ch)| ch.key == channel),
            "{}: `{channel}` is not discoverable via list_inputs; \
             the controller likely does not implement \
             Peripheral::for_each_attached_sim_input. Discovered: {inputs:?}",
            case.chip,
        );
    }
}

/// Every case's device must be DRIVABLE: `set_input` resolves it and the value
/// reaches the model, observed over the device's own protocol. Discovery alone
/// is not enough — a walk could list a device it cannot dispatch to.
#[test]
fn every_chip_drives_its_attached_input_device() {
    for case in CASES {
        let mut bus = case.build();
        let (channel, value) = case.probe();

        let before = case.read_back(&mut bus);
        assert!(
            (before - value).abs() > 0.5,
            "{}: boot state {before} is already the probe value {value}; \
             this case cannot prove a set landed",
            case.chip,
        );

        bus.set_input(None, channel, value).unwrap_or_else(|e| {
            panic!(
                "{}: set_input(None, {channel:?}, {value}) failed: {e:?}",
                case.chip
            )
        });

        let after = case.read_back(&mut bus);
        assert!(
            (after - value).abs() < 0.5,
            "{}: set_input reported success but the model still reports \
             {after} (expected ~{value}) — the set did not reach the device",
            case.chip,
        );
    }
}

/// Discovery must speak the same vocabulary dispatch accepts: the owner
/// `list_inputs` reports has to be a `component` `set_input` resolves. A walk
/// that reached devices but tagged them with an unusable owner would leave the
/// disambiguated form (two sensors, same channel) broken.
#[test]
fn discovered_owner_is_addressable_by_set_input() {
    for case in CASES {
        let mut bus = case.build();
        let (channel, value) = case.probe();
        let owner = bus
            .list_inputs()
            .into_iter()
            .find(|(_, ch)| ch.key == channel)
            .map(|(owner, _)| owner)
            .unwrap_or_else(|| panic!("{}: `{channel}` not discovered", case.chip));

        bus.set_input(Some(&owner), channel, value)
            .unwrap_or_else(|e| {
                panic!(
                    "{}: owner {owner:?} came from list_inputs but set_input \
                     rejected it: {e:?}",
                    case.chip
                )
            });

        let after = case.read_back(&mut bus);
        assert!(
            (after - value).abs() < 0.5,
            "{}: component-narrowed set did not reach the device ({after})",
            case.chip,
        );
    }
}
