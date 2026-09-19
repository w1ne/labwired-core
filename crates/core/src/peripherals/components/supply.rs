// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! ONE HOME FOR "IS THIS MODULE ACTUALLY POWERED?".
//!
//! The bug this exists for
//! =======================
//! A diagram could wire a display's SIGNAL pins only — SCL/SDA/CS/DC/RES, no
//! VCC and no GND — and the twin would clock in the whole init sequence and
//! report the panel `painted_bytes: 2048, lit: true, display_on: true`. On a
//! bench that panel is dark. The `PWR_SUPPLY_UNCONNECTED` ERC warning tells the
//! AUTHOR, but the simulator still lied, and it lied *structurally*: the
//! compiled manifest carried no supply information of any kind, so the engine
//! could not have known.
//!
//! `packages/board-config/src/compile/emitters.ts` (`unpoweredPartIds`) now
//! emits `powered: false` into a device's `config:` when it can see the part's
//! declared `power_in` pins are on no net. This module is the engine half: the
//! one place that says what that key means, so nine display models and every
//! I²C kit read it the same way.
//!
//! ⚠️ ABSENT MEANS POWERED, ALWAYS
//! ================================
//! Only an explicit `powered: false` darkens anything. Measured over the
//! 551-diagram corpus, 110 powered external devices across 66 lab manifests
//! have ZERO supply pins on any net — every curated
//! `core/examples/**/system.yaml` among them, because curated labs state
//! signals and leave the rails implicit. If a missing key meant "unpowered",
//! all of them would go black. [`powered_from_config`] is the asymmetry, in one
//! function: `None` and `Some(true)` are both powered.
//!
//! Why dark rather than an attach error
//! ====================================
//! It is what the hardware does. An `attach` error would refuse to build a
//! circuit the user can still legitimately want to run, and the *reason* is
//! already on the design-time side as the ERC warning. Each gated model reports
//! `"powered"` in its artifact meta so a dark frame is self-explaining instead
//! of looking like a firmware bug.

use crate::inspect::{Artifact, InspectOpts};
use crate::peripherals::device::I2cDevice;
use crate::peripherals::kit::{AttachCtx, ConfigKey, ConfigType};

/// The `powered` manifest key, declared once so every kit's
/// `peripherals-manifest.json` entry documents it identically.
///
/// (`st7789` predates this module and carries its own panel-specific wording;
/// it is the reference implementation this generalises, not an exception to
/// the semantics.)
pub const POWERED_CONFIG_KEY: ConfigKey = ConfigKey {
    name: std::borrow::Cow::Borrowed("powered"),
    ty: ConfigType::Bool,
    doc: std::borrow::Cow::Borrowed(
        "Whether the module's supply pins (VCC, GND) are connected. \
          Omit for a powered part -- ABSENT MEANS POWERED, because curated \
          labs state signals and leave the rails implicit. Only an explicit \
          `false`, which the diagram compiler emits when it can see the \
          supply pins are on no net, changes anything: the part then ignores \
          its bus entirely and reports the dark/idle state it has on a bench.",
    ),
};

/// Read the `powered` key with the asymmetry the corpus requires.
///
/// `None` (no key at all — every hand-written lab manifest) and `Some(true)`
/// are both powered. `Some(false)` is the only darkening value.
pub fn powered_from_config(ctx: &AttachCtx<'_>) -> bool {
    powered_from_placement(ctx.ext)
}

/// The same reading, straight off a placed device.
///
/// The `gpio_device` primitive does not attach through [`AttachCtx`] — it goes
/// through `SystemBus::attach_declarative_device`, which holds the
/// [`ExternalDevice`] itself — and a second `!= Some(false)` written there is
/// exactly how the asymmetry above would come to be spelled two ways. ONE
/// function, two callers.
pub fn powered_from_placement(ext: &labwired_config::ExternalDevice) -> bool {
    ext.config.get("powered").and_then(|v| v.as_bool()) != Some(false)
}

/// Stamp `"powered": false` into an artifact's meta so a dark frame explains
/// itself. Used by the models whose meta is built generically and by
/// [`UnpoweredI2cDevice`].
pub fn mark_unpowered(mut artifact: Artifact) -> Artifact {
    if let Some(obj) = artifact.meta.as_object_mut() {
        obj.insert("powered".to_string(), serde_json::Value::Bool(false));
    }
    artifact
}

/// An I²C slave with no supply: it is on the board, and it is not on the bus.
///
/// WHY A DECORATOR AND NOT A FIELD ON EACH MODEL. For SPI the dark behaviour is
/// per-model — an ST7789 stops latching commands, a MAX7219 stops shifting, an
/// e-paper stops accepting a frame — so each model owns its own gate. For I²C
/// it is not: the physically honest behaviour of an unpowered slave is
/// identical for all 34 kits and is a *bus* fact, not a model fact. An
/// unpowered chip does not pull SDA low for its address byte, so the master
/// sees a NACK and a scan finds nothing — exactly like the bench. That is
/// [`I2cDevice::claims_address`] returning `false`, and it is the same
/// mechanism the EFR32 controller already uses for an I²C whose pads were never
/// routed (`peripherals/i2c.rs`, `fn resolve`: "NO WIRES, NO DEVICE").
///
/// Writing that gate into 34 models would be 34 chances to miss one — and the
/// declarative kits (`declarative_i2c.rs`, every `configs/devices/*.yaml`) have
/// no per-device Rust to put it in at all. Wrapping at the ONE kit attach path
/// ([`crate::peripherals::kit::AttachCtx::attach_i2c_device`]) covers every
/// present and future I²C kit by construction.
///
/// Everything else forwards, exactly like the bus-trace decorator next to it in
/// `bus/bus_trace.rs`: `address()` so the device still names itself in a
/// listing, `as_any`/`as_any_mut` so downcasts keep working, `artifacts` so a
/// dark panel still reports its (untouched, therefore blank) framebuffer rather
/// than vanishing — with `"powered": false` stamped in so the blank frame says
/// why. A decorator that swallowed artifacts would turn "dark" into "no
/// evidence", which is the failure mode `inspect::DeviceEvidence` exists to
/// end.
pub struct UnpoweredI2cDevice {
    inner: Box<dyn I2cDevice>,
}

impl UnpoweredI2cDevice {
    pub fn new(inner: Box<dyn I2cDevice>) -> Self {
        Self { inner }
    }
}

impl I2cDevice for UnpoweredI2cDevice {
    fn address(&self) -> u8 {
        // The address it WOULD answer to. Forwarded so listings and the bus
        // trace still name the part; `claims_address` is what decides whether
        // it answers, and controllers must resolve through that (see the trait
        // doc), never by comparing `address()`.
        self.inner.address()
    }

    /// THE GATE. No supply, no ACK — so the master's address phase NACKs, an
    /// `i2c scan` reports nothing at this address, and `Wire.endTransmission()`
    /// returns 2, which is what the firmware sees on a bench.
    fn claims_address(&self, _addr: u8) -> bool {
        false
    }

    /// Unreachable in production (nothing resolves to this device), but a
    /// low-level fixture can still call it. An idle I²C bus is held high by its
    /// pull-ups, so the honest byte is 0xFF, not 0x00 — a model returning zeros
    /// would look like a chip answering with a valid all-zero register.
    fn read(&mut self) -> u8 {
        0xFF
    }

    /// Swallowed, not forwarded: an unpowered chip latches nothing. This is
    /// what keeps a display's framebuffer untouched rather than merely
    /// under-reported.
    fn write(&mut self, _data: u8) {}

    fn start(&mut self) {}
    fn stop(&mut self) {}

    fn artifacts(&self, id: &str, opts: &InspectOpts) -> Vec<Artifact> {
        self.inner
            .artifacts(id, opts)
            .into_iter()
            .map(mark_unpowered)
            .collect()
    }

    fn for_each_sim_input(
        &mut self,
        f: &mut dyn FnMut(&mut dyn crate::sim_input::SimInput) -> bool,
    ) -> bool {
        // Forwarded. Stimulus is the physical world telling the sensor what it
        // is measuring; that a part has no supply does not remove it from
        // `list_inputs`, and hiding it would turn a wiring mistake into a
        // confusing "no such input" from `set_input`.
        self.inner.for_each_sim_input(f)
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        self.inner.as_any()
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        self.inner.as_any_mut()
    }

    fn as_sim_input_mut(&mut self) -> Option<&mut dyn crate::sim_input::SimInput> {
        self.inner.as_sim_input_mut()
    }

    fn advance_time_us(&mut self, _us: u64) {
        // NOT forwarded. A free-running sample clock is the chip's own
        // oscillator; with no supply it does not run, so an unpowered sensor
        // must not accrue FIFO samples while the CPU is busy elsewhere.
    }

    fn take_pin_drives(&mut self) -> Vec<(String, bool)> {
        // NOT forwarded either, and for the same reason: an unpowered part
        // cannot assert an interrupt line. Returning nothing leaves the pad
        // wherever the board's pull leaves it, which is what a dead chip does.
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::components::ssd1306;

    /// The POSITIVE control for the decorator itself: the very same model,
    /// unwrapped, answers its address and takes bytes. Without this, "an
    /// unpowered device NACKs" would also pass on a wrapper that broke
    /// everything.
    #[test]
    fn a_powered_i2c_device_answers_its_address_and_latches_bytes() {
        let mut dev = ssd1306(0x3C);
        assert!(dev.claims_address(0x3C), "a powered panel ACKs its address");
        dev.start();
        dev.write(0x40); // Co=0, D/C=1 → data
        dev.write(0xFF);
        dev.stop();
        let art = I2cDevice::artifacts(&dev, "oled", &InspectOpts::default());
        let ink: u64 = art[0].meta["ink_bytes"].as_u64().unwrap();
        assert!(ink > 0, "a powered panel paints");
    }

    /// The FIX: no supply, no ACK. A scan finds nothing at the address.
    #[test]
    fn an_unpowered_i2c_device_nacks_every_address() {
        let dev = UnpoweredI2cDevice::new(Box::new(ssd1306(0x3C)));
        assert!(
            !dev.claims_address(0x3C),
            "an unpowered chip does not pull SDA low for its own address",
        );
        assert!(!dev.claims_address(0x3D));
        assert_eq!(
            dev.address(),
            0x3C,
            "it still names the address it would answer to",
        );
    }

    /// Bytes that reach it anyway (a low-level fixture, or a controller that
    /// resolved some other way) must not change anything.
    #[test]
    fn an_unpowered_i2c_device_never_accumulates_paint() {
        let mut dev = UnpoweredI2cDevice::new(Box::new(ssd1306(0x3C)));
        for _ in 0..5 {
            dev.start();
            dev.write(0x40);
            for _ in 0..64 {
                dev.write(0xFF);
            }
            dev.stop();
        }
        let art = dev.artifacts("oled", &InspectOpts::default());
        assert_eq!(
            art[0].meta["ink_bytes"], 0,
            "frame memory must be untouched, not just under-reported",
        );
    }

    /// The evidence must survive the wrapper, and must say WHY it is blank.
    #[test]
    fn an_unpowered_i2c_device_still_reports_evidence_marked_unpowered() {
        let dev = UnpoweredI2cDevice::new(Box::new(ssd1306(0x3C)));
        let art = dev.artifacts("oled", &InspectOpts::default());
        assert_eq!(art.len(), 1, "artifacts must be forwarded, never swallowed");
        assert_eq!(art[0].meta["powered"], false);
        assert_eq!(art[0].meta["ink_bytes"], 0);
    }

    /// An idle bus is held high by its pull-ups.
    #[test]
    fn an_unpowered_i2c_device_reads_as_an_idle_bus() {
        let mut dev = UnpoweredI2cDevice::new(Box::new(ssd1306(0x3C)));
        assert_eq!(dev.read(), 0xFF, "pull-ups, not a valid all-zero register");
    }

    // ─── The gate is WIRED ─────────────────────────────────────────────────
    //
    // ⚠️ THE TESTS ABOVE BUILD THE DECORATOR BY HAND. They prove it behaves,
    // and prove nothing about whether anything ever puts it on a device — the
    // classic "the guard is not wired to the path that matters". These two go
    // through `SystemBus::from_config`, i.e. a real manifest, the real kit
    // registry and the real `AttachCtx::attach_i2c_device`, and ask the
    // controller what it would answer on the wire.

    fn bus_with_oled(config: &str) -> crate::bus::SystemBus {
        use labwired_config::{ChipDescriptor, SystemManifest};
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let chip = ChipDescriptor::from_file(root.join("../../configs/chips/stm32f103.yaml"))
            .expect("load stm32f103.yaml");
        let manifest: SystemManifest = serde_yaml::from_str(&format!(
            r#"
name: supply-wiring
chip: "../chips/stm32f103.yaml"
cpu_hz: 72_000_000
external_devices:
  - id: "oled"
    type: "oled-ssd1306"
    connection: "i2c1"
    config:
      i2c_address: 0x3C
{config}
"#
        ))
        .expect("parse system manifest");
        crate::bus::SystemBus::from_config(&chip, &manifest).expect("build bus")
    }

    /// Does the controller find anything at 0x3C? This is what an `i2c scan`
    /// and `Wire.endTransmission()` resolve through.
    fn answers_at_0x3c(bus: &crate::bus::SystemBus) -> bool {
        let idx = bus
            .find_peripheral_index_by_name("i2c1")
            .expect("i2c1 must be registered");
        let any = bus.peripherals[idx].dev.as_any().expect("downcastable");
        let ctl = any
            .downcast_ref::<crate::peripherals::i2c::I2c>()
            .expect("i2c1 must be the generic I2c controller");
        assert_eq!(
            ctl.attached_devices().len(),
            1,
            "the OLED must attach either way — an unpowered part is still ON THE BOARD, \
             and a device that vanished from the bus would be a different lie",
        );
        ctl.attached_devices()[0].borrow().claims_address(0x3C)
    }

    /// The POSITIVE control for the wiring: a manifest with no supply key at
    /// all — the shape every curated lab has — still ACKs.
    #[test]
    fn a_manifest_without_the_key_attaches_a_device_that_answers() {
        assert!(
            answers_at_0x3c(&bus_with_oled("")),
            "ABSENT MEANS POWERED: if this ever fails, every shipped I2C lab is dead",
        );
        assert!(
            answers_at_0x3c(&bus_with_oled("      powered: true")),
            "an explicit true is powered too — only `false` silences",
        );
    }

    /// The FIX, end to end: `powered: false` in the manifest reaches the bus
    /// and the address stops answering.
    #[test]
    fn a_manifest_saying_powered_false_attaches_a_device_that_nacks() {
        assert!(
            !answers_at_0x3c(&bus_with_oled("      powered: false")),
            "no supply, no ACK — a scan must find nothing at 0x3C",
        );
    }
}
