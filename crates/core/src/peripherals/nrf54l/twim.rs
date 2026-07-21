// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF54L TWIM (I²C master with EasyDMA), the nRF54L-generation layout.
//!
//! Source: Nordic MDK SVD `nrf54l15_application.svd`, peripheral
//! `GLOBAL_TWIM21_S`, with SHORTS/ENABLE bit values from
//! `nrf54l15_application_peripherals.h`.
//!
//! **This is NOT the nRF52 TWIM at a different base**, for the same reason the
//! UARTE is not: the nRF54L generation moved EasyDMA into a `DMA.{RX,TX}`
//! cluster and renumbered the tasks and events.
//!
//! | function          | nRF52 | nRF54L |
//! |-------------------|-------|--------|
//! | start TX          | 0x008 | 0x050 (`TASKS_DMA.TX.START`) |
//! | start RX          | 0x000 | 0x028 (`TASKS_DMA.RX.START`) |
//! | TX complete       | 0x160 `EVENTS_LASTTX` | 0x168 (`EVENTS_DMA.TX.END`) |
//! | RX complete       | 0x15C `EVENTS_LASTRX` | 0x14C (`EVENTS_DMA.RX.END`) |
//! | TX buffer pointer | 0x544 | 0x73C (`DMA.TX.PTR`) |
//! | RX buffer pointer | 0x534 | 0x704 (`DMA.RX.PTR`) |
//!
//! `EVENTS_STOPPED` (0x104), `ENABLE` (0x500, value 6), `FREQUENCY` (0x524) and
//! `ADDRESS` (0x588) sit at the same offsets on both generations — the same
//! class of coincidence that disguised the UARTE incompatibility.
//!
//! `EVENTS_LASTRX`/`LASTTX` still exist (0x134/0x138) but moved, and the SHORTS
//! bit positions are unchanged from nRF52 (LASTTX_DMA_RX_START = 7,
//! LASTTX_SUSPEND = 8, LASTTX_STOP = 9, LASTRX_DMA_TX_START = 10,
//! LASTRX_STOP = 12), so the canonical write-then-repeated-START-read sequence
//! is driven exactly as before.
//!
//! Transfers are modelled as instantaneous: the whole buffer moves in the tick
//! the start task is written, and the completion events fire together. Real
//! silicon takes ~9 bit-times per byte; nothing in the boot paths modelled here
//! depends on that latency, and a driver that polls sees the same end state.
//!
//! Not modelled: DPPI SUBSCRIBE/PUBLISH routing, the DMA.RX match/candidate
//! engine, bus-error injection, and clock stretching.

use crate::peripherals::i2c::I2cDevice;
use crate::{Bus, PeripheralTickResult, SimResult};

/// The GRTC SYSCOUNTER low word (GRTC base 0x500E_2000 + 0x720). It counts at
/// 1 MHz, so its value is microseconds directly — and it is the SAME clock the
/// firmware reads for time (`board_time_us`). The TWIM advances its slaves
/// against this, not the raw CPU cycle clock, so a sensor's sample clock and the
/// firmware's sense of time cannot drift apart: whatever the GRTC reads, the PPG
/// sees the identical elapsed microseconds. (Tying the slave clock to the CPU
/// cycle counter instead makes them diverge by the build's cycles-per-
/// instruction, which is exactly the leaky abstraction this avoids.)
///
/// Low word only: 32 bits of microseconds is ~71 minutes — far longer than any
/// inter-service gap — and reading one word avoids perturbing the
/// SYSCOUNTERL-latches-SYSCOUNTERH read pairing the firmware depends on.
const GRTC_SYSCOUNTERL: u64 = 0x500E_2720;

// ── Tasks ────────────────────────────────────────────────────────────────
const OFF_TASKS_STOP: u64 = 0x004;
const OFF_TASKS_SUSPEND: u64 = 0x00C;
const OFF_TASKS_RESUME: u64 = 0x010;
const OFF_TASKS_DMA_RX_START: u64 = 0x028;
const OFF_TASKS_DMA_RX_STOP: u64 = 0x02C;
const OFF_TASKS_DMA_TX_START: u64 = 0x050;
const OFF_TASKS_DMA_TX_STOP: u64 = 0x054;

// ── Events ───────────────────────────────────────────────────────────────
const OFF_EVENTS_STOPPED: u64 = 0x104;
const OFF_EVENTS_ERROR: u64 = 0x114;
const OFF_EVENTS_SUSPENDED: u64 = 0x128;
const OFF_EVENTS_LASTRX: u64 = 0x134;
const OFF_EVENTS_LASTTX: u64 = 0x138;
const OFF_EVENTS_DMA_RX_END: u64 = 0x14C;
const OFF_EVENTS_DMA_RX_READY: u64 = 0x150;
const OFF_EVENTS_DMA_TX_END: u64 = 0x168;
const OFF_EVENTS_DMA_TX_READY: u64 = 0x16C;

// ── Config ───────────────────────────────────────────────────────────────
const OFF_SHORTS: u64 = 0x200;
const OFF_INTEN: u64 = 0x300;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;
const OFF_ERRORSRC: u64 = 0x4C4;
const OFF_ENABLE: u64 = 0x500;
const OFF_FREQUENCY: u64 = 0x524;
const OFF_ADDRESS: u64 = 0x588;
const OFF_DMA_RX_PTR: u64 = 0x704;
const OFF_DMA_RX_MAXCNT: u64 = 0x708;
const OFF_DMA_RX_AMOUNT: u64 = 0x70C;
const OFF_DMA_TX_PTR: u64 = 0x73C;
const OFF_DMA_TX_MAXCNT: u64 = 0x740;
const OFF_DMA_TX_AMOUNT: u64 = 0x744;

/// `TWIM_ENABLE_ENABLE_Enabled` (MDK) — 6, same as nRF52. The model does not
/// gate transfers on ENABLE (a disabled TWIM on silicon simply does not drive
/// the bus, and no firmware path here depends on that distinction), so this is
/// referenced only by the tests that drive the documented enable sequence.
#[cfg(test)]
const ENABLE_TWIM: u32 = 6;

// SHORTS bits — positions unchanged from nRF52.
const SHORT_LASTTX_DMA_RX_START: u32 = 1 << 7;
const SHORT_LASTTX_SUSPEND: u32 = 1 << 8;
const SHORT_LASTTX_STOP: u32 = 1 << 9;
const SHORT_LASTRX_DMA_TX_START: u32 = 1 << 10;
const SHORT_LASTRX_STOP: u32 = 1 << 12;

// INTEN bits (SVD field order).
const INTEN_STOPPED: u32 = 1 << 0;
const INTEN_ERROR: u32 = 1 << 1;

const MAXCNT_MASK: u32 = 0xFFFF;

#[derive(Debug, Default)]
enum Pending {
    #[default]
    None,
    Tx,
    Rx,
}

#[derive(Default)]
pub struct Nrf54lTwim {
    // Events
    events_stopped: u32,
    events_error: u32,
    events_suspended: u32,
    events_lastrx: u32,
    events_lasttx: u32,
    events_dma_rx_end: u32,
    events_dma_rx_ready: u32,
    events_dma_tx_end: u32,
    events_dma_tx_ready: u32,

    // Config
    shorts: u32,
    inten: u32,
    errorsrc: u32,
    enable: u32,
    frequency: u32,
    address: u32,
    dma_rx_ptr: u32,
    dma_rx_maxcnt: u32,
    dma_rx_amount: u32,
    dma_tx_ptr: u32,
    dma_tx_maxcnt: u32,
    dma_tx_amount: u32,

    /// Transfer armed by a start task, performed on the bus-aware tick.
    pending: Pending,

    /// Attached I²C slaves, addressed by their 7-bit address.
    slaves: Vec<Box<dyn I2cDevice>>,

    /// Per-slave GRTC timestamp (µs) at which each slave was last advanced to
    /// "now". Parallel to `slaves`. `u64::MAX` means "not yet serviced", so the
    /// first transaction seeds the mark instead of charging the slave for all
    /// the time since boot. A slave is advanced by the microseconds elapsed on
    /// the GRTC since this mark, immediately before it is serviced — so a late
    /// poll (e.g. after a BLE connection-event ISR) sees the samples that
    /// accrued while the CPU was away.
    last_us: Vec<u64>,
}

impl std::fmt::Debug for Nrf54lTwim {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Nrf54lTwim")
            .field("enable", &self.enable)
            .field("address", &self.address)
            .field("slaves", &self.slaves.len())
            .finish()
    }
}

impl Nrf54lTwim {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_slave(&mut self, slave: Box<dyn I2cDevice>) {
        self.slaves.push(slave);
        // u64::MAX = "not yet serviced": the first transaction seeds the mark to
        // the current GRTC time rather than advancing the slave by all of it.
        self.last_us.push(u64::MAX);
    }

    fn slave_index(&self, addr: u8) -> Option<usize> {
        self.slaves.iter().position(|s| s.address() == addr)
    }

    /// The firmware's own clock: the GRTC SYSCOUNTER (µs), read via the bus.
    /// Returns 0 if no GRTC is mapped (unit tests without one), which leaves
    /// slaves un-advanced.
    fn grtc_now_us(bus: &dyn Bus) -> u64 {
        bus.read_u32(GRTC_SYSCOUNTERL).unwrap_or(0) as u64
    }

    /// Advance slave `idx` to the current GRTC time before servicing it. The
    /// first service seeds the mark (no advance); each later service hands over
    /// the microseconds elapsed on the GRTC since the previous one.
    fn advance_slave_to_now(&mut self, idx: usize, bus: &dyn Bus) {
        let now_us = Self::grtc_now_us(bus);
        let last = self.last_us[idx];
        if last == u64::MAX {
            self.last_us[idx] = now_us;
            return;
        }
        if now_us > last {
            self.slaves[idx].advance_time_us(now_us - last);
            self.last_us[idx] = now_us;
        }
    }

    fn event_bitmap(&self) -> u32 {
        let mut b = 0;
        if self.events_stopped != 0 {
            b |= INTEN_STOPPED;
        }
        if self.events_error != 0 {
            b |= INTEN_ERROR;
        }
        b
    }

    /// Apply the LASTTX_*/LASTRX_* shorts after a leg completes. Returns the
    /// follow-on transfer to run, if any — this is what makes the canonical
    /// write-register-then-repeated-START-read sequence work without the CPU
    /// having to intervene between the two legs.
    fn apply_shorts_after_tx(&mut self) -> Pending {
        if self.shorts & SHORT_LASTTX_DMA_RX_START != 0 && self.dma_rx_maxcnt & MAXCNT_MASK > 0 {
            return Pending::Rx;
        }
        if self.shorts & SHORT_LASTTX_STOP != 0 {
            self.events_stopped = 1;
        }
        if self.shorts & SHORT_LASTTX_SUSPEND != 0 {
            self.events_suspended = 1;
        }
        Pending::None
    }

    fn apply_shorts_after_rx(&mut self) -> Pending {
        if self.shorts & SHORT_LASTRX_DMA_TX_START != 0 && self.dma_tx_maxcnt & MAXCNT_MASK > 0 {
            return Pending::Tx;
        }
        if self.shorts & SHORT_LASTRX_STOP != 0 {
            self.events_stopped = 1;
        }
        Pending::None
    }

    fn do_tx(&mut self, bus: &mut dyn Bus) {
        let len = (self.dma_tx_maxcnt & MAXCNT_MASK) as usize;
        let addr = (self.address & 0x7F) as u8;
        let Some(idx) = self.slave_index(addr) else {
            // No device at this address: NACK on the address byte, exactly as
            // silicon reports an unpopulated bus.
            self.errorsrc |= 1 << 1; // ANACK
            self.events_error = 1;
            self.events_stopped = 1;
            return;
        };

        self.advance_slave_to_now(idx, bus);
        self.slaves[idx].start();
        for i in 0..len {
            if let Ok(b) = bus.read_u8(self.dma_tx_ptr as u64 + i as u64) {
                self.slaves[idx].write(b);
            }
        }
        self.dma_tx_amount = len as u32;
        self.events_dma_tx_end = 1;
        self.events_dma_tx_ready = 1;
        self.events_lasttx = 1;

        // A repeated START (LASTTX_DMA_RX_START) keeps the bus; anything else
        // ends the transaction, which is what lets the slave reset its
        // register-pointer state machine.
        if self.shorts & SHORT_LASTTX_DMA_RX_START == 0 {
            self.slaves[idx].stop();
        }
    }

    fn do_rx(&mut self, bus: &mut dyn Bus) {
        let len = (self.dma_rx_maxcnt & MAXCNT_MASK) as usize;
        let addr = (self.address & 0x7F) as u8;
        let Some(idx) = self.slave_index(addr) else {
            self.errorsrc |= 1 << 1; // ANACK
            self.events_error = 1;
            self.events_stopped = 1;
            return;
        };

        self.advance_slave_to_now(idx, bus);
        // Model the (repeated) START that re-addresses the slave for the read
        // phase. On silicon every RX leg begins with an address+R byte; the
        // write leg (do_tx) already signals its START via `start()`, but the RX
        // leg did not, so the bus-trace wrapper never saw a read-direction
        // address frame and the logic-analyzer/I²C panel decoded reads as bare
        // Data with no AddrRead — the same boundary the ESP32-C3 controller
        // emits at OP_RSTART. This is observability-only and byte-identical: the
        // four smart-ring slaves (BMI270/MAX30102/DRV2605) use the default
        // no-op `start()`, and TMP117's `start()` only re-zeroes its
        // read_phase/writes-since-start framing counters (already 0 entering the
        // read, and never the register pointer), so no returned byte changes.
        // The trace wrapper turns this into the AddrRead frame it was missing.
        self.slaves[idx].start();
        for i in 0..len {
            let b = self.slaves[idx].read();
            let _ = bus.write_u8(self.dma_rx_ptr as u64 + i as u64, b);
        }
        self.dma_rx_amount = len as u32;
        self.events_dma_rx_end = 1;
        self.events_dma_rx_ready = 1;
        self.events_lastrx = 1;
        self.slaves[idx].stop();
    }
}

impl crate::Peripheral for Nrf54lTwim {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            OFF_TASKS_STOP
            | OFF_TASKS_SUSPEND
            | OFF_TASKS_RESUME
            | OFF_TASKS_DMA_RX_START
            | OFF_TASKS_DMA_RX_STOP
            | OFF_TASKS_DMA_TX_START
            | OFF_TASKS_DMA_TX_STOP => 0,

            OFF_EVENTS_STOPPED => self.events_stopped,
            OFF_EVENTS_ERROR => self.events_error,
            OFF_EVENTS_SUSPENDED => self.events_suspended,
            OFF_EVENTS_LASTRX => self.events_lastrx,
            OFF_EVENTS_LASTTX => self.events_lasttx,
            OFF_EVENTS_DMA_RX_END => self.events_dma_rx_end,
            OFF_EVENTS_DMA_RX_READY => self.events_dma_rx_ready,
            OFF_EVENTS_DMA_TX_END => self.events_dma_tx_end,
            OFF_EVENTS_DMA_TX_READY => self.events_dma_tx_ready,

            OFF_SHORTS => self.shorts,
            OFF_INTEN | OFF_INTENSET | OFF_INTENCLR => self.inten,
            OFF_ERRORSRC => self.errorsrc,
            OFF_ENABLE => self.enable,
            OFF_FREQUENCY => self.frequency,
            OFF_ADDRESS => self.address,
            OFF_DMA_RX_PTR => self.dma_rx_ptr,
            OFF_DMA_RX_MAXCNT => self.dma_rx_maxcnt & MAXCNT_MASK,
            OFF_DMA_RX_AMOUNT => self.dma_rx_amount & MAXCNT_MASK,
            OFF_DMA_TX_PTR => self.dma_tx_ptr,
            OFF_DMA_TX_MAXCNT => self.dma_tx_maxcnt & MAXCNT_MASK,
            OFF_DMA_TX_AMOUNT => self.dma_tx_amount & MAXCNT_MASK,
            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_TASKS_DMA_TX_START if value & 1 != 0 => self.pending = Pending::Tx,
            OFF_TASKS_DMA_RX_START if value & 1 != 0 => self.pending = Pending::Rx,
            OFF_TASKS_STOP | OFF_TASKS_DMA_TX_STOP | OFF_TASKS_DMA_RX_STOP if value & 1 != 0 => {
                self.events_stopped = 1;
            }
            OFF_TASKS_SUSPEND if value & 1 != 0 => self.events_suspended = 1,
            OFF_TASKS_RESUME if value & 1 != 0 => self.events_suspended = 0,
            OFF_TASKS_STOP
            | OFF_TASKS_SUSPEND
            | OFF_TASKS_RESUME
            | OFF_TASKS_DMA_RX_START
            | OFF_TASKS_DMA_RX_STOP
            | OFF_TASKS_DMA_TX_START
            | OFF_TASKS_DMA_TX_STOP => {}

            // EVENTS: SW write-1 ignored, write-0 clears.
            OFF_EVENTS_STOPPED if value == 0 => self.events_stopped = 0,
            OFF_EVENTS_ERROR if value == 0 => self.events_error = 0,
            OFF_EVENTS_SUSPENDED if value == 0 => self.events_suspended = 0,
            OFF_EVENTS_LASTRX if value == 0 => self.events_lastrx = 0,
            OFF_EVENTS_LASTTX if value == 0 => self.events_lasttx = 0,
            OFF_EVENTS_DMA_RX_END if value == 0 => self.events_dma_rx_end = 0,
            OFF_EVENTS_DMA_RX_READY if value == 0 => self.events_dma_rx_ready = 0,
            OFF_EVENTS_DMA_TX_END if value == 0 => self.events_dma_tx_end = 0,
            OFF_EVENTS_DMA_TX_READY if value == 0 => self.events_dma_tx_ready = 0,
            OFF_EVENTS_STOPPED
            | OFF_EVENTS_ERROR
            | OFF_EVENTS_SUSPENDED
            | OFF_EVENTS_LASTRX
            | OFF_EVENTS_LASTTX
            | OFF_EVENTS_DMA_RX_END
            | OFF_EVENTS_DMA_RX_READY
            | OFF_EVENTS_DMA_TX_END
            | OFF_EVENTS_DMA_TX_READY => {}

            OFF_SHORTS => self.shorts = value,
            OFF_INTEN => self.inten = value,
            OFF_INTENSET => self.inten |= value,
            OFF_INTENCLR => self.inten &= !value,
            // ERRORSRC is write-1-clear.
            OFF_ERRORSRC => self.errorsrc &= !value,
            OFF_ENABLE => self.enable = value & 0xF,
            OFF_FREQUENCY => self.frequency = value,
            OFF_ADDRESS => self.address = value,
            OFF_DMA_RX_PTR => self.dma_rx_ptr = value,
            OFF_DMA_RX_MAXCNT => self.dma_rx_maxcnt = value & MAXCNT_MASK,
            OFF_DMA_TX_PTR => self.dma_tx_ptr = value,
            OFF_DMA_TX_MAXCNT => self.dma_tx_maxcnt = value & MAXCNT_MASK,
            _ => {}
        }
        Ok(())
    }

    /// EasyDMA needs a bus handle, which `write_u32` does not have, so an armed
    /// transfer is performed here. The bus re-arms this entry via
    /// `refresh_bus_tick_index()` after every MMIO write.
    fn needs_bus_tick(&self) -> bool {
        !matches!(self.pending, Pending::None)
    }

    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        // A short can chain one leg into the other; bound the loop so a
        // pathological SHORTS setting cannot spin forever.
        for _ in 0..2 {
            match std::mem::take(&mut self.pending) {
                Pending::None => return,
                Pending::Tx => {
                    self.do_tx(bus);
                    self.pending = self.apply_shorts_after_tx();
                }
                Pending::Rx => {
                    self.do_rx(bus);
                    self.pending = self.apply_shorts_after_rx();
                }
            }
        }
        self.pending = Pending::None;
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // The ONLY thing tick() does is re-assert the level-held IRQ while an
        // enabled event (STOPPED/ERROR) is latched — the transfer itself runs
        // in `tick_with_bus`. Charge ZERO cost: a TWIM consumes no core cycles
        // (real EasyDMA runs on the bus), and a non-zero cost would inflate
        // `total_cycles` and so perturb the clock-derived nRF54L GRTC
        // SYSCOUNTER that firmware reads for time.
        PeripheralTickResult {
            irq: self.inten & self.event_bitmap() != 0,
            ..Default::default()
        }
    }

    /// The legacy per-cycle walk only needs this peripheral while `tick()` has
    /// output — i.e. while an enabled event holds the IRQ level. With nothing
    /// latched-and-enabled `tick()` is a genuine no-op, so the model drops out
    /// of the walk (and stops blocking idle fast-forward). This is provably
    /// walk-identical: every cycle skipped is one where `tick()` returned
    /// `irq: false` with zero cost. An armed EasyDMA transfer runs on the
    /// separate `needs_bus_tick` path and its wall-clock slave advance happens
    /// at transaction time via a bus GRTC read, not a per-cycle tick — so a
    /// pending transfer does not need the legacy walk either.
    fn legacy_tick_active(&self) -> bool {
        self.inten & self.event_bitmap() != 0
    }

    /// `legacy_tick_active` depends on mutable event/INTEN state, so the bus
    /// must re-check it after each tick rather than caching it once.
    fn legacy_tick_dynamic(&self) -> bool {
        true
    }

    /// `tick()` does real work (the level IRQ) whenever an enabled event is
    /// latched, so the walk is behaviorally significant and must not be
    /// statically deleted; `legacy_tick_active` handles the per-instant skip.
    fn needs_legacy_walk(&self) -> bool {
        true
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SystemBus;
    use crate::memory::LinearMemory;
    use crate::Peripheral;

    /// Minimal register-pointer slave, the same shape as a real sensor: a write
    /// sets the pointer, reads return and auto-increment.
    struct FakeSensor {
        regs: [u8; 256],
        ptr: u8,
        addr_written: bool,
    }

    impl Default for FakeSensor {
        fn default() -> Self {
            Self {
                regs: [0; 256],
                ptr: 0,
                addr_written: false,
            }
        }
    }

    impl I2cDevice for FakeSensor {
        fn address(&self) -> u8 {
            0x68
        }
        fn read(&mut self) -> u8 {
            let v = self.regs[self.ptr as usize];
            self.ptr = self.ptr.wrapping_add(1);
            v
        }
        fn write(&mut self, data: u8) {
            if !self.addr_written {
                self.ptr = data;
                self.addr_written = true;
            } else {
                self.regs[self.ptr as usize] = data;
                self.ptr = self.ptr.wrapping_add(1);
            }
        }
        fn stop(&mut self) {
            self.addr_written = false;
        }
    }

    fn rig() -> (Nrf54lTwim, SystemBus) {
        let mut twim = Nrf54lTwim::new();
        let mut sensor = FakeSensor::default();
        sensor.regs[0x75] = 0x68; // WHO_AM_I
        twim.push_slave(Box::new(sensor));

        // RAM-backed bus for the EasyDMA buffers.
        let mut bus = SystemBus::empty();
        bus.ram = LinearMemory::new(256, 0x2000_0000);
        (twim, bus)
    }

    /// A slave that records the total wall-clock it was advanced by. Used to
    /// prove the TWIM hands a slave the time that elapsed while the CPU was busy
    /// elsewhere — the mechanism that makes a starved PPG FIFO overflow.
    struct TimedSensor {
        advanced_us: std::sync::Arc<std::sync::atomic::AtomicU64>,
        addr_written: bool,
    }

    impl I2cDevice for TimedSensor {
        fn address(&self) -> u8 {
            0x57
        }
        fn read(&mut self) -> u8 {
            0xAB
        }
        fn write(&mut self, _data: u8) {
            self.addr_written = true;
        }
        fn stop(&mut self) {
            self.addr_written = false;
        }
        fn advance_time_us(&mut self, us: u64) {
            self.advanced_us
                .fetch_add(us, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Drive one repeated-START register read against the slave at `addr`.
    fn drive_read(twim: &mut Nrf54lTwim, bus: &mut SystemBus, addr: u32) {
        bus.write_u8(0x2000_0000, 0x00).unwrap();
        twim.write_u32(OFF_ENABLE, ENABLE_TWIM).unwrap();
        twim.write_u32(OFF_ADDRESS, addr).unwrap();
        twim.write_u32(OFF_SHORTS, SHORT_LASTTX_DMA_RX_START | SHORT_LASTRX_STOP)
            .unwrap();
        twim.write_u32(OFF_DMA_TX_PTR, 0x2000_0000).unwrap();
        twim.write_u32(OFF_DMA_TX_MAXCNT, 1).unwrap();
        twim.write_u32(OFF_DMA_RX_PTR, 0x2000_0010).unwrap();
        twim.write_u32(OFF_DMA_RX_MAXCNT, 1).unwrap();
        twim.write_u32(OFF_TASKS_DMA_TX_START, 1).unwrap();
        twim.tick_with_bus(bus);
    }

    /// Map a stand-in GRTC SYSCOUNTER the TWIM can read as the firmware's clock,
    /// and set it to `us` microseconds.
    fn set_grtc_us(bus: &mut SystemBus, us: u32) {
        if bus.extra_mem.is_empty() {
            bus.extra_mem.push(LinearMemory::new(0x800, 0x500E_2000));
        }
        bus.write_u32(GRTC_SYSCOUNTERL, us).unwrap();
    }

    #[test]
    fn slave_advances_by_grtc_time_elapsed_while_cpu_was_busy() {
        use std::sync::atomic::Ordering;
        use std::sync::{atomic::AtomicU64, Arc};

        let advanced = Arc::new(AtomicU64::new(0));
        let mut twim = Nrf54lTwim::new();
        twim.push_slave(Box::new(TimedSensor {
            advanced_us: advanced.clone(),
            addr_written: false,
        }));
        let mut bus = SystemBus::empty();
        bus.ram = LinearMemory::new(256, 0x2000_0000);

        // t = 0: the first service seeds the mark and must NOT advance the slave
        // (else it would be charged for all the time since boot).
        set_grtc_us(&mut bus, 0);
        drive_read(&mut twim, &mut bus, 0x57);
        assert_eq!(
            advanced.load(Ordering::Relaxed),
            0,
            "the first service seeds the mark, it does not advance"
        );

        // 1 ms of GRTC time passes with NO transaction — the CPU is off in a BLE
        // connection-event ISR. The next service must hand the slave that 1 ms,
        // which is the catch-up that overflows a starved FIFO.
        set_grtc_us(&mut bus, 1000);
        drive_read(&mut twim, &mut bus, 0x57);
        assert_eq!(
            advanced.load(Ordering::Relaxed),
            1000,
            "the slave must be advanced by the 1 ms that elapsed on the GRTC \
             while the CPU was busy"
        );
    }

    #[test]
    fn no_grtc_mapped_means_no_wall_clock_advance() {
        use std::sync::atomic::Ordering;
        use std::sync::{atomic::AtomicU64, Arc};

        // No GRTC mapped: grtc_now_us reads 0 forever, so after the seed the
        // slave is never advanced — byte-identical to the pre-seam behaviour.
        let advanced = Arc::new(AtomicU64::new(0));
        let mut twim = Nrf54lTwim::new();
        twim.push_slave(Box::new(TimedSensor {
            advanced_us: advanced.clone(),
            addr_written: false,
        }));
        let mut bus = SystemBus::empty();
        bus.ram = LinearMemory::new(256, 0x2000_0000);

        drive_read(&mut twim, &mut bus, 0x57);
        drive_read(&mut twim, &mut bus, 0x57);

        assert_eq!(
            advanced.load(Ordering::Relaxed),
            0,
            "no GRTC → slaves keep the old transaction-only behaviour"
        );
    }

    #[test]
    fn register_read_uses_repeated_start_and_returns_the_right_byte() {
        let (mut twim, mut bus) = rig();
        bus.write_u8(0x2000_0000, 0x75).unwrap(); // register pointer in RAM

        twim.write_u32(OFF_ENABLE, ENABLE_TWIM).unwrap();
        twim.write_u32(OFF_ADDRESS, 0x68).unwrap();
        // LASTTX -> repeated START into RX, then STOP: the canonical sequence.
        twim.write_u32(OFF_SHORTS, SHORT_LASTTX_DMA_RX_START | SHORT_LASTRX_STOP)
            .unwrap();
        twim.write_u32(OFF_DMA_TX_PTR, 0x2000_0000).unwrap();
        twim.write_u32(OFF_DMA_TX_MAXCNT, 1).unwrap();
        twim.write_u32(OFF_DMA_RX_PTR, 0x2000_0010).unwrap();
        twim.write_u32(OFF_DMA_RX_MAXCNT, 1).unwrap();
        twim.write_u32(OFF_TASKS_DMA_TX_START, 1).unwrap();

        assert!(twim.needs_bus_tick());
        twim.tick_with_bus(&mut bus);

        assert_eq!(
            bus.read_u8(0x2000_0010).unwrap(),
            0x68,
            "WHO_AM_I must come back through the repeated-START read"
        );
        assert_ne!(twim.read_u32(OFF_EVENTS_DMA_RX_END).unwrap(), 0);
        assert_ne!(
            twim.read_u32(OFF_EVENTS_STOPPED).unwrap(),
            0,
            "LASTRX_STOP must end the transaction"
        );
    }

    /// Logic-analyzer regression guard: a repeated-START register read against
    /// a trace-wrapped slave must produce BOTH a write-direction and a
    /// read-direction address frame in the bus trace — not just raw Data. The
    /// TWIM previously signalled `start()` only on the write leg, so the read
    /// leg decoded as bare Data with no `AddrRead` and the I²C panel could not
    /// tell the transaction's direction (empty/undecodable for all four ring
    /// sensors). This also pins that the returned byte is unchanged, i.e. the
    /// added `start()` is observability-only.
    #[test]
    fn bus_trace_emits_addr_read_frame_for_repeated_start_read() {
        use crate::bus::bus_trace::{new_log, wrap_i2c, BusPayload, I2cSym};

        let mut twim = Nrf54lTwim::new();
        let mut sensor = FakeSensor::default();
        sensor.regs[0x75] = 0x68; // WHO_AM_I
        let log = new_log();
        // The bus choke point wraps before push; emulate it here.
        twim.push_slave(wrap_i2c("twi21", &log, Box::new(sensor)));

        let mut bus = SystemBus::empty();
        bus.ram = LinearMemory::new(256, 0x2000_0000);
        bus.write_u8(0x2000_0000, 0x75).unwrap(); // register pointer in RAM

        twim.write_u32(OFF_ENABLE, ENABLE_TWIM).unwrap();
        twim.write_u32(OFF_ADDRESS, 0x68).unwrap();
        twim.write_u32(OFF_SHORTS, SHORT_LASTTX_DMA_RX_START | SHORT_LASTRX_STOP)
            .unwrap();
        twim.write_u32(OFF_DMA_TX_PTR, 0x2000_0000).unwrap();
        twim.write_u32(OFF_DMA_TX_MAXCNT, 1).unwrap();
        twim.write_u32(OFF_DMA_RX_PTR, 0x2000_0010).unwrap();
        twim.write_u32(OFF_DMA_RX_MAXCNT, 1).unwrap();
        twim.write_u32(OFF_TASKS_DMA_TX_START, 1).unwrap();
        twim.tick_with_bus(&mut bus);

        // Byte behaviour is unchanged: WHO_AM_I still returns through the
        // trace-wrapped slave exactly as in the un-wrapped read test.
        assert_eq!(bus.read_u8(0x2000_0010).unwrap(), 0x68);

        let events = log.snapshot();
        assert!(events.iter().all(|e| e.bus == "twi21"));
        // Write leg → write-direction address frame (addr << 1).
        assert!(
            events.iter().any(|e| matches!(
                &e.payload,
                BusPayload::I2c { kind: I2cSym::AddrWrite, byte, .. } if *byte == 0x68 << 1
            )),
            "missing AddrWrite address frame: {events:?}"
        );
        // Repeated-START read leg → read-direction address frame ((addr<<1)|1).
        assert!(
            events.iter().any(|e| matches!(
                &e.payload,
                BusPayload::I2c { kind: I2cSym::AddrRead, byte, .. } if *byte == (0x68 << 1) | 1
            )),
            "missing AddrRead address frame for the repeated-START read: {events:?}"
        );
    }

    /// A STOP between the two legs lets the slave reset its pointer, which is
    /// what makes the naive write/STOP/read sequence return the wrong byte on
    /// real silicon. Pin the modelled behaviour so the distinction survives.
    #[test]
    fn stop_between_legs_resets_the_slave_pointer() {
        let (mut twim, mut bus) = rig();
        bus.write_u8(0x2000_0000, 0x75).unwrap();

        twim.write_u32(OFF_ENABLE, ENABLE_TWIM).unwrap();
        twim.write_u32(OFF_ADDRESS, 0x68).unwrap();
        twim.write_u32(OFF_SHORTS, SHORT_LASTTX_STOP).unwrap(); // STOP, no repeated start
        twim.write_u32(OFF_DMA_TX_PTR, 0x2000_0000).unwrap();
        twim.write_u32(OFF_DMA_TX_MAXCNT, 1).unwrap();
        twim.write_u32(OFF_TASKS_DMA_TX_START, 1).unwrap();
        twim.tick_with_bus(&mut bus);

        // Separate RX transaction: the slave saw a STOP, so addr_written was
        // cleared — but current_register survives, mirroring a real part.
        twim.write_u32(OFF_DMA_RX_PTR, 0x2000_0010).unwrap();
        twim.write_u32(OFF_DMA_RX_MAXCNT, 1).unwrap();
        twim.write_u32(OFF_TASKS_DMA_RX_START, 1).unwrap();
        twim.tick_with_bus(&mut bus);
        assert_eq!(bus.read_u8(0x2000_0010).unwrap(), 0x68);
    }

    #[test]
    fn unaddressed_device_nacks_and_raises_error() {
        let (mut twim, mut bus) = rig();
        twim.write_u32(OFF_ENABLE, ENABLE_TWIM).unwrap();
        twim.write_u32(OFF_ADDRESS, 0x42).unwrap(); // nothing there
        twim.write_u32(OFF_DMA_TX_PTR, 0x2000_0000).unwrap();
        twim.write_u32(OFF_DMA_TX_MAXCNT, 1).unwrap();
        twim.write_u32(OFF_TASKS_DMA_TX_START, 1).unwrap();
        twim.tick_with_bus(&mut bus);

        assert_ne!(twim.read_u32(OFF_EVENTS_ERROR).unwrap(), 0);
        assert_ne!(
            twim.read_u32(OFF_ERRORSRC).unwrap() & (1 << 1),
            0,
            "ANACK must be set for an unpopulated address"
        );
    }

    #[test]
    fn errorsrc_is_write_one_to_clear() {
        let (mut twim, mut bus) = rig();
        twim.write_u32(OFF_ADDRESS, 0x42).unwrap();
        twim.write_u32(OFF_DMA_TX_MAXCNT, 1).unwrap();
        twim.write_u32(OFF_TASKS_DMA_TX_START, 1).unwrap();
        twim.tick_with_bus(&mut bus);
        assert_ne!(twim.read_u32(OFF_ERRORSRC).unwrap(), 0);

        twim.write_u32(OFF_ERRORSRC, 1 << 1).unwrap();
        assert_eq!(twim.read_u32(OFF_ERRORSRC).unwrap(), 0);
    }

    #[test]
    fn events_are_write_zero_to_clear() {
        let (mut twim, mut bus) = rig();
        twim.write_u32(OFF_ADDRESS, 0x68).unwrap();
        twim.write_u32(OFF_DMA_TX_MAXCNT, 0).unwrap();
        twim.write_u32(OFF_TASKS_DMA_TX_START, 1).unwrap();
        twim.tick_with_bus(&mut bus);
        assert_ne!(twim.read_u32(OFF_EVENTS_DMA_TX_END).unwrap(), 0);

        twim.write_u32(OFF_EVENTS_DMA_TX_END, 1).unwrap(); // ignored
        assert_ne!(twim.read_u32(OFF_EVENTS_DMA_TX_END).unwrap(), 0);
        twim.write_u32(OFF_EVENTS_DMA_TX_END, 0).unwrap(); // clears
        assert_eq!(twim.read_u32(OFF_EVENTS_DMA_TX_END).unwrap(), 0);
    }

    #[test]
    fn register_round_trips_and_unmapped_reads_zero() {
        let (mut twim, _bus) = rig();
        twim.write_u32(OFF_FREQUENCY, 0x0640_0000).unwrap();
        assert_eq!(twim.read_u32(OFF_FREQUENCY).unwrap(), 0x0640_0000);
        twim.write_u32(OFF_DMA_TX_PTR, 0x2000_1234).unwrap();
        assert_eq!(twim.read_u32(OFF_DMA_TX_PTR).unwrap(), 0x2000_1234);
        // MAXCNT is 16-bit.
        twim.write_u32(OFF_DMA_TX_MAXCNT, 0xFFFF_FFFF).unwrap();
        assert_eq!(twim.read_u32(OFF_DMA_TX_MAXCNT).unwrap(), MAXCNT_MASK);

        assert_eq!(twim.read_u32(0xFFC).unwrap(), 0);
    }
}
