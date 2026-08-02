// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! TM1637 4-digit 7-segment LED display (2-wire, bit-banged).
//!
//! The TM1637 (e.g. the RobotDyn 4-digit clock module) is not on a hardware bus:
//! the MCU bit-bangs two GPIO lines, `CLK` and `DIO`, with an I²C-like — but
//! **LSB-first and address-less** — protocol. Because both lines are MCU
//! outputs while the host writes display data, the model observes them exactly
//! the way [`HcSr04`](crate::peripherals::hc_sr04::HcSr04) observes its TRIG
//! line: the [`SystemBus`](crate::bus::SystemBus) re-reads the two GPIO output
//! bits after every MMIO write that touches the hosting port and feeds the
//! `(clk, dio)` levels to [`Tm1637::observe_lines`]. No polling, no timers — the
//! state machine advances on the same edges the firmware produces.
//!
//! Protocol decoded:
//!   * **Start** — `DIO` falls while `CLK` is high.
//!   * **Stop**  — `DIO` rises while `CLK` is high.
//!   * **Data**  — `DIO` sampled on each `CLK` rising edge, 8 bits **LSB-first**,
//!     followed by a 9th ACK clock (the chip pulls `DIO` low; ignored here).
//!   * **Commands** — data command `0x40`/`0x44` (auto-increment vs fixed
//!     address), address command `0xC0..=0xC5`, display control `0x80..=0x8F`
//!     (bit 3 = on, bits 0..2 = brightness).
//!
//! The six display grids are decoded through the standard a–g/dp segment font;
//! [`Tm1637::text`] renders the four leftmost digits a human would read.

/// GRID registers the TM1637 exposes (the 4-digit module wires the first four).
const GRIDS: usize = 6;

use crate::peripherals::components::seven_seg_font;

/// One TM1637 display wired to a `CLK` output pin and a `DIO` output pin.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Tm1637 {
    /// board_io / external-device id.
    pub id: String,
    /// Absolute address + bit of the `CLK` GPIO **output** register (ODR).
    pub clk_odr_addr: u64,
    pub clk_bit: u8,
    /// Absolute address + bit of the `DIO` GPIO **output** register (ODR).
    pub dio_odr_addr: u64,
    pub dio_bit: u8,
    /// Cached peripheral indices of the GPIO ports hosting CLK / DIO, resolved
    /// lazily on first use by the bus write-hook. `None` until resolved.
    #[serde(skip)]
    clk_peripheral_idx: Option<usize>,
    #[serde(skip)]
    dio_peripheral_idx: Option<usize>,

    // ─── Line-level protocol state ───
    prev_clk: bool,
    prev_dio: bool,
    in_transaction: bool,
    /// True once 8 data bits are in; the next CLK rising edge is the ACK.
    expecting_ack: bool,
    bit_buf: u8,
    bit_count: u8,
    /// Command byte of the current transaction (`None` until the first byte).
    txn_command: Option<u8>,

    // ─── Persistent display state ───
    /// False after a `0x44` fixed-address data command; auto-increment (0x40)
    /// otherwise. Persists across transactions, per the datasheet.
    auto_increment: bool,
    /// GRID write pointer set by the address command, advanced on each data
    /// byte when `auto_increment` is set.
    addr_pointer: u8,
    display_on: bool,
    brightness: u8,
    grids: [u8; GRIDS],
}

impl Tm1637 {
    pub fn new(id: String, clk_odr_addr: u64, clk_bit: u8, dio_odr_addr: u64, dio_bit: u8) -> Self {
        Self {
            id,
            clk_odr_addr,
            clk_bit,
            dio_odr_addr,
            dio_bit,
            clk_peripheral_idx: None,
            dio_peripheral_idx: None,
            // Idle bus is both lines high.
            prev_clk: true,
            prev_dio: true,
            in_transaction: false,
            expecting_ack: false,
            bit_buf: 0,
            bit_count: 0,
            txn_command: None,
            auto_increment: true,
            addr_pointer: 0,
            display_on: false,
            brightness: 0,
            grids: [0; GRIDS],
        }
    }

    pub(crate) fn clk_peripheral_idx(&self) -> Option<usize> {
        self.clk_peripheral_idx
    }
    pub(crate) fn set_clk_peripheral_idx(&mut self, idx: usize) {
        self.clk_peripheral_idx = Some(idx);
    }
    pub(crate) fn dio_peripheral_idx(&self) -> Option<usize> {
        self.dio_peripheral_idx
    }
    pub(crate) fn set_dio_peripheral_idx(&mut self, idx: usize) {
        self.dio_peripheral_idx = Some(idx);
    }

    /// Raw latched segment byte for GRID `i` (`0b0gfedcba`, dp = bit 7).
    pub fn grid(&self, i: usize) -> u8 {
        self.grids.get(i).copied().unwrap_or(0)
    }

    /// Whether the panel is switched on, and its 0..7 brightness.
    pub fn display_on(&self) -> bool {
        self.display_on
    }
    pub fn brightness(&self) -> u8 {
        self.brightness
    }

    /// Colon segment (dp of GRID 1) — lit on the clock-style 4-digit module.
    pub fn colon(&self) -> bool {
        self.grids[1] & 0x80 != 0
    }

    fn decode(seg: u8) -> char {
        seven_seg_font::decode(seg)
    }

    /// The four leftmost digits, decoded to characters.
    pub fn chars(&self) -> [char; 4] {
        let mut out = [' '; 4];
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = Self::decode(self.grids[i]);
        }
        out
    }

    /// The four leftmost digits as a string.
    pub fn text(&self) -> String {
        self.chars().iter().collect()
    }

    /// Feed the current `(CLK, DIO)` output levels. Called by the bus write-hook
    /// after any GPIO write that could have moved either line. Reconstructs the
    /// bit-bang edges by diffing against the previously observed levels.
    pub fn observe_lines(&mut self, clk: bool, dio: bool) {
        let clk_rose = !self.prev_clk && clk;
        let clk_steady_high = self.prev_clk && clk;

        // Start / stop are DIO edges while CLK stays high.
        if clk_steady_high && dio != self.prev_dio {
            if self.prev_dio && !dio {
                self.begin_transaction(); // DIO ↓ with CLK high → start
            } else {
                self.in_transaction = false; // DIO ↑ with CLK high → stop
            }
        } else if clk_rose && self.in_transaction {
            if self.expecting_ack {
                // 9th clock: chip ACKs by pulling DIO low; we consume it and
                // reset for the next byte in the same transaction.
                self.expecting_ack = false;
                self.bit_buf = 0;
                self.bit_count = 0;
            } else {
                // Sample one data bit, LSB-first.
                if dio {
                    self.bit_buf |= 1 << self.bit_count;
                }
                self.bit_count += 1;
                if self.bit_count == 8 {
                    let byte = self.bit_buf;
                    self.handle_byte(byte);
                    self.expecting_ack = true;
                }
            }
        }

        self.prev_clk = clk;
        self.prev_dio = dio;
    }

    fn begin_transaction(&mut self) {
        self.in_transaction = true;
        self.expecting_ack = false;
        self.bit_buf = 0;
        self.bit_count = 0;
        self.txn_command = None;
    }

    fn handle_byte(&mut self, byte: u8) {
        match self.txn_command {
            None => {
                // First byte of the transaction is a command.
                self.txn_command = Some(byte);
                match byte & 0xC0 {
                    0x40 => {
                        // Data command: bit 2 selects fixed (1) vs auto (0).
                        self.auto_increment = byte & 0x04 == 0;
                    }
                    0xC0 => {
                        // Address command: low bits select the GRID pointer.
                        self.addr_pointer = byte & 0x07;
                    }
                    0x80 => {
                        // Display control: bit 3 = on, bits 0..2 = brightness.
                        self.display_on = byte & 0x08 != 0;
                        self.brightness = byte & 0x07;
                    }
                    _ => { /* unsupported command — ignore */ }
                }
            }
            Some(_) => {
                // Subsequent bytes are display data for the address pointer.
                let slot = (self.addr_pointer as usize) % GRIDS;
                self.grids[slot] = byte;
                if self.auto_increment {
                    self.addr_pointer = self.addr_pointer.wrapping_add(1) % GRIDS as u8;
                }
            }
        }
    }
}

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct Tm16377SegKit;
pub static TM1637_7SEG_KIT: Tm16377SegKit = Tm16377SegKit;

static TM1637_7SEG_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "tm1637-7seg",
    label: "TM1637 7-Segment (4-digit)",
    summary: "4-digit 7-segment LED display with the TM1637 2-wire (CLK/DIO) driver.",
    detail: "The RobotDyn-style 4-digit clock display. The host bit-bangs two GPIO lines with the \
             TM1637's I²C-like, LSB-first, address-less protocol; the model observes both output \
             pins through the bus write-hook, decodes start/stop/data/ACK framing and the data / \
             address / display-control commands, and renders the four grids through the standard \
             a–g/dp segment font (including the clock colon).",
    transport: Transport::GpioGroup,
    category: Category::Gpio,
    config_keys: &[
        ConfigKey {
            name: "clk_pin",
            ty: ConfigType::Str,
            doc: "CLK GPIO pin the firmware bit-bangs (e.g. \"PA8\"). Defaults to PA8.",
        },
        ConfigKey {
            name: "dio_pin",
            ty: ConfigType::Str,
            doc: "DIO GPIO pin the firmware bit-bangs (e.g. \"PA9\"). Defaults to PA9.",
        },
    ],
    // No lab yet: examples/tm1637-7seg-lab has only a README + system.yaml — no demo
    // firmware/ELF is built or published. Declaring a LabRef would promise a
    // one-click demo that 404s (the playground gate rightly rejects it).
    // Re-add the LabRef when the demo firmware ships.
    labs: &[],
};

impl PeripheralKit for Tm16377SegKit {
    fn metadata(&self) -> &'static KitMetadata {
        &TM1637_7SEG_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let clk = ctx.config_str("clk_pin").unwrap_or("PA8").to_string();
        let dio = ctx.config_str("dio_pin").unwrap_or("PA9").to_string();
        let (clk_addr, clk_bit) = ctx.resolve_pin_odr(&clk).ok_or_else(|| {
            anyhow::anyhow!(
                "TM1637 '{}' clk_pin '{}' could not be resolved to a GPIO",
                ctx.device_id(),
                clk
            )
        })?;
        let (dio_addr, dio_bit) = ctx.resolve_pin_odr(&dio).ok_or_else(|| {
            anyhow::anyhow!(
                "TM1637 '{}' dio_pin '{}' could not be resolved to a GPIO",
                ctx.device_id(),
                dio
            )
        })?;
        let id = ctx.device_id().to_string();
        ctx.bus
            .tm1637
            .push(Tm1637::new(id, clk_addr, clk_bit, dio_addr, dio_bit));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive one bit-banged byte, LSB-first, plus the ACK clock. Assumes the
    /// bus is idle-high on entry and leaves CLK high, DIO high.
    fn send_byte(dev: &mut Tm1637, byte: u8) {
        for i in 0..8 {
            let bit = (byte >> i) & 1 != 0;
            dev.observe_lines(false, bit); // CLK low, set data
            dev.observe_lines(true, bit); // CLK rising → sample
        }
        // ACK clock: master releases DIO high, chip would pull low; one clock.
        dev.observe_lines(false, true);
        dev.observe_lines(true, true);
        // Return CLK low between bytes.
        dev.observe_lines(false, true);
    }

    fn start(dev: &mut Tm1637) {
        dev.observe_lines(true, true); // idle high
        dev.observe_lines(true, false); // DIO ↓ while CLK high → start
        dev.observe_lines(false, false); // CLK low to begin clocking
    }

    fn stop(dev: &mut Tm1637) {
        dev.observe_lines(false, false);
        dev.observe_lines(true, false);
        dev.observe_lines(true, true); // DIO ↑ while CLK high → stop
    }

    fn new_dev() -> Tm1637 {
        Tm1637::new("seg".into(), 0x4001_080C, 0, 0x4001_080C, 1)
    }

    #[test]
    fn writes_four_digits_with_auto_increment() {
        let mut dev = new_dev();
        // Data command: auto-increment.
        start(&mut dev);
        send_byte(&mut dev, 0x40);
        stop(&mut dev);
        // Address command (GRID 0) + four segment bytes for "1234".
        start(&mut dev);
        send_byte(&mut dev, 0xC0);
        send_byte(&mut dev, 0x06); // '1'
        send_byte(&mut dev, 0x5B); // '2'
        send_byte(&mut dev, 0x4F); // '3'
        send_byte(&mut dev, 0x66); // '4'
        stop(&mut dev);
        // Display on, full brightness.
        start(&mut dev);
        send_byte(&mut dev, 0x8F);
        stop(&mut dev);

        assert_eq!(dev.text(), "1234");
        assert!(dev.display_on());
        assert_eq!(dev.brightness(), 7);
    }

    #[test]
    fn fixed_address_mode_targets_single_grid() {
        let mut dev = new_dev();
        start(&mut dev);
        send_byte(&mut dev, 0x44); // fixed address
        stop(&mut dev);
        start(&mut dev);
        send_byte(&mut dev, 0xC2); // GRID 2
        send_byte(&mut dev, 0x3F); // '0'
        stop(&mut dev);
        assert_eq!(dev.chars()[2], '0');
        assert_eq!(dev.chars()[0], ' ');
    }

    /// End-to-end through the bus: bit-bang CLK/DIO by writing a real GPIO ODR
    /// register and confirm the write-hook drives the display's state machine.
    #[test]
    fn driven_through_bus_write_hook() {
        use crate::bus::SystemBus;
        use crate::peripherals::gpio::{GpioPort, GpioRegisterLayout};

        const GPIOA: u64 = 0x4800_0000; // stm32v2: ODR @ 0x14, BSRR @ 0x18
        const ODR: u64 = GPIOA + 0x14;
        const BSRR: u64 = GPIOA + 0x18;
        const CLK: u8 = 8; // PA8
        const DIO: u8 = 9; // PA9

        let mut bus = SystemBus::empty();
        bus.add_peripheral(
            "gpioa",
            GPIOA,
            0x400,
            None,
            Box::new(GpioPort::new_with_layout(GpioRegisterLayout::Stm32V2)),
        );
        bus.tm1637
            .push(Tm1637::new("seg".into(), ODR, CLK, ODR, DIO));

        // Drive the lines through BSRR (atomic set/reset), the way real GPIO
        // bit-bang code does. Writes go through the `Bus` trait method — the
        // same path CPU MMIO takes — so the maybe_clock_tm1637 write-hook fires.
        use crate::Bus;
        let set = |bus: &mut SystemBus, clk: bool, dio: bool| {
            let bit = |b: u8, hi: bool| if hi { 1u32 << b } else { 1u32 << (b + 16) };
            let v = bit(CLK, clk) | bit(DIO, dio);
            Bus::write_u32(bus, BSRR, v).unwrap();
        };
        let byte = |bus: &mut SystemBus, b: u8| {
            for i in 0..8 {
                let bit = (b >> i) & 1 != 0;
                set(bus, false, bit);
                set(bus, true, bit);
            }
            set(bus, false, true); // ACK clock
            set(bus, true, true);
            set(bus, false, true);
        };

        // Idle → start.
        set(&mut bus, true, true);
        set(&mut bus, true, false); // start
        set(&mut bus, false, false);
        byte(&mut bus, 0x40); // data cmd
        set(&mut bus, true, false);
        set(&mut bus, true, true); // stop
                                   // Address cmd + one digit.
        set(&mut bus, true, false); // start
        set(&mut bus, false, false);
        byte(&mut bus, 0xC0);
        byte(&mut bus, 0x6D); // '5'
        set(&mut bus, true, false);
        set(&mut bus, true, true); // stop

        assert_eq!(bus.tm1637[0].chars()[0], '5');
    }

    #[test]
    fn colon_bit_is_reported() {
        let mut dev = new_dev();
        start(&mut dev);
        send_byte(&mut dev, 0x40);
        stop(&mut dev);
        start(&mut dev);
        send_byte(&mut dev, 0xC0);
        send_byte(&mut dev, 0x06); // grid0 '1'
        send_byte(&mut dev, 0x5B | 0x80); // grid1 '2' + colon
        stop(&mut dev);
        assert!(dev.colon());
    }
}
