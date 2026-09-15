// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `virtual_ble` — a **LabWired-defined** BLE link-layer controller, for parts
//! whose real radio the vendor does not document.
//!
//! # Read this before using it
//!
//! This device **does not exist on any silicon**. It is not a model of the
//! EFR32 radio, or of any other radio. It is a simulator device in the same
//! sense as [`crate::peripherals::simctl`]: a door firmware knocks on, with a
//! register map LabWired invented and publishes. `ID` reads the ASCII magic
//! `LWBL` precisely so that a memory dump, a register inspector or a confused
//! human can tell in one word that this block is ours.
//!
//! Every other radio in this engine is modelled from a register map its vendor
//! published: the nRF52 `RADIO` from Nordic's SVD, the ESP32-C3 RW-BLE core
//! from Espressif's. Silicon Labs publishes neither. Checked, on 2026-08-21,
//! for EFR32MG26:
//!
//! * The EFR32xG26 Reference Manual rev 1.0 has a chapter 5 "Radio
//!   Transceiver" — four pages of prose (RF synthesizer, modulation modes,
//!   framing, CRC) and **no register documentation at all**, while every other
//!   peripheral gets a full register map and description.
//! * The CMSIS device headers for EFR32MG26 (`simplicity_sdk` tag
//!   `sisdk-2025.6`) ship 73 peripheral headers. None of them is `rac`, `frc`,
//!   `modem`, `protimer`, `bufc`, `synth` or `agc` — the blocks that make up
//!   the radio. They are absent, not merely undocumented.
//! * There is no SVD for the family.
//!
//! So the radio is reachable only through Silicon Labs' closed RAIL binary.
//! Modelling it faithfully would mean reverse-engineering an entire
//! undocumented peripheral from bus traffic, and inventing EFR32-shaped
//! register names for it would be **worse than this device**: it would look
//! like silicon in an inspector and it would be fiction.
//!
//! # What this buys, and what it costs
//!
//! Buys: a board whose radio is opaque can still take part in a BLE lab. It
//! advertises and scans on the same [`crate::peripherals::ble_air`] every other
//! LabWired BLE controller uses, so a BRD2709A can advertise to an ESP32-C3
//! scanner in one lab, and the PDUs on that air are real BLE PDUs.
//!
//! Costs, stated plainly: **a sketch built against this device does not run on
//! the physical board.** On silicon the same sketch has to go through the
//! vendor stack. That is a real seam, it is not hidden, and closing it means
//! shipping the vendor RAIL/link-layer libraries in the compile lane — tracked
//! separately, not papered over here.
//!
//! # Register map (32-bit accesses)
//!
//! | Offset | Name | Access | Meaning |
//! |--------|------|--------|---------|
//! | `0x00` | `ID` | R | `0x4C57424C` = `"LWBL"`. This block is a LabWired device. |
//! | `0x04` | `CTRL` | RW | bit0 `ADV_EN`, bit1 `SCAN_EN`. |
//! | `0x08` | `STATUS` | R | bit0 `RX_AVAIL`, bits[15:8] queued frame count. |
//! | `0x0C` | `CHANNEL` | RW | RF channel to listen on while scanning (0..=39). |
//! | `0x10` | `ACCESSADDR` | RW | Access address. Reset `0x8E89BED6` (advertising). |
//! | `0x14` | `CRCINIT` | RW | CRC init. Reset `0x555555` (advertising). |
//! | `0x18` | `ADVINTERVAL` | RW | Advertising interval, in units of 625 µs. |
//! | `0x1C` | `ADDRL` | RW | Own device address, bytes 0..3. |
//! | `0x20` | `ADDRH` | RW | Own device address, bytes 4..5. |
//! | `0x24` | `TXLEN` | RW | Length of the staged PDU in `TXBUF`, including the two header bytes. |
//! | `0x28` | `TXCMD` | W | Write 1: transmit `TXBUF` once, now, on `CHANNEL`. |
//! | `0x2C` | `RXCMD` | W | Write 1: pop the oldest received frame into `RXBUF`. |
//! | `0x30` | `RXLEN` | R | Length of the popped frame; 0 when nothing was popped. |
//! | `0x34` | `RXCHANNEL` | R | Channel the popped frame arrived on. |
//! | `0x38` | `IEN` | RW | bit0: raise the IRQ while a frame is queued. |
//! | `0x3C` | `IF` | RW1C | bit0: a frame arrived. |
//! | `0x100`..`0x13F` | `TXBUF` | RW | Staged PDU: header, length, payload. |
//! | `0x200`..`0x23F` | `RXBUF` | R | Popped PDU, same layout. |
//!
//! # Faithfully modelled
//!
//! * **The PDU is a real BLE PDU.** Firmware stages `[header, length,
//!   payload…]` and that is exactly what lands on the air and what a peer
//!   receives — no re-framing, nothing synthesised.
//! * **Advertising is periodic and hops.** With `ADV_EN` set the controller
//!   transmits `TXBUF` on channels 37, 38 and 39 once per `ADVINTERVAL`,
//!   measured in simulated CPU cycles, which is what BLE advertising is.
//! * **A scanner hears only its channel** and only frames whose access address
//!   matches `ACCESSADDR`, and never its own transmissions.
//! * **The RX queue is bounded** and drops the oldest, so a firmware that stops
//!   draining misses frames rather than stalling its peer.
//!
//! # Idealised — present, but not physical
//!
//! * No PHY: no GFSK, no preamble, no whitening, no CRC computation, no bit
//!   errors, no path loss, no RSSI, no collisions. Inherited from
//!   [`ble_air`], where the same list is spelled out.
//! * No connections, no channel-hopping map, no encryption, no pairing. This
//!   is an advertising/scanning device; a connection-oriented lab needs more
//!   than it has, and it will not pretend otherwise.
//! * Transmission is atomic and instantaneous — the air-time a real 31-byte
//!   advertisement occupies is not charged to anything.
//! * `ADVINTERVAL` is honoured as a period, but the three channel copies go out
//!   in the same instant rather than spaced by the real inter-channel delay.

use crate::peripherals::ble_air::{next_node_id, BleAirBus, BleAirFrame};
use crate::{PeripheralTickResult, SimResult};

/// `ID` — ASCII `"LWBL"`, little-endian. A LabWired device, not silicon.
pub const LWBL_MAGIC: u32 = 0x4C42_574C;

const OFF_ID: u64 = 0x00;
const OFF_CTRL: u64 = 0x04;
const OFF_STATUS: u64 = 0x08;
const OFF_CHANNEL: u64 = 0x0C;
const OFF_ACCESSADDR: u64 = 0x10;
const OFF_CRCINIT: u64 = 0x14;
const OFF_ADVINTERVAL: u64 = 0x18;
const OFF_ADDRL: u64 = 0x1C;
const OFF_ADDRH: u64 = 0x20;
const OFF_TXLEN: u64 = 0x24;
const OFF_TXCMD: u64 = 0x28;
const OFF_RXCMD: u64 = 0x2C;
const OFF_RXLEN: u64 = 0x30;
const OFF_RXCHANNEL: u64 = 0x34;
const OFF_IEN: u64 = 0x38;
const OFF_IF: u64 = 0x3C;
const OFF_TXBUF: u64 = 0x100;
const OFF_RXBUF: u64 = 0x200;

/// `CTRL.ADV_EN`.
const CTRL_ADV_EN: u32 = 1 << 0;
/// `CTRL.SCAN_EN`.
const CTRL_SCAN_EN: u32 = 1 << 1;
/// `STATUS.RX_AVAIL` / `IF.RX`.
const RX_AVAIL: u32 = 1 << 0;

/// The BLE advertising access address, and the reset value of `ACCESSADDR`.
const ADV_ACCESS_ADDRESS: u32 = 0x8E89_BED6;
/// The BLE advertising CRC init, and the reset value of `CRCINIT`.
const ADV_CRC_INIT: u32 = 0x0055_5555;
/// The three primary advertising channels, in the order a real controller
/// visits them.
const ADV_CHANNELS: [u8; 3] = [37, 38, 39];

/// Largest PDU this device carries: two header bytes plus a 62-byte payload.
/// Comfortably covers a legacy 31-byte advertisement and its scan response.
const BUF_LEN: usize = 64;
/// Received frames retained before the oldest is dropped.
const RX_DEPTH: usize = 16;

/// One BLE tick unit, 625 µs, expressed as a divisor of the CPU clock. The
/// controller needs the CPU frequency to turn `ADVINTERVAL` into cycles.
const BLE_TICK_US: u64 = 625;

/// A LabWired virtual BLE controller. See the module documentation — this is
/// not a model of any silicon.
#[derive(Debug)]
pub struct VirtualBle {
    air: BleAirBus,
    /// This controller's identity on the air, so it never decodes its own
    /// transmission.
    node_id: u64,
    /// Sequence number to read the air from. Joins at the air's CURRENT
    /// sequence, never at 0: a radio has no history buffer, and a lab reuses
    /// its air across a restart, so starting at 0 would replay the previous
    /// run's traffic as live peer packets.
    rx_cursor: u64,
    /// CPU cycles per second, so `ADVINTERVAL` means the same wall time it
    /// would on a real controller.
    cpu_hz: u64,

    ctrl: u32,
    channel: u8,
    access_address: u32,
    crc_init: u32,
    /// Advertising interval, in 625 µs units. Reset 160 = 100 ms, the interval
    /// most beacon examples use.
    adv_interval: u32,
    addr_lo: u32,
    addr_hi: u32,

    tx_len: u32,
    tx_buf: [u8; BUF_LEN],
    rx_buf: [u8; BUF_LEN],
    rx_len: u32,
    rx_channel: u8,

    ien: u32,
    iflag: u32,

    /// Frames pulled off the air and not yet popped by firmware.
    queued: std::collections::VecDeque<BleAirFrame>,
    /// Cycles since the last advertising burst.
    since_adv: u64,

    /// Bus-published cycle clock, attached by `SystemBus::add_peripheral`.
    /// `Some` selects scheduler mode; `None` keeps the legacy per-cycle walk.
    clock: Option<crate::CycleClock>,
    /// The absolute cycle `since_adv` was last brought forward to. Owned by
    /// [`VirtualBle::advance_to`]; the legacy walk never touches it.
    anchor: u64,
    /// In-flight singleton guard (cancellation contract layer 2): true while a
    /// service event is queued for this controller.
    chain_live: bool,
}

impl VirtualBle {
    /// Mint a controller on `air`. Two controllers minted on the same air hear
    /// each other; controllers on different airs are fully isolated.
    pub fn new(air: BleAirBus, cpu_hz: u64) -> Self {
        let rx_cursor = air.current_seq();
        Self {
            air,
            node_id: next_node_id(),
            rx_cursor,
            cpu_hz: cpu_hz.max(1),
            ctrl: 0,
            channel: ADV_CHANNELS[0],
            access_address: ADV_ACCESS_ADDRESS,
            crc_init: ADV_CRC_INIT,
            adv_interval: 160,
            addr_lo: 0,
            addr_hi: 0,
            tx_len: 0,
            tx_buf: [0; BUF_LEN],
            rx_buf: [0; BUF_LEN],
            rx_len: 0,
            rx_channel: 0,
            ien: 0,
            iflag: 0,
            queued: std::collections::VecDeque::new(),
            since_adv: 0,
            clock: None,
            anchor: 0,
            chain_live: false,
        }
    }

    /// Build on the process-global air, for the ordinary factory path which
    /// has no lab identity to thread through. `cpu_hz` is corrected by
    /// [`crate::Peripheral::attach_cpu_hz`] as soon as the bus registers this
    /// device; the placeholder only matters for a hand-built bus that bypasses
    /// the choke point.
    pub fn new_default() -> Self {
        Self::new(
            crate::peripherals::ble_air::default_ble_air_bus().clone(),
            1_000_000,
        )
    }

    /// Cycles between advertising bursts, from `ADVINTERVAL` and the CPU clock.
    /// Never zero: a zero interval would transmit on every tick and drown the
    /// air, and a real controller rejects it too (the BLE minimum is 20 ms).
    fn adv_period_cycles(&self) -> u64 {
        let units = self.adv_interval.max(1) as u64;
        (self.cpu_hz.saturating_mul(units * BLE_TICK_US) / 1_000_000).max(1)
    }

    /// The PDU currently staged in `TXBUF`, clamped to `TXLEN`.
    fn staged_pdu(&self) -> Vec<u8> {
        let len = (self.tx_len as usize).min(BUF_LEN);
        self.tx_buf[..len].to_vec()
    }

    fn transmit_on(&self, channel: u8) {
        let pdu = self.staged_pdu();
        if pdu.is_empty() {
            return;
        }
        self.air.transmit(BleAirFrame {
            seq: 0,
            source: self.node_id,
            channel,
            access_address: self.access_address,
            crc_init: self.crc_init,
            pdu,
        });
    }

    /// Drain whatever the air has for us on the tuned channel. Returns true if
    /// anything was queued.
    fn drain_air(&mut self) -> bool {
        if self.ctrl & CTRL_SCAN_EN == 0 {
            return false;
        }
        let mut got = false;
        // Drain every frame at or after the cursor, one at a time: the air is a
        // broadcast medium and `receive_from` leaves the frame in place for the
        // other listeners, so the cursor is what stops us re-reading it.
        while let Some(frame) = self.air.receive_from(
            self.channel,
            self.access_address,
            self.rx_cursor,
            self.node_id,
        ) {
            self.rx_cursor = frame.seq + 1;
            self.queued.push_back(frame);
            while self.queued.len() > RX_DEPTH {
                self.queued.pop_front();
            }
            got = true;
        }
        got
    }

    fn pop_rx(&mut self) {
        self.rx_buf = [0; BUF_LEN];
        match self.queued.pop_front() {
            Some(frame) => {
                let n = frame.pdu.len().min(BUF_LEN);
                self.rx_buf[..n].copy_from_slice(&frame.pdu[..n]);
                self.rx_len = n as u32;
                self.rx_channel = frame.channel;
            }
            None => {
                self.rx_len = 0;
                self.rx_channel = 0;
            }
        }
        if self.queued.is_empty() {
            self.iflag &= !RX_AVAIL;
        }
    }

    fn status(&self) -> u32 {
        let mut s = 0;
        if !self.queued.is_empty() {
            s |= RX_AVAIL;
        }
        s |= ((self.queued.len() as u32) & 0xFF) << 8;
        s
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            OFF_ID => LWBL_MAGIC,
            OFF_CTRL => self.ctrl,
            OFF_STATUS => self.status(),
            OFF_CHANNEL => self.channel as u32,
            OFF_ACCESSADDR => self.access_address,
            OFF_CRCINIT => self.crc_init,
            OFF_ADVINTERVAL => self.adv_interval,
            OFF_ADDRL => self.addr_lo,
            OFF_ADDRH => self.addr_hi,
            OFF_TXLEN => self.tx_len,
            OFF_RXLEN => self.rx_len,
            OFF_RXCHANNEL => self.rx_channel as u32,
            OFF_IEN => self.ien,
            OFF_IF => self.iflag,
            o if (OFF_TXBUF..OFF_TXBUF + BUF_LEN as u64).contains(&o) => {
                word_from(&self.tx_buf, (o - OFF_TXBUF) as usize)
            }
            o if (OFF_RXBUF..OFF_RXBUF + BUF_LEN as u64).contains(&o) => {
                word_from(&self.rx_buf, (o - OFF_RXBUF) as usize)
            }
            _ => 0,
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        match offset {
            OFF_CTRL => {
                let was_adv = self.ctrl & CTRL_ADV_EN != 0;
                self.ctrl = value & (CTRL_ADV_EN | CTRL_SCAN_EN);
                // Starting to advertise transmits immediately rather than after
                // one silent interval — which is what a controller does, and
                // what makes a short lab see the first packet.
                if !was_adv && self.ctrl & CTRL_ADV_EN != 0 {
                    self.advertise_burst();
                    self.since_adv = 0;
                }
            }
            OFF_CHANNEL => self.channel = (value & 0x3F) as u8,
            OFF_ACCESSADDR => self.access_address = value,
            OFF_CRCINIT => self.crc_init = value,
            OFF_ADVINTERVAL => self.adv_interval = value,
            OFF_ADDRL => self.addr_lo = value,
            OFF_ADDRH => self.addr_hi = value,
            OFF_TXLEN => self.tx_len = value.min(BUF_LEN as u32),
            OFF_TXCMD if value & 1 != 0 => {
                self.transmit_on(self.channel);
            }
            OFF_RXCMD if value & 1 != 0 => {
                self.pop_rx();
            }
            OFF_IEN => self.ien = value,
            // Write-1-to-clear, the Series-2 convention firmware here already
            // knows from every other IF register on the part.
            OFF_IF => self.iflag &= !value,
            o if (OFF_TXBUF..OFF_TXBUF + BUF_LEN as u64).contains(&o) => {
                word_into(&mut self.tx_buf, (o - OFF_TXBUF) as usize, value)
            }
            // RXBUF is read-only: it is what the air delivered.
            _ => {}
        }
    }

    fn advertise_burst(&mut self) {
        for ch in ADV_CHANNELS {
            self.transmit_on(ch);
        }
    }

    // ── Drive-mode plumbing (walk vs event scheduler) ──────────────────────

    crate::cycle_clock::scheduler_mode!();

    /// Test/differential knob: detach the clock, pinning the controller to the
    /// legacy walk so a differential can build its reference lane from the same
    /// bus assembly.
    pub fn force_legacy_walk(&mut self) {
        self.clock = None;
        self.chain_live = false;
    }

    /// One legacy walk tick: age the advertising interval, transmit if it came
    /// due, drain the air and report the held IRQ level. Shared by the walk,
    /// the hardware-oracle forced walk and the event chain, so the three cannot
    /// drift apart.
    fn service(&mut self, cycles: u64) -> PeripheralTickResult {
        let mut result = PeripheralTickResult::default();

        if self.ctrl & CTRL_ADV_EN != 0 {
            self.since_adv = self.since_adv.saturating_add(cycles);
            let period = self.adv_period_cycles();
            if self.since_adv >= period {
                self.since_adv -= period;
                self.advertise_burst();
            }
        }

        if self.drain_air() {
            self.iflag |= RX_AVAIL;
        }
        if self.ien & RX_AVAIL != 0 && self.iflag & RX_AVAIL != 0 {
            result.irq = true;
        }
        result
    }

    /// Bring the controller forward to absolute cycle `now`. Idempotent; a
    /// `now` at or behind the anchor is ignored.
    fn advance_to(&mut self, now: u64) -> PeripheralTickResult {
        if now <= self.anchor {
            return PeripheralTickResult::default();
        }
        let elapsed = now - self.anchor;
        self.anchor = now;
        self.service(elapsed)
    }

    /// CORE clocks until this controller next needs the CPU, or `None` when it
    /// is idle and the chain may die.
    ///
    /// ⚠️ Scanning re-arms at **1**, which is not a poll rate this model picks
    /// — it is the walk's cadence, reproduced. The air is a broadcast medium
    /// written by ANOTHER machine, so nothing in this one can raise an event
    /// when a peer transmits; the only faithful answer is to look as often as
    /// the walk looked. The drain loop delivers one such event per scheduler
    /// drain, so on a batched bus this costs one heap operation per peripheral
    /// tick, not one per cycle, and RX latency quantises to the tick interval
    /// exactly like every other level-triggered model on the bus.
    ///
    /// A controller with neither bit set — which is every lab that does not use
    /// BLE at all, including every bus that merely has this block mapped — arms
    /// nothing.
    fn cycles_to_next_service(&self) -> Option<u64> {
        let scanning = self.ctrl & CTRL_SCAN_EN != 0;
        let advertising = self.ctrl & CTRL_ADV_EN != 0;
        match (scanning, advertising) {
            (true, _) => Some(1),
            (false, true) => Some(
                self.adv_period_cycles()
                    .saturating_sub(self.since_adv)
                    .max(1),
            ),
            (false, false) => None,
        }
    }
}

fn word_from(buf: &[u8; BUF_LEN], at: usize) -> u32 {
    let mut w = 0u32;
    for i in 0..4 {
        if let Some(b) = buf.get(at + i) {
            w |= (*b as u32) << (i * 8);
        }
    }
    w
}

fn word_into(buf: &mut [u8; BUF_LEN], at: usize, value: u32) {
    for i in 0..4 {
        if let Some(slot) = buf.get_mut(at + i) {
            *slot = ((value >> (i * 8)) & 0xFF) as u8;
        }
    }
}

impl crate::Peripheral for VirtualBle {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_word(offset & !3);
        Ok(((word >> ((offset % 4) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let shift = (offset % 4) * 8;
        let merged = (self.read_word(reg) & !(0xFFu32 << shift)) | ((value as u32) << shift);
        self.write_word(reg, merged);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_word(offset))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_word(offset, value);
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    /// The controller has to be visited: advertising is periodic and the air
    /// has to be drained even while firmware is busy elsewhere. In scheduler
    /// mode the event chain is what visits it — at the advertising deadline,
    /// and at the walk's own cadence while scanning (see
    /// [`VirtualBle::cycles_to_next_service`]) — so the walk has nothing left
    /// to do. Without a clock the walk is the only thing that drains the air
    /// and the conservative `true` stands.
    fn needs_legacy_walk(&self) -> bool {
        !self.scheduler_mode()
    }

    fn attach_cycle_clock(&mut self, clock: crate::CycleClock) {
        self.anchor = clock.now();
        self.clock = Some(clock);
    }

    /// Bring the advertising interval forward before an MMIO write observes the
    /// controller, so a `CTRL` rewrite or an `RXCMD` pop sees the same state the
    /// walk would have left at this cycle.
    fn sync_to(&mut self, now_cycle: u64) {
        if self.scheduler_mode() {
            // The IRQ verdict is dropped deliberately: a write path cannot pend
            // a line. The chain re-derives the held level on its next visit,
            // exactly as the walk's next tick would.
            let _ = self.advance_to(now_cycle);
        }
    }

    /// Arm the service chain when a write leaves the controller with work —
    /// enabling advertising or scanning. delay-0 → deadline
    /// `current_cycle + 1`, the cycle the walk's next tick would have run on;
    /// the chain then perpetuates itself from `on_event`.
    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if !self.scheduler_mode() || self.chain_live {
            return Vec::new();
        }
        match self.cycles_to_next_service() {
            Some(d) => {
                self.chain_live = true;
                vec![(d - 1, 0u32)]
            }
            None => Vec::new(),
        }
    }

    fn on_event(
        &mut self,
        _event_token: u32,
        sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if !self.scheduler_mode() {
            return crate::sched::EventResult::default();
        }
        let irq = self.advance_to(sched.now()).irq;
        let delay = self.cycles_to_next_service();
        self.chain_live = delay.is_some();
        crate::sched::EventResult {
            raise_own_irq: irq,
            reschedule_delay: delay,
            ..Default::default()
        }
    }

    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        // A scheduler-mode instance is walk-skipped and the event chain owns
        // the service; the guard keeps a stray direct call from advertising
        // twice.
        if self.scheduler_mode() {
            return PeripheralTickResult::default();
        }
        self.service(cycles)
    }

    /// The bare-CPU hardware oracle freezes the CPU and deliberately asks for
    /// the pre-scheduler one-tick service, so the `scheduler_mode()` no-op in
    /// [`Self::tick_elapsed`] must NOT apply here.
    fn tick_elapsed_forced(&mut self, cycles: u64) -> PeripheralTickResult {
        self.service(cycles)
    }

    /// `ADVINTERVAL` is specified in 625 µs units, so the controller needs the
    /// core clock to turn an interval into cycles. Taken from the bus rather
    /// than from a `config:` key so `ChipDescriptor::cpu_hz` stays the one
    /// place the frequency is written.
    fn attach_cpu_hz(&mut self, hz: u64) {
        self.cpu_hz = hz.max(1);
    }

    fn attach_ble_air(&mut self, air: BleAirBus) {
        self.rx_cursor = air.current_seq();
        self.air = air;
        // Anything queued came off the OLD air and belongs to a lab this
        // controller is no longer in.
        self.queued.clear();
        self.iflag &= !RX_AVAIL;
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    /// ⚠️ The mutable accessor DEFAULTS TO `None`, and the walk-vs-scheduler
    /// differential reaches this model mutably to pin it back onto the walk.
    /// Without it the reference lane silently becomes the candidate lane.
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

    /// 78 MHz — the EFR32MG26's `cpu_hz`, so the interval arithmetic below is
    /// the arithmetic the BRD2709A actually does.
    const MG26_HZ: u64 = 78_000_000;

    fn pair() -> (VirtualBle, VirtualBle) {
        let air = BleAirBus::new();
        (
            VirtualBle::new(air.clone(), MG26_HZ),
            VirtualBle::new(air, MG26_HZ),
        )
    }

    /// A legacy `ADV_NONCONN_IND` carrying manufacturer data: header 0x02,
    /// length, AdvA, then an AD structure.
    fn beacon_pdu(tag: u8) -> Vec<u8> {
        let mut pdu = vec![0x02, 0x0C];
        pdu.extend_from_slice(&[0x11, 0x22, 0x33, 0x44, 0x55, 0x66]); // AdvA
        pdu.extend_from_slice(&[0x05, 0xFF, 0xE5, 0x02, tag, 0x01]); // manufacturer AD
        pdu
    }

    fn stage(dev: &mut VirtualBle, pdu: &[u8]) {
        for (i, chunk) in pdu.chunks(4).enumerate() {
            let mut w = 0u32;
            for (j, b) in chunk.iter().enumerate() {
                w |= (*b as u32) << (j * 8);
            }
            dev.write_u32(OFF_TXBUF + (i * 4) as u64, w).unwrap();
        }
        dev.write_u32(OFF_TXLEN, pdu.len() as u32).unwrap();
    }

    fn popped(dev: &mut VirtualBle) -> Vec<u8> {
        dev.write_u32(OFF_RXCMD, 1).unwrap();
        let len = dev.read_u32(OFF_RXLEN).unwrap() as usize;
        (0..len)
            .map(|i| dev.read(OFF_RXBUF + i as u64).unwrap())
            .collect()
    }

    #[test]
    fn the_id_register_says_this_is_not_silicon() {
        let (dev, _) = pair();
        assert_eq!(dev.read_u32(OFF_ID).unwrap(), LWBL_MAGIC);
        let ascii: Vec<u8> = LWBL_MAGIC.to_le_bytes().to_vec();
        assert_eq!(&ascii, b"LWBL", "the magic must read as ASCII in a dump");
    }

    #[test]
    fn resets_to_the_advertising_parameters() {
        let (dev, _) = pair();
        assert_eq!(dev.read_u32(OFF_ACCESSADDR).unwrap(), ADV_ACCESS_ADDRESS);
        assert_eq!(dev.read_u32(OFF_CRCINIT).unwrap(), ADV_CRC_INIT);
        assert_eq!(dev.read_u32(OFF_CTRL).unwrap(), 0, "radio starts off");
        assert_eq!(dev.read_u32(OFF_STATUS).unwrap(), 0, "nothing queued");
    }

    #[test]
    fn a_scanner_receives_exactly_the_pdu_the_advertiser_staged() {
        let (mut adv, mut scan) = pair();

        scan.write_u32(OFF_CHANNEL, 37).unwrap();
        scan.write_u32(OFF_CTRL, CTRL_SCAN_EN).unwrap();

        let pdu = beacon_pdu(0xA5);
        stage(&mut adv, &pdu);
        adv.write_u32(OFF_CTRL, CTRL_ADV_EN).unwrap();

        scan.tick_elapsed(1);
        assert_eq!(scan.read_u32(OFF_STATUS).unwrap() & RX_AVAIL, RX_AVAIL);
        assert_eq!(popped(&mut scan), pdu, "the PDU crosses the air unchanged");
        assert_eq!(scan.read_u32(OFF_RXCHANNEL).unwrap(), 37);
    }

    #[test]
    fn advertising_visits_all_three_primary_channels() {
        let air = BleAirBus::new();
        let mut adv = VirtualBle::new(air.clone(), MG26_HZ);
        stage(&mut adv, &beacon_pdu(0x01));

        let mut scanners: Vec<(u8, VirtualBle)> = ADV_CHANNELS
            .iter()
            .map(|ch| (*ch, VirtualBle::new(air.clone(), MG26_HZ)))
            .collect();
        for (ch, s) in &mut scanners {
            s.write_u32(OFF_CHANNEL, *ch as u32).unwrap();
            s.write_u32(OFF_CTRL, CTRL_SCAN_EN).unwrap();
        }

        adv.write_u32(OFF_CTRL, CTRL_ADV_EN).unwrap();
        for (ch, s) in &mut scanners {
            s.tick_elapsed(1);
            assert_eq!(
                s.read_u32(OFF_STATUS).unwrap() & RX_AVAIL,
                RX_AVAIL,
                "channel {ch} heard nothing"
            );
        }
    }

    #[test]
    fn a_scanner_on_another_channel_hears_nothing() {
        let (mut adv, mut scan) = pair();
        scan.write_u32(OFF_CHANNEL, 5).unwrap(); // a data channel
        scan.write_u32(OFF_CTRL, CTRL_SCAN_EN).unwrap();

        stage(&mut adv, &beacon_pdu(0x02));
        adv.write_u32(OFF_CTRL, CTRL_ADV_EN).unwrap();

        scan.tick_elapsed(1);
        assert_eq!(scan.read_u32(OFF_STATUS).unwrap() & RX_AVAIL, 0);
    }

    /// Access-address selectivity: a scanner parked on the advertising address
    /// must not decode connection traffic that happens to share a channel.
    #[test]
    fn a_scanner_filters_on_the_access_address() {
        let (mut adv, mut scan) = pair();
        scan.write_u32(OFF_CHANNEL, 37).unwrap();
        scan.write_u32(OFF_CTRL, CTRL_SCAN_EN).unwrap();

        adv.write_u32(OFF_ACCESSADDR, 0x1234_5678).unwrap();
        stage(&mut adv, &beacon_pdu(0x03));
        adv.write_u32(OFF_CTRL, CTRL_ADV_EN).unwrap();

        scan.tick_elapsed(1);
        assert_eq!(scan.read_u32(OFF_STATUS).unwrap() & RX_AVAIL, 0);
    }

    #[test]
    fn a_controller_never_hears_itself() {
        let air = BleAirBus::new();
        let mut dev = VirtualBle::new(air, MG26_HZ);
        dev.write_u32(OFF_CHANNEL, 37).unwrap();
        stage(&mut dev, &beacon_pdu(0x04));
        // Advertise AND scan at once — a real controller cannot decode its own
        // transmission, and neither does this one.
        dev.write_u32(OFF_CTRL, CTRL_ADV_EN | CTRL_SCAN_EN).unwrap();
        dev.tick_elapsed(1);
        assert_eq!(dev.read_u32(OFF_STATUS).unwrap() & RX_AVAIL, 0);
    }

    #[test]
    fn advertising_repeats_once_per_interval_not_once_per_tick() {
        let (mut adv, mut scan) = pair();
        scan.write_u32(OFF_CHANNEL, 37).unwrap();
        scan.write_u32(OFF_CTRL, CTRL_SCAN_EN).unwrap();

        stage(&mut adv, &beacon_pdu(0x05));
        // 160 units = 100 ms. At 78 MHz that is 7_800_000 cycles.
        adv.write_u32(OFF_ADVINTERVAL, 160).unwrap();
        assert_eq!(adv.adv_period_cycles(), 7_800_000);

        adv.write_u32(OFF_CTRL, CTRL_ADV_EN).unwrap(); // burst 1, on enable
        adv.tick_elapsed(7_799_999);
        adv.tick_elapsed(1); // burst 2, exactly on the boundary
        adv.tick_elapsed(7_799_998); // not yet burst 3

        scan.tick_elapsed(1);
        let mut heard = 0;
        while scan.read_u32(OFF_STATUS).unwrap() & RX_AVAIL != 0 {
            popped(&mut scan);
            heard += 1;
        }
        assert_eq!(heard, 2, "two intervals elapsed, so two advertisements");
    }

    #[test]
    fn a_single_shot_transmit_does_not_need_advertising_enabled() {
        let (mut tx, mut rx) = pair();
        rx.write_u32(OFF_CHANNEL, 39).unwrap();
        rx.write_u32(OFF_CTRL, CTRL_SCAN_EN).unwrap();

        let pdu = beacon_pdu(0x06);
        stage(&mut tx, &pdu);
        tx.write_u32(OFF_CHANNEL, 39).unwrap();
        tx.write_u32(OFF_TXCMD, 1).unwrap();

        rx.tick_elapsed(1);
        assert_eq!(popped(&mut rx), pdu);
    }

    #[test]
    fn the_interrupt_follows_ien_and_the_flag_is_write_one_to_clear() {
        let (mut adv, mut scan) = pair();
        scan.write_u32(OFF_CHANNEL, 37).unwrap();
        scan.write_u32(OFF_CTRL, CTRL_SCAN_EN).unwrap();
        stage(&mut adv, &beacon_pdu(0x07));

        // IEN clear: a frame arrives, the flag sets, but no interrupt is raised.
        adv.write_u32(OFF_CTRL, CTRL_ADV_EN).unwrap();
        let r = scan.tick_elapsed(1);
        assert!(!r.irq);
        assert_eq!(scan.read_u32(OFF_IF).unwrap() & RX_AVAIL, RX_AVAIL);

        // Draining the queue clears the flag; writing 1 clears it too.
        scan.write_u32(OFF_IF, RX_AVAIL).unwrap();
        assert_eq!(scan.read_u32(OFF_IF).unwrap() & RX_AVAIL, 0);

        scan.write_u32(OFF_IEN, RX_AVAIL).unwrap();
        adv.write_u32(OFF_TXCMD, 1).unwrap();
        adv.write_u32(OFF_CHANNEL, 37).unwrap();
        adv.write_u32(OFF_TXCMD, 1).unwrap();
        let r = scan.tick_elapsed(1);
        assert!(r.irq, "IEN.RX set and a frame queued must interrupt");
    }

    #[test]
    fn a_scanner_that_stops_draining_loses_the_oldest_frames() {
        let (mut adv, mut scan) = pair();
        scan.write_u32(OFF_CHANNEL, 37).unwrap();
        scan.write_u32(OFF_CTRL, CTRL_SCAN_EN).unwrap();
        adv.write_u32(OFF_CHANNEL, 37).unwrap();

        for tag in 0..(RX_DEPTH as u8 + 4) {
            stage(&mut adv, &beacon_pdu(tag));
            adv.write_u32(OFF_TXCMD, 1).unwrap();
            scan.tick_elapsed(1);
        }
        assert_eq!(scan.queued.len(), RX_DEPTH, "the queue is bounded");
        // The oldest survivor is frame 4, not frame 0. Byte 12 is the tag:
        // header, length, six AdvA bytes, then the AD structure
        // `05 FF E5 02 <tag> 01`.
        let first = popped(&mut scan);
        assert_eq!(
            first[12], 4,
            "the oldest frames were dropped, not the newest"
        );
    }

    #[test]
    fn popping_an_empty_queue_reads_a_zero_length_not_a_stale_frame() {
        let (mut adv, mut scan) = pair();
        scan.write_u32(OFF_CHANNEL, 37).unwrap();
        scan.write_u32(OFF_CTRL, CTRL_SCAN_EN).unwrap();
        adv.write_u32(OFF_CHANNEL, 37).unwrap();
        stage(&mut adv, &beacon_pdu(0x08));
        adv.write_u32(OFF_TXCMD, 1).unwrap();
        scan.tick_elapsed(1);

        assert!(!popped(&mut scan).is_empty());
        assert_eq!(popped(&mut scan).len(), 0, "second pop finds nothing");
        assert_eq!(scan.read_u32(OFF_RXLEN).unwrap(), 0);
    }

    /// A zero interval must not be taken literally: transmitting every tick
    /// would flood the air, and no real controller accepts it either.
    #[test]
    fn a_zero_advertising_interval_is_clamped() {
        let (mut adv, _) = pair();
        adv.write_u32(OFF_ADVINTERVAL, 0).unwrap();
        assert!(adv.adv_period_cycles() >= 1);
    }
}
