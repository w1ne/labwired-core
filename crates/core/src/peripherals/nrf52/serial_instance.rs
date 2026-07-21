// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! nRF52 Serial Instance — SPIM0/TWIM0 at a shared base address.
//!
//! On real nRF52840 silicon the single 4 KB window at 0x40003000 hosts
//! SPIM0, SPIS0, SPI0, TWIM0, TWI0 and TWIS0; the active sub-peripheral
//! is selected by ENABLE (offset 0x500):
//!
//!   ENABLE = 6 → TWIM (I²C master with EasyDMA)
//!   ENABLE = 7 → SPIM (SPI master with EasyDMA)
//!
//! This struct wraps [`Spi`] (Nrf52Spim layout) and [`Nrf52Twim`] behind
//! the same [`Peripheral`] surface. Reads/writes to offset 0x500 update a
//! shared enable field and are forwarded to both sub-peripherals so their
//! internal enable state stays coherent. All other offsets are dispatched
//! to the sub-peripheral selected by the current enable value.

use crate::peripherals::i2c::I2cDevice;
use crate::peripherals::nrf52::twim::Nrf52Twim;
use crate::peripherals::spi::{Spi, SpiDevice, SpiRegisterLayout};
use crate::{Bus, Peripheral, PeripheralTickResult, SimResult};

const OFF_ENABLE: u64 = 0x500;
const ENABLE_TWIM: u32 = 6;
const ENABLE_SPIM: u32 = 7;
const ENABLE_MASK: u32 = 0xF;

/// Unified SPIM0/TWIM0 serial instance at a single MMIO base.
///
/// Holds both sub-peripheral models live; the active one is determined by
/// the ENABLE register at offset 0x500.  Sub-peripheral state is always
/// up-to-date regardless of which is enabled.
#[derive(Debug)]
pub struct Nrf52SerialInstance {
    spim: Spi,
    twim: Nrf52Twim,
    enable: u32,
}

impl Nrf52SerialInstance {
    pub fn new() -> Self {
        Self {
            spim: Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim),
            twim: Nrf52Twim::new(),
            enable: 0,
        }
    }

    /// Attach an I²C slave. `dev` is expected to already be trace-wrapped by the
    /// caller (the nRF52 factory wraps via `bus_trace::wrap_i2c`); this only
    /// forwards to the TWIM's raw push.
    pub fn attach_i2c(&mut self, dev: Box<dyn I2cDevice>) {
        self.twim.push_slave(dev);
    }

    /// Attach a SPI device (already trace-wrapped by the caller); forwards to the
    /// SPIM's raw push.
    pub fn attach_spi(&mut self, dev: Box<dyn SpiDevice>) {
        self.spim.push_device(dev);
    }

    /// The TWIM sub-peripheral's attached I²C slaves, in attach order.
    pub fn attached_i2c_devices(&self) -> &[std::cell::RefCell<Box<dyn I2cDevice>>] {
        self.twim.attached_devices()
    }

    /// The SPIM sub-peripheral's attached SPI devices, in attach order.
    pub fn attached_spi_devices_mut(&mut self) -> &mut [Box<dyn SpiDevice>] {
        &mut self.spim.attached_devices
    }

    fn active(&self) -> u32 {
        self.enable & ENABLE_MASK
    }
}

impl Default for Nrf52SerialInstance {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for Nrf52SerialInstance {
    fn read(&self, offset: u64) -> SimResult<u8> {
        if offset & !3 == OFF_ENABLE {
            let byte_shift = (offset % 4) * 8;
            return Ok(((self.enable & ENABLE_MASK) >> byte_shift) as u8);
        }
        match self.active() {
            ENABLE_TWIM => self.twim.read(offset),
            ENABLE_SPIM => self.spim.read(offset),
            // ENABLE=0: pinctrl writes PSEL before ENABLE is set; shadow to TWIM.
            _ => self.twim.read(offset),
        }
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        if offset & !3 == OFF_ENABLE {
            // Reconstruct a 32-bit RMW before forwarding.
            let byte_shift = (offset % 4) * 8;
            let mask: u32 = 0xFF << byte_shift;
            let new32 = (self.enable & !mask) | ((value as u32) << byte_shift);
            self.enable = new32 & ENABLE_MASK;
            self.twim.write(offset, value)?;
            self.spim.write(offset, value)?;
            return Ok(());
        }
        match self.active() {
            ENABLE_TWIM => self.twim.write(offset, value),
            ENABLE_SPIM => self.spim.write(offset, value),
            // ENABLE=0: pinctrl writes PSEL before ENABLE is set; shadow to TWIM.
            _ => self.twim.write(offset, value),
        }
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        if offset == OFF_ENABLE {
            return Ok(self.enable & ENABLE_MASK);
        }
        match self.active() {
            ENABLE_TWIM => self.twim.read_u32(offset),
            ENABLE_SPIM => self.spim.read_u32(offset),
            // ENABLE=0: pinctrl writes PSEL before ENABLE is set; shadow to TWIM.
            _ => self.twim.read_u32(offset),
        }
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset == OFF_ENABLE {
            self.enable = value & ENABLE_MASK;
            // Keep both sub-peripherals' internal enable coherent.
            self.twim.write_u32(offset, value)?;
            self.spim.write_u32(offset, value)?;
            return Ok(());
        }
        match self.active() {
            ENABLE_TWIM => self.twim.write_u32(offset, value),
            ENABLE_SPIM => self.spim.write_u32(offset, value),
            // ENABLE=0: pinctrl writes PSEL before ENABLE is set; shadow to TWIM.
            _ => self.twim.write_u32(offset, value),
        }
    }

    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        if offset & !3 == OFF_ENABLE {
            return Ok((self.enable & ENABLE_MASK) as u16);
        }
        match self.active() {
            ENABLE_TWIM => self.twim.read_u16(offset),
            ENABLE_SPIM => self.spim.read_u16(offset),
            _ => self.twim.read_u16(offset),
        }
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        if offset & !3 == OFF_ENABLE {
            self.enable = (value as u32) & ENABLE_MASK;
            self.twim.write_u16(offset, value)?;
            self.spim.write_u16(offset, value)?;
            return Ok(());
        }
        match self.active() {
            ENABLE_TWIM => self.twim.write_u16(offset, value),
            ENABLE_SPIM => self.spim.write_u16(offset, value),
            _ => self.twim.write_u16(offset, value),
        }
    }

    fn tick(&mut self) -> PeripheralTickResult {
        match self.active() {
            ENABLE_TWIM => self.twim.tick(),
            ENABLE_SPIM => self.spim.tick(),
            _ => PeripheralTickResult::default(),
        }
    }

    fn needs_bus_tick(&self) -> bool {
        match self.active() {
            ENABLE_TWIM => self.twim.needs_bus_tick(),
            ENABLE_SPIM => self.spim.needs_bus_tick(),
            _ => false,
        }
    }

    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        match self.active() {
            ENABLE_TWIM => self.twim.tick_with_bus(bus),
            ENABLE_SPIM => self.spim.tick_with_bus(bus),
            _ => {}
        }
    }

    fn on_event(
        &mut self,
        event_token: u32,
        sched: &mut crate::sched::EventScheduler,
        bus: &mut dyn Bus,
    ) -> crate::sched::EventResult {
        match self.active() {
            ENABLE_TWIM => self.twim.on_event(event_token, sched, bus),
            ENABLE_SPIM => self.spim.on_event(event_token, sched, bus),
            _ => crate::sched::EventResult::default(),
        }
    }

    fn uses_scheduler(&self) -> bool {
        match self.active() {
            ENABLE_TWIM => self.twim.uses_scheduler(),
            ENABLE_SPIM => self.spim.uses_scheduler(),
            _ => false,
        }
    }

    fn sync_to(&mut self, tick_now: u64) {
        match self.active() {
            ENABLE_TWIM => self.twim.sync_to(tick_now),
            ENABLE_SPIM => self.spim.sync_to(tick_now),
            _ => {}
        }
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        match self.active() {
            ENABLE_TWIM => self.twim.take_scheduled_events(),
            ENABLE_SPIM => self.spim.take_scheduled_events(),
            _ => Vec::new(),
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    /// Forward the stimulus walk into BOTH sub-peripherals.
    ///
    /// The mux owns its devices (the nRF52 factory attaches manifest externals
    /// straight into `twim`/`spim`), so a walk that stopped at this struct
    /// would report no inputs at all for every device on the shared
    /// SPIM0/TWIM0 window — discoverable by nothing, drivable by nothing.
    /// Both sub-models are held live regardless of ENABLE, so both are walked:
    /// a sensor must stay addressable while the mux happens to be in the other
    /// mode.
    fn for_each_attached_sim_input(
        &mut self,
        f: &mut dyn FnMut(&mut dyn crate::sim_input::SimInput) -> bool,
    ) -> bool {
        self.twim.for_each_attached_sim_input(f) || self.spim.for_each_attached_sim_input(f)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "enable": self.enable,
            "spim": self.spim.snapshot(),
            "twim": self.twim.snapshot(),
        })
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        if let Some(v) = state.get("enable").and_then(|v| v.as_u64()) {
            self.enable = (v as u32) & ENABLE_MASK;
        }
        if let Some(spim_state) = state.get("spim") {
            self.spim.restore(spim_state.clone())?;
        }
        if let Some(twim_state) = state.get("twim") {
            self.twim.restore(twim_state.clone())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Bus, DmaRequest, SimulationConfig};
    use std::collections::HashMap;

    // ── Minimal flat-RAM bus ──────────────────────────────────────────────────

    struct FlatRam {
        mem: HashMap<u64, u8>,
        config: SimulationConfig,
    }

    impl FlatRam {
        fn new() -> Self {
            Self {
                mem: HashMap::new(),
                config: SimulationConfig::default(),
            }
        }

        fn write_slice(&mut self, base: u64, data: &[u8]) {
            for (i, &b) in data.iter().enumerate() {
                self.mem.insert(base + i as u64, b);
            }
        }

        fn read_slice(&self, base: u64, len: usize) -> Vec<u8> {
            (0..len)
                .map(|i| *self.mem.get(&(base + i as u64)).unwrap_or(&0))
                .collect()
        }
    }

    impl Bus for FlatRam {
        fn read_u8(&self, addr: u64) -> crate::SimResult<u8> {
            Ok(*self.mem.get(&addr).unwrap_or(&0))
        }
        fn write_u8(&mut self, addr: u64, value: u8) -> crate::SimResult<()> {
            self.mem.insert(addr, value);
            Ok(())
        }
        fn tick_peripherals(&mut self) -> Vec<u32> {
            Vec::new()
        }
        fn execute_dma(&mut self, _requests: &[DmaRequest]) -> crate::SimResult<()> {
            Ok(())
        }
        fn config(&self) -> &SimulationConfig {
            &self.config
        }
    }

    fn write32(s: &mut Nrf52SerialInstance, offset: u64, value: u32) {
        s.write_u32(offset, value).unwrap();
    }

    fn read32(s: &Nrf52SerialInstance, offset: u64) -> u32 {
        s.read_u32(offset).unwrap()
    }

    // ── ENABLE readback ───────────────────────────────────────────────────────

    #[test]
    fn enable_readback_twim() {
        let mut s = Nrf52SerialInstance::new();
        write32(&mut s, OFF_ENABLE, ENABLE_TWIM);
        assert_eq!(read32(&s, OFF_ENABLE), ENABLE_TWIM);
    }

    #[test]
    fn enable_readback_spim() {
        let mut s = Nrf52SerialInstance::new();
        write32(&mut s, OFF_ENABLE, ENABLE_SPIM);
        assert_eq!(read32(&s, OFF_ENABLE), ENABLE_SPIM);
    }

    #[test]
    fn enable_mask_4_bits() {
        let mut s = Nrf52SerialInstance::new();
        write32(&mut s, OFF_ENABLE, 0xFF);
        assert_eq!(read32(&s, OFF_ENABLE), 0xF);
    }

    // ── TWIM path: muxed BME280 chip-id read (0xD0 → 0x60) ──────────────────
    //
    // Sequence mirrors a typical nRF52 I²C register read:
    //   1. TX  [reg_addr]          — set the register pointer to 0xD0
    //   2. RX  1 byte              — read the chip-id
    //
    // BME280 Bme280::read_register(0xD0) returns 0x60 per the silicon model.

    #[test]
    fn twim_path_reads_bme280_chip_id() {
        let bme280 = crate::peripherals::components::Bme280::new(0x76);
        let mut s = Nrf52SerialInstance::new();
        s.attach_i2c(Box::new(bme280));
        let mut bus = FlatRam::new();

        // TX buffer: one byte = register address 0xD0.
        let tx_base: u64 = 0x2000_0000;
        let rx_base: u64 = 0x2000_0100;
        bus.write_slice(tx_base, &[0xD0u8]);

        // Enable TWIM.
        write32(&mut s, OFF_ENABLE, ENABLE_TWIM);

        // Set I²C target address (BME280 = 0x76).
        write32(&mut s, 0x588, 0x76); // ADDRESS

        // TX descriptor: send register address byte.
        write32(&mut s, 0x544, tx_base as u32); // TXD.PTR
        write32(&mut s, 0x548, 1); // TXD.MAXCNT = 1

        // SHORT: LASTTX→STARTRX (write-then-read in one transaction).
        write32(&mut s, 0x200, 1 << 7); // SHORTS: LASTTX_STARTRX

        // RX descriptor: receive one byte.
        write32(&mut s, 0x534, rx_base as u32); // RXD.PTR
        write32(&mut s, 0x538, 1); // RXD.MAXCNT = 1

        // Fire TASKS_STARTTX.
        write32(&mut s, 0x008, 1);
        assert!(s.needs_bus_tick(), "pending TX must be set");

        // Drive the EasyDMA engine to completion. The TWIM model now imposes
        // realistic wire latency (see Nrf52Twim::transfer_cycles), so the
        // TX→RX chain spans many ticks rather than two; drain until idle.
        let mut guard = 0u32;
        while s.needs_bus_tick() {
            s.tick_with_bus(&mut bus);
            guard += 1;
            assert!(guard < 5_000_000, "muxed TWIM transfer never completed");
        }

        let chip_id = bus.read_slice(rx_base, 1);
        assert_eq!(chip_id[0], 0x60, "BME280 chip-id must be 0x60");

        // EVENTS_LASTRX must be set (nRF52840 TWIM offset 0x15C = bit-23).
        assert_eq!(read32(&s, 0x15C), 1, "EVENTS_LASTRX must be 1");
    }

    // ── SPIM path: EasyDMA loopback through the mux ───────────────────────────

    #[test]
    fn spim_path_easydma_events_end() {
        let mut s = Nrf52SerialInstance::new();
        let mut bus = FlatRam::new();

        let tx_base: u64 = 0x2000_0200;
        let rx_base: u64 = 0x2000_0300;
        bus.write_slice(tx_base, &[0xAA, 0xBB, 0xCC]);

        // Enable SPIM.
        write32(&mut s, OFF_ENABLE, ENABLE_SPIM);

        write32(&mut s, 0x544, tx_base as u32); // TXD.PTR
        write32(&mut s, 0x548, 3); // TXD.MAXCNT
        write32(&mut s, 0x534, rx_base as u32); // RXD.PTR
        write32(&mut s, 0x538, 3); // RXD.MAXCNT

        // TASKS_START (nRF52 SPIM offset 0x010).
        write32(&mut s, 0x010, 1);
        assert!(s.needs_bus_tick(), "SPIM pending after TASKS_START");

        s.tick_with_bus(&mut bus);

        // EVENTS_END must be 1.
        assert_eq!(
            read32(&s, 0x118),
            1,
            "EVENTS_END must be 1 after SPIM transfer"
        );
        assert_eq!(read32(&s, 0x120), 1, "EVENTS_ENDTX must be 1");
        assert_eq!(read32(&s, 0x110), 1, "EVENTS_ENDRX must be 1");

        // TXD.AMOUNT == 3.
        assert_eq!(read32(&s, 0x54C), 3, "TXD.AMOUNT");
        assert!(!s.needs_bus_tick(), "no pending after SPIM tick");
    }

    // ── Dispatch isolation ────────────────────────────────────────────────────
    // When TWIM is active a write to a TWIM-specific offset must not bleed
    // SPIM state (and vice versa).  Light smoke: write to TWIM ADDRESS (0x588)
    // while TWIM is active, then switch to SPIM and verify EVENTS_END is still 0.

    // ── Pre-enable PSEL shadow ────────────────────────────────────────────────
    // nrfx / Zephyr pinctrl writes PSEL.SCL and PSEL.SDA _before_ ENABLE is
    // written (they share offset space 0x508/0x50C with SPIM PSEL).  The mux
    // must shadow those writes to the TWIM model so nrfx_twim_init can read
    // them back without calling nrf_gpio_pin_present_check on 0x7FFFFFFF.

    #[test]
    fn psel_writes_before_enable_are_shadowed_to_twim() {
        let mut s = Nrf52SerialInstance::new();

        // ENABLE=0 at construction; write PSEL.SCL (P0.27) and PSEL.SDA (P0.26)
        // the way Zephyr pinctrl does before writing ENABLE.
        write32(&mut s, 0x508, 27); // PSEL.SCL
        write32(&mut s, 0x50C, 26); // PSEL.SDA

        // Now enable TWIM; the stored PSEL values must be readable.
        write32(&mut s, OFF_ENABLE, ENABLE_TWIM);
        assert_eq!(
            read32(&s, 0x508),
            27,
            "PSEL.SCL written before ENABLE must survive into TWIM mode"
        );
        assert_eq!(
            read32(&s, 0x50C),
            26,
            "PSEL.SDA written before ENABLE must survive into TWIM mode"
        );
    }

    #[test]
    fn dispatch_isolation_twim_write_does_not_set_spim_events() {
        let mut s = Nrf52SerialInstance::new();

        // Enable TWIM and poke a TWIM-specific register.
        write32(&mut s, OFF_ENABLE, ENABLE_TWIM);
        write32(&mut s, 0x588, 0x42); // ADDRESS (TWIM-only)

        // Switch to SPIM — no transfer was initiated; EVENTS_END must be 0.
        write32(&mut s, OFF_ENABLE, ENABLE_SPIM);
        assert_eq!(
            read32(&s, 0x118),
            0,
            "EVENTS_END must be 0 after TWIM-side write with no SPIM transfer"
        );
        assert!(
            !s.needs_bus_tick(),
            "no pending bus tick on SPIM when no transfer started"
        );
    }
}
