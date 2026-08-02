// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Test-only fixture shared by the per-controller TCA9548A coverage tests.
//!
//! Every I²C controller family resolves its slave through
//! [`I2cDevice::claims_address`] + [`I2cDevice::select_address`] so a bus
//! switch can answer for the devices behind its enabled channels. That change
//! is mechanical, compiles everywhere, and leaves every pre-existing test green
//! — which is exactly why each family needs a switch actually driven through
//! ITS controller. The controller tests live next to their register models (the
//! offsets and command opcodes they poke are private module constants), so what
//! they share is this fixture, not a test file.
//!
//! One [`ChannelTag`] per channel, all at the SAME fixed address, each with its
//! own tag byte: a family that resolves the address once and caches it, or that
//! falls back to a stale `SLAVE_ADDR`, answers with the wrong tag and fails.

use crate::peripherals::components::tca9548a::Tca9548a;
use crate::peripherals::i2c::I2cDevice;

/// Switch address used by every family's test (A2=A1=A0=0).
pub(crate) const MUX_ADDR: u8 = 0x70;

/// The downstream sensors' shared, unchangeable address. Modelled on the
/// VCNL4010, which has no strap pin — the reason a switch is the only way to
/// run several of them on one bus.
pub(crate) const SENSOR_ADDR: u8 = 0x13;

/// Tag byte reported by the device on channel `ch`.
pub(crate) fn tag_for(channel: u8) -> u8 {
    0xA0 + channel
}

/// Minimal downstream slave: answers at one address, returns a constant tag on
/// every read, and records what was written to it. Deliberately NOT a real
/// sensor — these tests cover the controllers' address resolution, not a part
/// model.
pub(crate) struct ChannelTag {
    address: u8,
    tag: u8,
    /// Every data byte the master clocked into this device.
    pub(crate) written: Vec<u8>,
    pub(crate) starts: usize,
    pub(crate) stops: usize,
    /// Microseconds this device has been told have elapsed.
    pub(crate) elapsed_us: u64,
}

impl ChannelTag {
    pub(crate) fn new(address: u8, tag: u8) -> Self {
        Self {
            address,
            tag,
            written: Vec::new(),
            starts: 0,
            stops: 0,
            elapsed_us: 0,
        }
    }
}

impl I2cDevice for ChannelTag {
    fn address(&self) -> u8 {
        self.address
    }
    fn read(&mut self) -> u8 {
        self.tag
    }
    fn write(&mut self, data: u8) {
        self.written.push(data);
    }
    fn start(&mut self) {
        self.starts += 1;
    }
    fn stop(&mut self) {
        self.stops += 1;
    }
    fn advance_time_us(&mut self, us: u64) {
        self.elapsed_us += us;
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

/// A switch at [`MUX_ADDR`] carrying `channels` sensors, one per channel
/// 0..`channels`, ALL at [`SENSOR_ADDR`], tagged [`tag_for`].
pub(crate) fn mux_with_tags(channels: u8) -> Tca9548a {
    let mut mux = Tca9548a::new(MUX_ADDR);
    for ch in 0..channels {
        mux.attach(ch, Box::new(ChannelTag::new(SENSOR_ADDR, tag_for(ch))))
            .expect("channel in range");
    }
    mux
}

/// The bytes `channel`'s sensor received, read out of the switch's wiring
/// rather than over the bus — so an assertion about delivery is independent of
/// the read path being asserted elsewhere.
pub(crate) fn bytes_written_to(mux: &Tca9548a, channel: u8) -> Vec<u8> {
    mux.channel_devices(channel)[0]
        .as_any()
        .and_then(|a| a.downcast_ref::<ChannelTag>())
        .expect("channel holds a ChannelTag")
        .written
        .clone()
}
