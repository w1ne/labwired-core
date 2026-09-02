// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! ONE HOME FOR THE OFF-CHIP DEVICE VOCABULARY.
//!
//! Why this module exists
//! ======================
//! An *off-chip device* — a sensor on an I²C bus, a panel on SPI, a GPS on a
//! UART, a WS2812 strip watching a pad — is not part of any MCU. It is the
//! other side of a wire. The traits that say what such a thing can do are
//! therefore the engine's stable public vocabulary, and they were living
//! inside the four MCU-peripheral files that happened to call them first:
//! `I2cDevice` in `peripherals::i2c` next to the STM32 CR1/CR2 register file,
//! `SpiDevice` in `peripherals::spi`, `UartStreamDevice`/`UartStreamHost` in
//! `peripherals::uart`, and `GpioObserver` declared TWICE — once in
//! `peripherals::esp32::gpio` and once, byte for byte, in
//! `peripherals::esp32s3::gpio`.
//!
//! That placement is what makes a device change an engine change. It also
//! produced the duplication directly: a component that wanted to watch pad
//! edges on both ESP32 families had to write the same `impl` twice, because
//! the two families each owned a private copy of the same three-line contract.
//! Six components did (`servo`, `ws2812`, `step_dir_motor`, `h_bridge_motor`,
//! `unipolar_stepper`, `ili9341_parallel`) and every call site that accepted
//! an observer carried a two-trait bound (`SystemBus::install_gpio_observer`,
//! `AttachCtx::install_gpio_observer`) that no third family could satisfy
//! without a third declaration and a third `impl` on all six.
//!
//! What is here
//! ============
//! The traits themselves, moved verbatim. **Nothing else changed**: the old
//! paths keep `pub use` re-exports, so every `impl`, bound, and intra-doc link
//! outside this module still resolves, and the behaviour is bit-identical.
//!
//! This is the cheap half of the split the remediation ledger's C-1 row is
//! about. Whether or not a `labwired-peripheral-api` crate is ever created,
//! the vocabulary now has one home to read and one place to change; if it is
//! created, this file is a `git mv` rather than surgery on four register
//! models.
//!
//! What is deliberately NOT here
//! =============================
//! The MCU-side controllers (`Uart`, `Spi`, `I2c`, `Esp32Gpio`,
//! `Esp32s3Gpio`). Those are register models of silicon and belong with their
//! families. The line this module draws is the wire, not the bus protocol.

use std::any::Any;
use std::sync::mpsc::{Receiver, Sender};

// ── I²C ─────────────────────────────────────────────────────────────────────
pub trait I2cDevice: Send {
    fn address(&self) -> u8;
    fn read(&mut self) -> u8;
    fn write(&mut self, data: u8);
    fn start(&mut self) {}
    fn stop(&mut self) {}

    /// What this device can show of itself — its own inspect evidence.
    ///
    /// The ONE place a an I²C device's artifacts are decided is the model
    /// itself, next to the buffers it owns. Default: nothing, which is correct
    /// for a sensor with no display surface and honest for anything else —
    /// absent means "this engine has nothing to show", never "the screen was
    /// blank". See [`crate::inspect::DeviceEvidence`] for why this is not a
    /// central match on concrete types.
    ///
    /// Implementations must read the model's REAL buffer and synthesize
    /// nothing; a panel that was never painted reports zero.
    fn artifacts(
        &self,
        _id: &str,
        _opts: &crate::inspect::InspectOpts,
    ) -> Vec<crate::inspect::Artifact> {
        Vec::new()
    }

    /// Does this device answer to `addr` on the wire *right now*?
    ///
    /// A plain slave owns exactly one address, so the default is the obvious
    /// `self.address() == addr` — every existing model keeps its behaviour with
    /// no edit. The hook exists for devices whose answered-address set is not a
    /// singleton and is not static: an I²C **bus switch** (TCA9548A) answers to
    /// its own control address *and*, while a channel is enabled, to every
    /// address reachable behind that channel. That set changes whenever
    /// firmware rewrites the switch's control register, so it cannot be
    /// flattened into one `address()` at attach time.
    ///
    /// Controllers MUST resolve a slave with this, never by comparing
    /// `address()` — a flat `position(|d| d.address() == addr)` is first-match
    /// and makes four identical sensors behind a mux collapse into one.
    fn claims_address(&self, addr: u8) -> bool {
        self.address() == addr
    }

    /// Tell the device which address the master just selected, immediately
    /// after [`claims_address`](Self::claims_address) returned `true` for it and
    /// before any `start`/`write`/`read`/`stop` of that transaction.
    ///
    /// Default no-op: a single-address slave already knows who it is. A bus
    /// switch uses it to decide whether this transaction targets its own
    /// control register or is to be forwarded to the downstream device(s) that
    /// claim `addr` on the currently enabled channel(s).
    fn select_address(&mut self, addr: u8) {
        let _ = addr;
    }

    /// Walk every [`SimInput`](crate::sim_input::SimInput) surface this device
    /// exposes, including devices nested *behind* it. Returns `true` if `f`
    /// asked to stop early.
    ///
    /// The default is exactly the old behaviour — a device offers at most its
    /// own [`as_sim_input_mut`](Self::as_sim_input_mut). It is overridden by
    /// containers (the TCA9548A mux) so their children stay reachable from the
    /// ONE stimulus walk in [`crate::bus::SystemBus::for_each_sim_input`].
    /// Without it, putting a sensor behind a mux would silently subtract it
    /// from `list_inputs` / `set_input` — the same class of invisible-device
    /// bug the controller-level seam was introduced to kill.
    fn for_each_sim_input(
        &mut self,
        f: &mut dyn FnMut(&mut dyn crate::sim_input::SimInput) -> bool,
    ) -> bool {
        match self.as_sim_input_mut() {
            Some(si) => f(si),
            None => false,
        }
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        None
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        None
    }
    /// Runtime-drivable view of this device, if it accepts simulated input.
    /// Overridden by input devices (accelerometers, …) so the generic
    /// [`crate::Machine::set_input`] resolver can reach them without a
    /// downcast. Default `None` = not an input device.
    fn as_sim_input_mut(&mut self) -> Option<&mut dyn crate::sim_input::SimInput> {
        None
    }

    /// Advance this device's free-running sample/measurement clock by `us`
    /// microseconds of wall-clock time.
    ///
    /// Real sensors sample on their own oscillator, independent of when the CPU
    /// gets around to reading them: a PPG FIFO keeps filling at its configured
    /// rate whether or not firmware is draining it. A bus master that knows the
    /// elapsed wall-clock calls this on a slave immediately before servicing it,
    /// so a *late* poll observes exactly the samples that accrued while the CPU
    /// was busy elsewhere — and a FIFO that was allowed to overrun reports the
    /// overflow it really would have. Without this hook a model only advances on
    /// the very transactions that would have prevented the overflow, which hides
    /// precisely the CPU-starvation failures worth simulating.
    ///
    /// Default no-op: a purely register-mapped device has no clock to advance.
    fn advance_time_us(&mut self, _us: u64) {}
}

// ── SPI ─────────────────────────────────────────────────────────────────────
/// Trait implemented by simulated SPI devices (peripherals attached to an SPI bus).
///
/// For v1, CS-pin-aware routing is not implemented: all transfers are broadcast
/// to every attached device and the first non-zero MISO byte wins.  This is
/// correct for single-device labs (MAX31855 alone).  CS-aware routing is noted
/// as a Phase 2 follow-up.
pub trait SpiDevice: Send {
    fn needs_external_bus_poll(&self) -> bool {
        false
    }
    fn component_id(&self) -> Option<&str> {
        None
    }
    fn attach_can_bus(
        &mut self,
        _tx: Sender<crate::network::CanFrame>,
        _rx: Receiver<crate::network::CanFrame>,
    ) -> anyhow::Result<()> {
        anyhow::bail!("SPI device is not a CAN controller")
    }
    fn poll_external_bus(&mut self) {}
    /// Called when the CS line goes low (chip is selected).
    fn cs_select(&mut self) {}
    /// Called when the CS line goes high (chip is released — flush state).
    fn cs_release(&mut self) {}
    /// SPI is full-duplex: master sends `mosi_byte`, device returns its current MISO byte.
    /// On read-only devices like MAX31855, `mosi_byte` is ignored.
    fn transfer(&mut self, mosi_byte: u8) -> u8;
    /// CS pin label this device is wired to (e.g. "PA4" or numeric pin ID). Used by the bus
    /// dispatcher to pick which device responds when the firmware drives a particular CS line.
    fn cs_pin(&self) -> &str;

    /// What this device can show of itself — its own inspect evidence.
    ///
    /// The ONE place a a SPI device's artifacts are decided is the model
    /// itself, next to the buffers it owns. Default: nothing, which is correct
    /// for a sensor with no display surface and honest for anything else —
    /// absent means "this engine has nothing to show", never "the screen was
    /// blank". See [`crate::inspect::DeviceEvidence`] for why this is not a
    /// central match on concrete types.
    ///
    /// Implementations must read the model's REAL buffer and synthesize
    /// nothing; a panel that was never painted reports zero.
    fn artifacts(
        &self,
        _id: &str,
        _opts: &crate::inspect::InspectOpts,
    ) -> Vec<crate::inspect::Artifact> {
        Vec::new()
    }
    /// Data/Command (D/C) pin label this device observes, if any (e.g. "PB6").
    ///
    /// Displays like the Nokia 5110 (PCD8544) distinguish command bytes from
    /// pixel-data bytes by the level of a dedicated GPIO line rather than by
    /// byte semantics. When this returns `Some(pin)`, the bus latches that
    /// pin's current output level into the device via [`set_dc_level`] after
    /// each MMIO write, so the value is current by the time the firmware
    /// writes the SPI data register. Default `None` → the bus does no latching
    /// and the device infers framing from the protocol (ILI9341 / SSD1680).
    ///
    /// [`set_dc_level`]: SpiDevice::set_dc_level
    fn dc_pin(&self) -> Option<&str> {
        None
    }
    /// Latched level of the [`dc_pin`](SpiDevice::dc_pin) at transfer time,
    /// pushed by the bus. No-op for devices that do not observe a D/C line.
    fn set_dc_level(&mut self, _level: bool) {}
    /// Resolved `(ODR address, bit)` of the D/C line. The bus computes this
    /// once at install time (from [`dc_pin`](SpiDevice::dc_pin)) and records it
    /// via [`set_dc_source`]; thereafter the bus reads that GPIO output bit
    /// just before each transfer and pushes the level via [`set_dc_level`].
    /// Default `None` → no D/C latching.
    ///
    /// [`set_dc_source`]: SpiDevice::set_dc_source
    fn dc_source(&self) -> Option<(u64, u8)> {
        None
    }
    /// Bus-side setter recording the resolved D/C `(ODR address, bit)`.
    fn set_dc_source(&mut self, _odr_addr: u64, _bit: u8) {}
    fn as_any(&self) -> Option<&dyn Any> {
        None
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        None
    }
    /// Runtime-drivable view of this device, if it accepts simulated input.
    /// Same contract as the hook on `I2cDevice`: input devices override it so
    /// the generic [`crate::Machine::set_input`] resolver can reach them
    /// without a downcast. Default `None` = not an input device.
    fn as_sim_input_mut(&mut self) -> Option<&mut dyn crate::sim_input::SimInput> {
        None
    }
    /// Binary mid-flight snapshot for runtime resume. Default empty;
    /// override for stateful devices (e-paper panels with framebuffers,
    /// thermocouples with cached temperatures, etc.).
    fn runtime_snapshot(&self) -> Vec<u8> {
        Vec::new()
    }
    fn restore_runtime_snapshot(&mut self, _bytes: &[u8]) -> crate::SimResult<()> {
        Ok(())
    }
}

// ── UART stream ─────────────────────────────────────────────────────────────
/// A UART model that can host a [`UartStreamDevice`] — the contract an
/// inter-chip cross-link binds to.
///
/// This exists so the cross-link seam names a CAPABILITY rather than one
/// concrete struct. It used to downcast to [`Uart`], which silently excluded
/// every chip family with its own UART model: an ESP32-C3's `uart1` reported
/// "is not a UART" and two C3s could not be wired together at all. A new UART
/// model now joins by implementing this trait, not by editing the seam.
pub trait UartStreamHost {
    /// Bind a peer to this UART's RX/TX paths.
    fn attach_stream_device(&mut self, dev: Box<dyn UartStreamDevice>);

    /// Stop mirroring TX to the console/capture sink. A cross-linked UART
    /// carries raw protocol octets, not console text, and letting those into
    /// the serial monitor floods it with binary that looks identical on both
    /// peers.
    fn detach_console_sink(&mut self);

    /// True when any attached peer carries protocol octets — the test
    /// `attach_uart_tx_sink` uses to leave a linked UART off the console sink.
    fn hosts_protocol_peer(&self) -> bool;
}

/// A device that emits bytes through the UART's RX path (e.g. a GPS module).
pub trait UartStreamDevice: Send {
    /// Called periodically by the bus tick. Returns the next byte to push into UART RX,
    /// or None if no byte is pending. Implementations should respect `elapsed_us` to
    /// pace output (e.g. 9600 baud → ~1 ms/byte → emit one byte per ~1000 us tick).
    fn poll(&mut self, elapsed_us: u32) -> Option<u8>;
    /// Observe a byte transmitted by firmware on the TX path. Default: ignore.
    /// Bidirectional peers (e.g. an IO-Link master) override this to receive the
    /// device's responses, complementing `poll` which drives the RX path.
    fn on_tx_byte(&mut self, _byte: u8) {}

    /// True when this peer's traffic is raw protocol octets rather than console
    /// text, so the UART hosting it must be kept OFF the console capture sink.
    ///
    /// Without this, a node's link UART and its console UART push into the same
    /// buffer and the two byte streams splice together — a two-chip run prints
    /// `pPiInNgGer up` and no serial assertion can be trusted. Default `false`:
    /// an ordinary stream device (a GPS emitting NMEA) is console-safe.
    fn carries_protocol_octets(&self) -> bool {
        false
    }
    fn as_any(&self) -> Option<&dyn Any> {
        None
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        None
    }
    /// Runtime-drivable view of this device, if it accepts simulated input.
    /// Same contract as the hook on `I2cDevice`: input devices override it so
    /// the generic [`crate::Machine::set_input`] resolver can reach them
    /// without a downcast. Default `None` = not an input device.
    fn as_sim_input_mut(&mut self) -> Option<&mut dyn crate::sim_input::SimInput> {
        None
    }
}

// ── I2S ─────────────────────────────────────────────────────────────────────
/// A device on a serial-audio (I2S) bus.
///
/// I2S is not SPI with different framing, and modelling it as bytes on a wire
/// loses the one thing that decides whether a recording works: WHICH CHANNEL a
/// slot belongs to. So the unit here is a 32-bit slot plus the state of the
/// word clock, not a byte.
///
/// Per the EFR32xG26 reference manual section 20.3.3.8 (p.629): "A word
/// transmitted while the word clock is low is for the left channel, and a word
/// transmitted while the word clock is high is for the right." Every I2S part
/// picks a side, and a part addressed on the other side is silent rather than
/// broken -- which is why `right` is an argument and not something the device
/// infers.
pub trait I2sDevice: Send {
    /// The device's next 32-bit slot for this channel, MSB-aligned.
    ///
    /// A device that does not drive the requested channel returns 0: on real
    /// hardware its output is high-Z and the bus pulldown wins, so silence is
    /// the honest answer rather than the other channel's sample.
    fn next_slot(&mut self, right: bool) -> u32;

    /// Human-facing id, for evidence and stimulus routing.
    fn component_id(&self) -> Option<&str> {
        None
    }

    fn artifacts(
        &self,
        _id: &str,
        _opts: &crate::inspect::InspectOpts,
    ) -> Vec<crate::inspect::Artifact> {
        Vec::new()
    }

    /// Both halves are required for a downcast to work from outside. A
    /// one-sided impl compiles, passes every unit test, and wires nothing.
    fn as_any(&self) -> Option<&dyn Any> {
        None
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        None
    }
    fn as_sim_input_mut(&mut self) -> Option<&mut dyn crate::sim_input::SimInput> {
        None
    }
}

// ── GPIO edge observation ───────────────────────────────────────────────────
/// Notified synchronously inside the bus write path on every GPIO pin
/// transition. Observers must not panic — a panic propagates out of
/// `bus.write_u8` and crashes the simulator.
pub trait GpioObserver: Send + Sync + std::fmt::Debug {
    fn on_pin_change(&self, pin: u8, from: bool, to: bool, sim_cycle: u64);
}
