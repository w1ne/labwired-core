// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! TCA9548A / PCA9548A — 8-channel bidirectional I²C switch.
//!
//! The part exists to solve exactly one problem: several slaves that share a
//! **fixed, unchangeable** address (four VCNL4010 proximity sensors are all
//! 0x13, and the part has no address-strap pin) cannot coexist on one bus. The
//! switch sits between the master and eight downstream bus segments and passes
//! SDA/SCL through to whichever segments its control register enables.
//!
//! ## Silicon behaviour modelled (TI SCPS207, §8.3 / §8.5)
//!
//! * **Own address** `1110 A2 A1 A0` → 0x70 … 0x77 from the three strap pins.
//! * **One register.** There is no register pointer and no sub-addressing: a
//!   write to the switch's own address loads the control register, a read from
//!   it returns the control register. Bit *n* enables channel *n*.
//! * **Reset state** is 0x00 — every channel disabled. (Firmware that forgets
//!   to select a channel therefore talks to nothing, which is the real failure
//!   and must stay visible.)
//! * **The control register is a bitmask, not a selector.** Firmware CAN
//!   legally enable several channels at once; the datasheet says so and drivers
//!   do it to broadcast a configuration write to identical sensors. See
//!   [`Tca9548a::read`] for what the model does when that produces two talkers.
//! * **Pass-through is transparent.** The switch is an analogue pass-gate, not
//!   a repeater: it does not buffer, re-address, or delay. So a downstream
//!   device sees exactly the START / address / data / STOP the master issued.
//!
//! ## What is NOT modelled
//!
//! * The `RESET` pin (active-low, clears the control register). There is no
//!   GPIO seam on the [`I2cDevice`] trait to observe a pad from, so a firmware
//!   that resets the switch by toggling that pin is not reproduced. Firmware
//!   that resets it by writing 0x00 — the common path — is.
//! * Channel-to-channel leakage, V_OL budgeting, and the 400 kHz timing spec:
//!   this engine models the byte protocol, not the analogue bus.

use crate::peripherals::i2c::I2cDevice;

/// Lowest address the strap pins can produce (A2=A1=A0=0).
pub const TCA9548A_BASE_ADDR: u8 = 0x70;

/// Number of downstream bus segments.
pub const TCA9548A_CHANNELS: usize = 8;

/// Which device(s) the master's currently selected address resolves to.
///
/// Resolved once per address phase in [`I2cDevice::select_address`] and held
/// for the rest of the transaction, mirroring the pass-gates the switch closes
/// for the duration of a transfer.
#[derive(Debug, Default, Clone)]
struct Selection {
    /// The master addressed the switch itself → the control register.
    control: bool,
    /// `(channel, index)` of every downstream device on an ENABLED channel that
    /// claims the selected address. More than one entry is a genuine bus
    /// collision, deliberately preserved — see [`Tca9548a::read`].
    downstream: Vec<(usize, usize)>,
}

/// 8-channel I²C switch. Holds the devices on each channel and forwards the
/// master's transactions to the ones its control register currently exposes.
pub struct Tca9548a {
    /// 7-bit address decoded from the A2/A1/A0 straps (0x70 … 0x77).
    address: u8,
    /// The one control register. Bit *n* = channel *n* pass-gates closed.
    /// Reset value 0x00 (all channels isolated).
    control: u8,
    /// Devices on each downstream segment. Index = channel number.
    channels: [Vec<Box<dyn I2cDevice>>; TCA9548A_CHANNELS],
    /// Target of the transaction in flight.
    selected: Selection,
}

impl Tca9548a {
    /// Switch at an explicit 7-bit address. Addresses outside 0x70..=0x77
    /// cannot be produced by the real part's straps, so they are clamped into
    /// range rather than silently modelling an impossible board.
    pub fn new(address: u8) -> Self {
        Self {
            address: TCA9548A_BASE_ADDR + (address & 0x07),
            control: 0x00,
            channels: std::array::from_fn(|_| Vec::new()),
            selected: Selection::default(),
        }
    }

    /// Switch at the strap-decoded address `0x70 | (a2<<2) | (a1<<1) | a0`.
    pub fn with_straps(a0: bool, a1: bool, a2: bool) -> Self {
        Self::new(TCA9548A_BASE_ADDR | (u8::from(a0)) | (u8::from(a1) << 1) | (u8::from(a2) << 2))
    }

    /// Attach `device` to downstream segment `channel` (0..8). Out-of-range
    /// channels are rejected so a mis-wired manifest fails loudly at build time
    /// instead of producing a device that is attached to nothing.
    pub fn attach(&mut self, channel: u8, device: Box<dyn I2cDevice>) -> anyhow::Result<()> {
        let ch = channel as usize;
        if ch >= TCA9548A_CHANNELS {
            anyhow::bail!(
                "TCA9548A at 0x{:02x}: channel {} is out of range (0..={})",
                self.address,
                channel,
                TCA9548A_CHANNELS - 1
            );
        }
        self.channels[ch].push(device);
        Ok(())
    }

    /// Current control-register value: bit *n* set = channel *n* enabled.
    pub fn control_register(&self) -> u8 {
        self.control
    }

    /// Whether channel `channel` is currently passed through to the master.
    pub fn channel_enabled(&self, channel: u8) -> bool {
        (channel as usize) < TCA9548A_CHANNELS && (self.control >> channel) & 1 != 0
    }

    /// Devices attached to `channel`, in attach order. Enabled or not — this is
    /// the physical wiring, not the switch state.
    pub fn channel_devices(&self, channel: u8) -> &[Box<dyn I2cDevice>] {
        match self.channels.get(channel as usize) {
            Some(v) => v,
            None => &[],
        }
    }

    /// Every `(channel, index)` on an ENABLED channel whose device claims
    /// `addr`. Order is channel-ascending, then attach order.
    fn enabled_claimants(&self, addr: u8) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for (ch, devices) in self.channels.iter().enumerate() {
            if (self.control >> ch) & 1 == 0 {
                continue;
            }
            for (idx, dev) in devices.iter().enumerate() {
                if dev.claims_address(addr) {
                    out.push((ch, idx));
                }
            }
        }
        out
    }

    /// Run `f` over every currently selected downstream device.
    ///
    /// Take-and-restore rather than clone: this runs on every byte of every
    /// transaction, and a per-byte Vec allocation on the bus path is pure
    /// overhead. Nothing `f` does re-derives the selection (only
    /// `select_address` writes it, and it is not reachable from here), so the
    /// list handed back is the one that was taken.
    fn for_each_selected(&mut self, mut f: impl FnMut(&mut Box<dyn I2cDevice>)) {
        let selected = std::mem::take(&mut self.selected.downstream);
        for &(ch, idx) in &selected {
            f(&mut self.channels[ch][idx]);
        }
        self.selected.downstream = selected;
    }
}

impl Default for Tca9548a {
    fn default() -> Self {
        Self::new(TCA9548A_BASE_ADDR)
    }
}

impl std::fmt::Debug for Tca9548a {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let populated: Vec<(usize, usize)> = self
            .channels
            .iter()
            .enumerate()
            .filter(|(_, d)| !d.is_empty())
            .map(|(ch, d)| (ch, d.len()))
            .collect();
        f.debug_struct("Tca9548a")
            .field("address", &format_args!("0x{:02x}", self.address))
            .field("control", &format_args!("0b{:08b}", self.control))
            .field("channels", &populated)
            .finish()
    }
}

impl I2cDevice for Tca9548a {
    /// The switch's OWN address. Resolution never goes through this — see
    /// [`I2cDevice::claims_address`] — but the bus trace uses it to label the
    /// device and tooling uses it to identify the part.
    fn address(&self) -> u8 {
        self.address
    }

    /// The switch answers to its own control address at all times, plus every
    /// address reachable through a channel its control register currently
    /// enables. That second set changes on every control-register write, which
    /// is exactly why this cannot be flattened into a static `address()`.
    fn claims_address(&self, addr: u8) -> bool {
        if addr == self.address {
            return true;
        }
        self.channels.iter().enumerate().any(|(ch, devices)| {
            (self.control >> ch) & 1 != 0 && devices.iter().any(|d| d.claims_address(addr))
        })
    }

    /// Latch what this transaction is for.
    ///
    /// Note that the switch's own address and a downstream device are NOT
    /// mutually exclusive: on real hardware the pass-gates are closed while the
    /// switch still decodes its own address, so a (mis-)wired downstream device
    /// that shares the switch's address collides with the control register.
    /// The datasheet warns against that layout; the model reproduces it instead
    /// of hiding it.
    fn select_address(&mut self, addr: u8) {
        self.selected = Selection {
            control: addr == self.address,
            downstream: self.enabled_claimants(addr),
        };
        // The switch is the master as far as the downstream segment is
        // concerned, so it owes each selected device the same selection the
        // controller owes it. Skipping this works for a plain slave (whose
        // `select_address` is a no-op) and silently breaks a switch behind a
        // switch, which would then forward every byte to whatever it had
        // selected last.
        self.for_each_selected(|d| d.select_address(addr));
    }

    /// START is an analogue event on the shared wire: every downstream device
    /// behind a closed pass-gate sees it, and so does the switch.
    fn start(&mut self) {
        self.for_each_selected(|d| d.start());
    }

    fn stop(&mut self) {
        self.for_each_selected(|d| d.stop());
    }

    /// A byte clocked toward the selected target(s).
    ///
    /// Addressed at its own address, the switch has exactly one register: every
    /// byte written loads the control register (last byte wins), with no
    /// register pointer to advance. Otherwise the byte is passed through to
    /// every selected downstream device — all of them, because they are
    /// electrically parallel and each really does receive it.
    ///
    /// The new channel mask takes effect for the NEXT address phase, not
    /// mid-byte: silicon latches the control register on that byte's ACK, so
    /// the pass-gates that carried the byte are the ones selected at the
    /// preceding address phase. The common firmware shape — write the mask,
    /// repeated-START to the sensor — works because the repeated START runs a
    /// fresh [`select_address`](I2cDevice::select_address).
    fn write(&mut self, data: u8) {
        if self.selected.control {
            self.control = data;
        }
        self.for_each_selected(|d| d.write(data));
    }

    /// A byte clocked back from the selected target(s).
    ///
    /// * **Switch selected** → the control register, repeated for as many bytes
    ///   as the master clocks (there is no pointer to advance).
    /// * **Exactly one downstream talker** → its byte, unchanged. This is the
    ///   normal case and is byte-identical to talking to the device directly.
    /// * **Several talkers** → the bitwise AND of what each drives, which is
    ///   what the wire physically carries. I²C SDA is open-drain with a pull-up:
    ///   a device asserts a `0` by pulling the line down and a `1` by releasing
    ///   it, so the bus is a wired-AND and any device driving `0` wins that bit.
    ///   Every talker is still clocked (each `read()` really happens), because
    ///   on hardware each one advances its own output shift register whether or
    ///   not its bits survive the collision — that side effect is part of the
    ///   corruption. The result is garbage, deterministically: enabling two
    ///   channels that hold the same sensor is a firmware bug, and the model's
    ///   job is to reproduce the garbled read instead of silently picking a
    ///   winner.
    fn read(&mut self) -> u8 {
        let mut byte = if self.selected.control {
            self.control
        } else if self.selected.downstream.is_empty() {
            // Nothing selected: the bus is left to the pull-ups.
            return 0xFF;
        } else {
            0xFF
        };
        self.for_each_selected(|d| byte &= d.read());
        byte
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    /// Advance EVERY downstream device, on every channel, enabled or not.
    ///
    /// A sensor behind an open pass-gate is still powered and still sampling on
    /// its own oscillator — the switch only disconnects it from the master's
    /// wire. Advancing only the enabled channels would mean a FIFO stopped
    /// filling the moment firmware looked away, which is precisely the
    /// CPU-starvation / overrun class this hook exists to expose.
    fn advance_time_us(&mut self, us: u64) {
        for devices in self.channels.iter_mut() {
            for dev in devices.iter_mut() {
                dev.advance_time_us(us);
            }
        }
    }

    /// The switch itself drives nothing physical, but the sensors behind it do.
    /// Walk every channel so a device on a mux stays reachable from
    /// `list_inputs` / `set_input` — a mux must not subtract stimulus
    /// reachability.
    fn for_each_sim_input(
        &mut self,
        f: &mut dyn FnMut(&mut dyn crate::sim_input::SimInput) -> bool,
    ) -> bool {
        for devices in self.channels.iter_mut() {
            for dev in devices.iter_mut() {
                if dev.for_each_sim_input(f) {
                    return true;
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal downstream slave: answers at one address, returns a constant
    /// tag, counts the microseconds handed to it. Deliberately NOT a real
    /// sensor — this file tests the switch, not a part model.
    struct Tag {
        address: u8,
        value: u8,
        started: usize,
        stopped: usize,
        written: Vec<u8>,
        elapsed_us: u64,
    }

    impl Tag {
        fn new(address: u8, value: u8) -> Self {
            Self {
                address,
                value,
                started: 0,
                stopped: 0,
                written: Vec::new(),
                elapsed_us: 0,
            }
        }
    }

    impl I2cDevice for Tag {
        fn address(&self) -> u8 {
            self.address
        }
        fn read(&mut self) -> u8 {
            self.value
        }
        fn write(&mut self, data: u8) {
            self.written.push(data);
        }
        fn start(&mut self) {
            self.started += 1;
        }
        fn stop(&mut self) {
            self.stopped += 1;
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

    fn four_identical() -> Tca9548a {
        let mut mux = Tca9548a::new(0x70);
        for ch in 0..4u8 {
            mux.attach(ch, Box::new(Tag::new(0x13, 0xA0 + ch))).unwrap();
        }
        mux
    }

    /// Select `channel`, then read one byte from `addr`.
    fn read_via(mux: &mut Tca9548a, channel: u8, addr: u8) -> u8 {
        mux.select_address(mux.address());
        mux.start();
        mux.write(1 << channel);
        mux.stop();

        assert!(mux.claims_address(addr));
        mux.select_address(addr);
        mux.start();
        let b = mux.read();
        mux.stop();
        b
    }

    #[test]
    fn straps_decode_the_seven_addresses() {
        assert_eq!(Tca9548a::with_straps(false, false, false).address(), 0x70);
        assert_eq!(Tca9548a::with_straps(true, false, false).address(), 0x71);
        assert_eq!(Tca9548a::with_straps(false, true, false).address(), 0x72);
        assert_eq!(Tca9548a::with_straps(true, true, true).address(), 0x77);
        assert_eq!(Tca9548a::default().address(), 0x70);
    }

    #[test]
    fn reset_state_isolates_every_channel() {
        let mux = four_identical();
        assert_eq!(mux.control_register(), 0x00);
        assert!(mux.claims_address(0x70), "own address always answers");
        assert!(
            !mux.claims_address(0x13),
            "no channel is enabled at reset, so nothing downstream is reachable"
        );
    }

    /// The whole point of the part: four sensors that CANNOT be re-addressed,
    /// each independently reachable behind its own channel.
    #[test]
    fn four_devices_at_one_address_are_independently_reachable() {
        let mut mux = four_identical();
        for ch in 0..4u8 {
            assert_eq!(
                read_via(&mut mux, ch, 0x13),
                0xA0 + ch,
                "channel {ch} must answer with its own device"
            );
        }
    }

    #[test]
    fn switching_channels_changes_who_answers() {
        let mut mux = four_identical();
        assert_eq!(read_via(&mut mux, 2, 0x13), 0xA2);
        assert_eq!(read_via(&mut mux, 0, 0x13), 0xA0);
        assert_eq!(read_via(&mut mux, 3, 0x13), 0xA3);
    }

    #[test]
    fn control_register_reads_back_and_is_a_bitmask() {
        let mut mux = four_identical();
        mux.select_address(0x70);
        mux.start();
        mux.write(0b0000_0101);
        mux.stop();

        mux.select_address(0x70);
        mux.start();
        assert_eq!(mux.read(), 0b0000_0101);
        // No register pointer: a second byte repeats the same register.
        assert_eq!(mux.read(), 0b0000_0101);
        mux.stop();

        assert!(mux.channel_enabled(0));
        assert!(!mux.channel_enabled(1));
        assert!(mux.channel_enabled(2));
    }

    #[test]
    fn a_disabled_channel_does_not_answer() {
        let mut mux = four_identical();
        mux.select_address(0x70);
        mux.write(1 << 1);
        assert!(mux.claims_address(0x13));
        mux.select_address(0x13);
        assert_eq!(mux.read(), 0xA1);

        // Isolate everything: the address must stop being claimed, so the
        // controller NACKs exactly as an empty bus does.
        mux.select_address(0x70);
        mux.write(0x00);
        assert!(!mux.claims_address(0x13));
    }

    /// Two channels enabled at once is legal on silicon and firmware does it.
    /// Both sensors drive SDA on the read; the open-drain bus carries the
    /// wired-AND, so the master gets corruption — not one device's answer.
    #[test]
    fn simultaneous_channels_collide_as_a_wired_and() {
        let mut mux = Tca9548a::new(0x70);
        mux.attach(0, Box::new(Tag::new(0x13, 0b1111_0000)))
            .unwrap();
        mux.attach(1, Box::new(Tag::new(0x13, 0b1100_1100)))
            .unwrap();

        mux.select_address(0x70);
        mux.write(0b0000_0011); // both channels
        mux.select_address(0x13);
        mux.start();
        let b = mux.read();
        mux.stop();

        assert_eq!(
            b, 0b1100_0000,
            "an open-drain bus with two talkers carries the AND of what they drive"
        );
        assert_ne!(b, 0b1111_0000, "must not silently pick the first channel");
        assert_ne!(b, 0b1100_1100, "must not silently pick the last channel");
    }

    /// A broadcast write with several channels enabled reaches every device —
    /// the reason drivers enable multiple channels on purpose.
    #[test]
    fn simultaneous_channels_broadcast_a_write_to_all_of_them() {
        let mut mux = four_identical();
        mux.select_address(0x70);
        mux.write(0b0000_1111);

        mux.select_address(0x13);
        mux.start();
        mux.write(0x5A);
        mux.stop();

        for ch in 0..4u8 {
            let tag = mux.channel_devices(ch)[0]
                .as_any()
                .unwrap()
                .downcast_ref::<Tag>()
                .unwrap();
            assert_eq!(tag.written, vec![0x5A], "channel {ch}");
            assert_eq!(tag.started, 1, "channel {ch}");
            assert_eq!(tag.stopped, 1, "channel {ch}");
        }
    }

    /// Free-running sensor clocks do not stop because a pass-gate is open.
    #[test]
    fn advance_time_reaches_every_channel_including_disabled_ones() {
        let mut mux = four_identical();
        mux.select_address(0x70);
        mux.write(0b0000_0001); // only channel 0 is connected to the master

        mux.advance_time_us(2_500);

        for ch in 0..4u8 {
            let tag = mux.channel_devices(ch)[0]
                .as_any()
                .unwrap()
                .downcast_ref::<Tag>()
                .unwrap();
            assert_eq!(
                tag.elapsed_us, 2_500,
                "channel {ch} sensor keeps sampling while isolated"
            );
        }
    }

    #[test]
    fn repeated_start_after_a_control_write_sees_the_new_mask() {
        // Firmware writes the channel mask and then, without a STOP, issues a
        // repeated START to the sensor — the shape every TCA9548A driver uses.
        let mut mux = four_identical();
        mux.select_address(0x70);
        mux.start();
        mux.write(1 << 3);
        // repeated START, no stop
        mux.select_address(0x13);
        mux.start();
        assert_eq!(mux.read(), 0xA3);
    }

    #[test]
    fn out_of_range_channel_is_rejected() {
        let mut mux = Tca9548a::new(0x70);
        assert!(mux.attach(8, Box::new(Tag::new(0x13, 1))).is_err());
        assert!(mux.attach(7, Box::new(Tag::new(0x13, 1))).is_ok());
    }

    #[test]
    fn unselected_read_releases_the_bus() {
        let mut mux = four_identical();
        mux.select_address(0x55); // nothing claims this
        assert_eq!(mux.read(), 0xFF, "pull-ups hold SDA high");
    }

    /// Nested switches: the recursion in `claims_address` / `select_address`
    /// means a switch behind a switch works with no extra code.
    #[test]
    fn a_switch_behind_a_switch_routes_through() {
        let mut inner = Tca9548a::new(0x71);
        inner.attach(5, Box::new(Tag::new(0x13, 0xEE))).unwrap();

        let mut outer = Tca9548a::new(0x70);
        outer.attach(0, Box::new(inner)).unwrap();

        // Enable outer channel 0, which exposes the inner switch's own address.
        outer.select_address(0x70);
        outer.write(0b0000_0001);
        assert!(outer.claims_address(0x71));

        // Program the inner switch through the outer one.
        outer.select_address(0x71);
        outer.start();
        outer.write(1 << 5);
        outer.stop();

        assert!(outer.claims_address(0x13));
        outer.select_address(0x13);
        outer.start();
        assert_eq!(outer.read(), 0xEE);
    }
}
