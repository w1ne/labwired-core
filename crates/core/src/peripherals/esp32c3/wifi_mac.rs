// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C3 WiFi MAC (`0x6003_3000`, 12 KiB) — behavioral model for the
//! MAC ↔ SimNet bridge.
//!
//! Most of the MAC window is register-backed (the driver's bring-up does
//! read-modify-write + driver-managed scratch — see the Layer-2 RE in
//! `docs/esp32c3_wifi_mac_bridge.md`). On top of that this model implements the
//! pieces that need real behaviour to move frames:
//!
//! * **MAC-ready** (`0xD14` bit0): the HAL busy-polls it before `mac_txrx_init`;
//!   reported set (folds in the former standalone `wifi_mac_ready` override).
//! * **RX descriptor ring**: the driver writes the ring base to `0x88` — a
//!   singly-linked list of `{flags|len, buffer_ptr, next_ptr}` descriptors
//!   (word0 bit31 = owner/HW-may-fill, low 16 = buffer capacity, 1600).
//! * **Interrupt + event register** (`0xC3C` get / `0xC40` W1C clear): the MAC
//!   ISR `wDev_ProcessFiq` reads the event word; RX-success is `0x0100_4000`.
//!   The MAC interrupt is interrupt-matrix source 0.
//!
//! **RX inject** (`queue_rx_frame`): a received 802.11 frame is queued; on the
//! next bus tick the model walks the RX ring for an owner descriptor, DMAs the
//! frame into its buffer, writes back word0 (received length, owner cleared),
//! sets the RX-success event bits, and — while the event is pending — emits MAC
//! interrupt source 0 (level-sensitive, like the SYSTIMER), so the trap path
//! runs `wDev_ProcessFiq` → `lmacProcessRxSucData`.
//!
//! This is the SimNet-facing endpoint of the bridge; the frame source/sink (a
//! frame-level `VirtualAp`) pushes/pulls via `queue_rx_frame` / `take_tx_frames`.

use crate::{Bus, CycleClock, Peripheral, PeripheralTickResult, SimResult};
use std::collections::VecDeque;
use std::sync::atomic::AtomicU32;

/// Debug: base address of the RX buffer the model most recently delivered a
/// frame into. The bus's `LABWIRED_RXBUF_TRACE` read-trace reads this to log
/// the driver's reads relative to the buffer (RE'ing the rx-control header
/// format). 0 = no delivery yet.
pub static RX_DBG_BUF: AtomicU32 = AtomicU32::new(0);

/// Process-cached `LABWIRED_RXBUF_TRACE` gate. The trace guard sits on the
/// hottest path in the engine (`Bus::read_u32` — every load instruction), and
/// `std::env::var` is a real syscall-backed lookup; checking it per read cost
/// measurable native/wasm throughput (profiling artifact found on the C3 OLED
/// lab). The env var is read ONCE per process — set it before launch, as with
/// any debug trace.
pub(crate) fn rxbuf_trace_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("LABWIRED_RXBUF_TRACE").is_ok())
}

const MAC_READY: u64 = 0xD14; // bit0 polled by hal_init
const RX_RING_BASE: u64 = 0x88; // driver writes the RX descriptor-list head here
/// RX descriptor-reload handshake (`hal_mac_rx_*_dscr_reload`): the driver sets
/// bit0 to hand a descriptor back to the MAC and spins until the MAC clears it.
const RX_DSCR_RELOAD: u64 = 0x84;
/// HW rx-control header bytes the MAC DMAs ahead of the 802.11 frame (CSI off).
/// The driver reads the frame at buf+48 and rssi/rate from header bytes 44/45.
const RX_CTRL_HDR_LEN: usize = 48;
const EVENT_GET: u64 = 0xC3C; // hal_mac_interrupt_get_event reads
const EVENT_CLR: u64 = 0xC40; // hal_mac_interrupt_clr_event writes (W1C)

/// RX-success event bits `wDev_ProcessFiq` routes to `lmacProcessRxSucData`.
const EVENT_RX_DONE: u32 = 0x0100_4000;
// RX descriptor word0 is an ESP `lldesc_t`: size[11:0], length[23:12],
// offset[28:24], sosf[29], eof[30], owner[31]. An empty (HW-owned) descriptor
// reads e.g. 0x80640640 (owner=1, length=size=1600); after the MAC fills it the
// driver expects owner=0, eof=1, length=actual-rx-bytes, size preserved.
const DESC_OWNER: u32 = 1 << 31;
const DESC_EOF: u32 = 1 << 30;
/// Interrupt-matrix source ID for the WiFi MAC (MAC_INTR_MAP @ offset 0).
const MAC_INTR_SOURCE: u32 = 0;

// TX path (RE'd): per-AC PLCP0 register at 0x60033D08 - AC*8 holds the TX buffer
// pointer (low 20 bits, the frame is in DRAM 0x3FC#####) plus flags; writing
// bits 0xC000_0000 kicks transmission. TX is fire-and-forget — the driver does
// not block, so the model captures the frame and asynchronously signals
// TX-complete (event bit 0x80 + per-queue done state) so lmacProcessTxComplete
// runs. Offsets relative to the 0x60033000 base.
const PLCP0_AC0: u64 = 0xD08; // AC1=0xD00, AC2=0xCF8, AC3=0xCF0
const TX_KICK_BITS: u32 = 0xC000_0000;
const EVENT_TX_DONE: u32 = 0x80;
const TXQ_DONE_STATE: u64 = 0xCB0; // hal_mac_get_txq_state mode2 reads &0xF here
/// DRAM window the low-20-bit TX buffer pointer resolves into.
const DRAM_BASE: u32 = 0x3FC0_0000;

/// Capacity of the network-analyzer frame-trace ring buffer.
const TRACE_CAP: usize = 512;

/// One captured 802.11 frame, for the WiFi network analyzer (the WiFi analog of
/// the BLE `AirFrameTrace`). `bytes` is the raw 802.11 frame as it crossed the
/// MAC; the analyzer UI decodes the type/addresses/payload. Direction is from
/// the device's point of view: `"tx"` = transmitted by this STA, `"rx"` =
/// delivered to this STA from the (virtual) air.
#[derive(Debug, Clone, serde::Serialize)]
pub struct WifiFrameTrace {
    /// Monotonic capture sequence (per MAC), so the UI can order/dedup.
    pub seq: u64,
    /// `"tx"` or `"rx"` from this device's perspective.
    pub dir: &'static str,
    /// Raw 802.11 frame bytes (no rx-control header / FCS).
    pub bytes: Vec<u8>,
    /// Receive signal strength (dBm, signed) for `"rx"` frames; 0 for `"tx"`.
    pub rssi: i8,
}

#[derive(Debug)]
pub struct Esp32c3WifiMac {
    regs: Vec<u32>,
    /// RX descriptor-ring head (DRAM pointer the driver wrote to `0x88`).
    rx_ring: u32,
    /// Frames received from the virtual network, awaiting DMA into the RX ring.
    pending_rx: VecDeque<Vec<u8>>,
    /// Frames the real MAC transmitted (captured TX), for the bridge to drain.
    tx_out: VecDeque<Vec<u8>>,
    /// Pending TX kicks: (access category, PLCP0 register value) recorded on the
    /// 0xC000_0000 write; processed (frame captured + TX-complete signaled) on
    /// the next bus tick, which has bus access to read the frame from DRAM.
    pending_tx: VecDeque<(u8, u32)>,
    /// Network-analyzer ring buffer of recently captured frames (TX + RX).
    trace: VecDeque<WifiFrameTrace>,
    /// Next capture sequence number.
    trace_seq: u64,
    /// When true, this MAC is attached to the shared [`virtual_wifi`] medium:
    /// transmitted frames are submitted to it and the medium's per-station inbox
    /// is pulled into the RX ring each tick. When false, the model uses the
    /// `tx_out`/`queue_rx_*` path driven by the CLI bridge (single-device runs).
    /// Medium mode is what lets two C3 instances talk over one virtual AP.
    medium_mode: bool,
    /// This station's own MAC, learned from the `addr2` (SA) of the first frame
    /// it transmits — so we needn't predict the eFuse byte order. Used to pull
    /// this station's medium inbox and to beacon it.
    medium_mac: Option<[u8; 6]>,
    /// Tick counter for periodic beacon injection in medium mode.
    medium_beacon_ctr: u32,
    /// Bus-published cycle clock (walk-free plan). `Some` once
    /// [`SystemBus::add_peripheral`](crate::bus::SystemBus) attaches it (under
    /// the `event-scheduler` feature); its presence flips the model onto the
    /// event-scheduler drive mode. In that mode the per-cycle legacy walk skips
    /// this peripheral: the MAC interrupt LEVEL (source 0, asserted while an
    /// event is pending) is exported through [`Self::matrix_irq_sources`] and
    /// re-derived by the bus (`refresh_esp32c3_sched_sources`, run on the event
    /// path and — crucially, on a walk-DELETED bus — at the MMIO write choke so
    /// the level de-asserts after `EVENT_CLR`), rather than re-emitted every
    /// tick by [`Self::tick`]. The descriptor-ring PUMP is orthogonal: it rides
    /// the write-armed, self-perpetuating bus-tick path
    /// ([`Self::needs_bus_tick`]), so it costs nothing while WiFi is idle. `None`
    /// (feature off, a hand-built bus, or the differential's
    /// [`Self::force_legacy_walk`]) keeps the legacy per-cycle walk. Not
    /// serialized — re-attached by the bus.
    clock: Option<CycleClock>,
    /// The shared WiFi medium this MAC submits to / pulls its inbox from in
    /// medium mode. `new()` binds the process-global default; `with_wifi` binds
    /// an explicit per-group bus (MACs sharing a bus form one virtual network).
    wifi: super::virtual_wifi::VirtualWifiBus,
}

impl Default for Esp32c3WifiMac {
    fn default() -> Self {
        Self::new()
    }
}

impl Esp32c3WifiMac {
    pub fn new() -> Self {
        Self {
            regs: vec![0u32; 0x3000 / 4],
            rx_ring: 0,
            pending_rx: VecDeque::new(),
            tx_out: VecDeque::new(),
            pending_tx: VecDeque::new(),
            trace: VecDeque::new(),
            trace_seq: 0,
            medium_mode: false,
            medium_mac: None,
            medium_beacon_ctr: 0,
            clock: None,
            wifi: super::virtual_wifi::default_medium(),
        }
    }

    /// Build a MAC bound to an explicit WiFi bus. MACs sharing a bus form one
    /// virtual network (same AP, DHCP, STA↔STA routing); MACs on different buses
    /// are isolated. Prefer over `new()`'s process-global default.
    pub fn with_wifi(wifi: super::virtual_wifi::VirtualWifiBus) -> Self {
        Self {
            wifi,
            ..Self::new()
        }
    }

    /// True when the event scheduler owns this block's interrupt-level drive
    /// (feature on AND bus clock attached). One predicate so the walk and
    /// scheduler drive modes can never mix, mirroring the C3 SARADC/I²C/LEDC
    /// migrations.
    #[inline]
    fn scheduler_mode(&self) -> bool {
        cfg!(feature = "event-scheduler") && self.clock.is_some()
    }

    /// Test/differential knob: detach the cycle clock, pinning the model to the
    /// legacy per-cycle walk (`uses_scheduler() == false`). Used by the
    /// walk-on-vs-scheduler differential gates to build the reference config
    /// from the same bus assembly (mirrors `Esp32c3ApbSarAdc::force_legacy_walk`).
    pub fn force_legacy_walk(&mut self) {
        self.clock = None;
    }

    /// Attach this MAC to the shared [`virtual_wifi`] medium. Enables medium
    /// mode: TX frames are submitted to the medium (which learns this station's
    /// MAC from the frame's SA), and the medium's inbox is pulled into the RX
    /// ring each tick. Two C3 instances both attached share one virtual AP.
    pub fn attach_to_medium(&mut self) {
        self.medium_mode = true;
    }

    /// Record a captured frame in the analyzer ring buffer (oldest dropped at
    /// `TRACE_CAP`). `dir` is `"tx"` or `"rx"` from this device's perspective.
    fn trace_push(&mut self, dir: &'static str, bytes: &[u8], rssi: i8) {
        if self.trace.len() == TRACE_CAP {
            self.trace.pop_front();
        }
        self.trace.push_back(WifiFrameTrace {
            seq: self.trace_seq,
            dir,
            bytes: bytes.to_vec(),
            rssi,
        });
        self.trace_seq += 1;
    }

    /// Non-consuming snapshot of the analyzer frame trace, most-recent first.
    /// The network-analyzer UI polls this each tick (mirrors the BLE
    /// `air_trace_snapshot`).
    pub fn trace_snapshot(&self) -> Vec<WifiFrameTrace> {
        self.trace.iter().rev().cloned().collect()
    }

    /// Clear the analyzer frame trace (UI "reset").
    pub fn trace_clear(&mut self) {
        self.trace.clear();
    }

    /// Length of the 802.11 frame in a captured TX buffer: parse the MAC header,
    /// LLC/SNAP, and IP total-length when it's a data frame; otherwise return the
    /// whole buffer. (The HW length lives in a separate per-AC register we don't
    /// decode; parsing the frame is robust enough for the bridge.)
    fn tx_frame_len(buf: &[u8]) -> usize {
        if buf.len() < 2 {
            return buf.len();
        }
        let fc0 = buf[0];
        let ftype = (fc0 >> 2) & 0x3;
        if ftype == 2 {
            // Data frame: [hdr 24, +2 if QoS][LLC/SNAP 8][IP...]. ethertype is
            // the last 2 bytes of SNAP; if IPv4 (0x0800), use IP total length.
            let hdr = if (fc0 & 0x80) != 0 { 26 } else { 24 };
            let snap = hdr + 8;
            if buf.len() >= snap {
                let ethertype = u16::from_be_bytes([buf[snap - 2], buf[snap - 1]]);
                if ethertype == 0x0800 && buf.len() >= snap + 4 {
                    let ip_total = u16::from_be_bytes([buf[snap + 2], buf[snap + 3]]) as usize;
                    return (snap + ip_total).min(buf.len());
                }
            }
        }
        buf.len()
    }

    /// Process one pending TX kick: read the transmitted 802.11 frame from DRAM,
    /// stash it for the bridge, and signal TX-complete (success) to the driver.
    fn process_tx(&mut self, bus: &mut dyn Bus) {
        let Some((ac, plcp0)) = self.pending_tx.pop_front() else {
            return;
        };
        // PLCP0 low-20 bits point at a TX lldesc (word0=flags, word1=buffer
        // pointer, word2=next), not the frame itself. Follow word1 to the frame.
        let desc = DRAM_BASE | (plcp0 & 0x000F_FFFF);
        let bufptr = bus.read_u32(desc as u64 + 4).unwrap_or(0);
        if !(DRAM_BASE..DRAM_BASE + 0x10_0000).contains(&bufptr) {
            return; // not a resolvable DRAM frame pointer
        }
        let mut raw = vec![0u8; 1600];
        for (i, b) in raw.iter_mut().enumerate() {
            *b = bus.read_u8(bufptr as u64 + i as u64).unwrap_or(0);
        }
        let len = Self::tx_frame_len(&raw);
        raw.truncate(len);
        if std::env::var("LABWIRED_MAC_TRACE").is_ok() {
            let head: Vec<String> = raw.iter().take(72).map(|b| format!("{b:02x}")).collect();
            eprintln!(
                "[tx] ac={ac} buf={bufptr:#010x} len={len}: {}",
                head.join(" ")
            );
        }
        self.trace_push("tx", &raw, 0);
        if self.medium_mode {
            // Medium mode: learn our own MAC from the frame's SA (addr2, bytes
            // 10..16) and hand the frame to the shared virtual AP, which responds
            // / routes it to the destination station.
            if raw.len() >= 16 {
                let mut sa = [0u8; 6];
                sa.copy_from_slice(&raw[10..16]);
                if self.medium_mac.is_none() && sa != [0u8; 6] {
                    self.medium_mac = Some(sa);
                }
                self.wifi.submit(sa, &raw);
            }
        } else {
            self.tx_out.push_back(raw);
        }
        // Signal TX-complete: set the per-queue done state (mode2 reads &0xF at
        // 0xCB0) and the TX-done event bit, then the level-sensitive MAC IRQ
        // runs lmacProcessTxComplete.
        self.regs[(TXQ_DONE_STATE / 4) as usize] |= 1 << (ac & 0xF);
        self.set_event(EVENT_TX_DONE);
    }

    /// Queue an 802.11 frame received from the virtual network; delivered to the
    /// driver's RX ring on the next bus tick.
    pub fn queue_rx_frame(&mut self, frame: Vec<u8>) {
        self.pending_rx.push_back(frame);
    }

    /// Queue a frame at the FRONT of the RX backlog so it is delivered before
    /// any already-queued frames. Used for unicast responses (auth/assoc resp,
    /// DHCP offer/ack, ARP reply) that must reach the driver inside its per-state
    /// timeout window — they must not sit behind a backlog of periodic beacons.
    pub fn queue_rx_priority(&mut self, frame: Vec<u8>) {
        self.pending_rx.push_front(frame);
    }

    /// Number of RX frames waiting to be delivered. The bridge uses this to avoid
    /// flooding the backlog with beacons (which would delay real responses).
    pub fn pending_rx_len(&self) -> usize {
        self.pending_rx.len()
    }

    /// Drain frames the real MAC transmitted (captured on TX kick).
    pub fn take_tx_frames(&mut self) -> Vec<Vec<u8>> {
        self.tx_out.drain(..).collect()
    }

    fn event(&self) -> u32 {
        self.regs[(EVENT_GET / 4) as usize]
    }

    fn set_event(&mut self, bits: u32) {
        self.regs[(EVENT_GET / 4) as usize] |= bits;
    }

    /// Deliver one pending RX frame into the next owner-held RX descriptor.
    /// Returns true if a frame was delivered (RX event then set).
    fn deliver_one_rx(&mut self, bus: &mut dyn Bus) -> bool {
        if self.pending_rx.is_empty() || self.rx_ring == 0 {
            return false;
        }
        // Walk the singly-linked descriptor list for an owner-held descriptor.
        let mut desc = self.rx_ring;
        for _ in 0..16 {
            if desc == 0 || !(0x3fc0_0000..0x3fd0_0000).contains(&desc) {
                break;
            }
            let w0 = bus.read_u32(desc as u64).unwrap_or(0);
            let buf = bus.read_u32(desc as u64 + 4).unwrap_or(0);
            let next = bus.read_u32(desc as u64 + 8).unwrap_or(0);
            let cap = (w0 & 0xFFF) as usize; // lldesc size field (buffer capacity)
            if w0 & DESC_OWNER != 0 && buf != 0 && cap > 0 {
                let frame = self.pending_rx.pop_front().unwrap();
                self.trace_push("rx", &frame, -60);
                // HW DMA layout: a RX_CTRL_HDR_LEN-byte rx-control header, then
                // the 802.11 frame (CSI off → no CSI block). The driver reads
                // the frame at buf + 48 and rssi/rate from header bytes 44/45.
                let total = (RX_CTRL_HDR_LEN + frame.len()).min(cap);
                let mut hdr = [0u8; RX_CTRL_HDR_LEN];
                // word@0: bit28 (0x1000_0000) = "frame matched an enabled vif"
                // (the STA interface). wDev_ProcessRxSucData's acceptance gate
                // (`word0 & 0x30000000 != 0` and the bit28 check) drops the
                // frame to wDev_DiscardFrame without it.
                hdr[0..4].copy_from_slice(&0x1000_0000u32.to_le_bytes());
                // word@4: EOF/valid bit27 (0x0800_0000) + length[19:8] = total
                // DMA bytes (header + frame); the driver re-packs this.
                let w4 = 0x0800_0000u32 | (((total as u32) & 0xFFF) << 8);
                hdr[4..8].copy_from_slice(&w4.to_le_bytes());
                hdr[44] = 0xC4; // rssi ≈ -60 dBm (signed)
                hdr[45] = 0x0B; // rate (11 Mbps CCK)
                                // hdr[47] left 0 (the driver special-cases 0xF5).
                for (i, b) in hdr.iter().enumerate() {
                    let _ = bus.write_u8(buf as u64 + i as u64, *b);
                }
                for (i, b) in frame.iter().take(total - RX_CTRL_HDR_LEN).enumerate() {
                    let _ = bus.write_u8(buf as u64 + (RX_CTRL_HDR_LEN + i) as u64, *b);
                }
                // Write back the lldesc as HW does on RX completion: owner
                // cleared, eof set, length[23:12] = TOTAL DMA bytes (the driver
                // subtracts the 48-byte header to get the 802.11 length), size
                // preserved.
                let new_w0 = DESC_EOF | (((total as u32) & 0xFFF) << 12) | (w0 & 0xFFF);
                let _ = bus.write_u32(desc as u64, new_w0);
                // RX status registers the driver reads to locate the filled
                // descriptor: 0x90 = last-filled (low 20 bits) + 0xc64 upper,
                // 0x8c = next descriptor (HW's write head).
                self.regs[(0x90 / 4) as usize] = desc & 0x000F_FFFF;
                self.regs[(0xc64 / 4) as usize] = desc & 0xFFF0_0000;
                self.regs[(0x8c / 4) as usize] = next;
                self.set_event(EVENT_RX_DONE);
                RX_DBG_BUF.store(buf, std::sync::atomic::Ordering::Relaxed);
                if rxbuf_trace_enabled() {
                    eprintln!(
                        "[rxinj] desc={desc:#010x} buf={buf:#010x} total={total} (hdr48+frame{})",
                        frame.len()
                    );
                }
                return true;
            }
            desc = next;
        }
        false
    }
}

impl Peripheral for Esp32c3WifiMac {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let w = self.read_u32(offset & !3)?;
        Ok((w >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let sh = (offset & 3) * 8;
        let cur = *self.regs.get((aligned / 4) as usize).unwrap_or(&0);
        self.write_u32(aligned, (cur & !(0xFFu32 << sh)) | ((value as u32) << sh))
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let stored = *self.regs.get((offset / 4) as usize).unwrap_or(&0);
        Ok(match offset {
            // hal_init busy-polls bit0 for MAC clock/reset ready.
            MAC_READY => stored | 1,
            _ => stored,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            // W1C: the ISR clears the event bits it handled.
            EVENT_CLR => {
                if let Some(slot) = self.regs.get_mut((EVENT_GET / 4) as usize) {
                    *slot &= !value;
                }
                if let Some(slot) = self.regs.get_mut((offset / 4) as usize) {
                    *slot = value;
                }
            }
            RX_DSCR_RELOAD => {
                // The MAC consumes/reloads the descriptor instantly: clear the
                // bit0 the driver set, so its spin-until-clear loop exits.
                if let Some(slot) = self.regs.get_mut((offset / 4) as usize) {
                    *slot = value & !1;
                }
            }
            RX_RING_BASE => {
                self.rx_ring = value;
                if let Some(slot) = self.regs.get_mut((offset / 4) as usize) {
                    *slot = value;
                }
            }
            // Per-AC PLCP0 registers (0xD08, 0xD00, 0xCF8, 0xCF0): a write with
            // the kick bits (0xC000_0000) starts transmission of the frame the
            // low 20 bits point at. Record it; the frame is captured + acked on
            // the next bus tick.
            0xCF0 | 0xCF8 | 0xD00 | 0xD08 => {
                if let Some(slot) = self.regs.get_mut((offset / 4) as usize) {
                    *slot = value;
                }
                if value & TX_KICK_BITS == TX_KICK_BITS {
                    let ac = ((PLCP0_AC0 - offset) / 8) as u8;
                    self.pending_tx.push_back((ac, value));
                }
            }
            _ => {
                if let Some(slot) = self.regs.get_mut((offset / 4) as usize) {
                    *slot = value;
                }
            }
        }
        Ok(())
    }

    /// Walk-free plan (the descriptor-ring PUMP axis): the bus runs
    /// [`Self::tick_with_bus`] only while WiFi is actually up, so an idle MAC
    /// (WiFi off / unconfigured — the OLED demo never enables WiFi) arms NOTHING
    /// and drops out of the bus-tick set. This is what lets a walk-DELETED C3
    /// bus take the trivial per-cycle path (`per_cycle_tick_is_trivial` requires
    /// an empty `bus_tick_indices`).
    ///
    /// Each condition is a WRITE-ARMED / setup flag re-consulted by the bus's
    /// existing `refresh_bus_tick_index` (called after every MMIO write and
    /// after every `tick_with_bus`), so no new event machinery is needed:
    ///
    /// * `rx_ring != 0` — the driver enabled the RX ring by writing its head to
    ///   `0x88` (an MMIO write, so it arms the pump through the write choke) and
    ///   the ring stays live for the whole WiFi session. Keying RX on the ring
    ///   (NOT `pending_rx`) is deliberate: `queue_rx_frame` is a non-MMIO
    ///   bridge/medium injection that can't refresh the index — but a frame can
    ///   only be delivered once the ring exists, and once it does the MAC is
    ///   already resident, so externally-queued frames are pumped without any
    ///   extra re-arm. RX-down (ring never enabled) ⇒ idle.
    /// * `!pending_tx.is_empty()` — a driver PLCP0 kick write (`0xC000_0000`
    ///   bits) queued a frame to transmit; the pump drains it and drops out.
    /// * `medium_mode` — a two-C3 medium station polls its shared inbox and
    ///   beacons each tick; the toggle is one-time setup (`attach_to_medium`),
    ///   which rebuilds the tick index once.
    ///
    /// The set is self-perpetuating: after every `tick_with_bus` the bus calls
    /// `refresh_bus_tick_index`, so the pump keeps ticking until it has drained.
    fn needs_bus_tick(&self) -> bool {
        self.rx_ring != 0 || !self.pending_tx.is_empty() || self.medium_mode
    }

    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        // Capture any transmitted frame + signal TX-complete (in medium mode this
        // submits to the shared AP), then in medium mode pull any frames the AP
        // queued for this station, then deliver one queued RX frame into the ring.
        self.process_tx(bus);
        if self.medium_mode {
            if let Some(mac) = self.medium_mac {
                // Periodic beacon so the scanning station keeps seeing the AP.
                self.medium_beacon_ctr = self.medium_beacon_ctr.wrapping_add(1);
                if self.medium_beacon_ctr % 2_000_000 == 0 {
                    self.wifi.queue_beacon(mac, 1);
                }
                for frame in self.wifi.take_inbox(mac) {
                    self.pending_rx.push_back(frame);
                }
            }
        }
        self.deliver_one_rx(bus);
    }

    /// LEGACY per-cycle walk path (the interrupt-LEVEL axis): re-assert the MAC
    /// interrupt source while an event is pending so `wDev_ProcessFiq` runs and
    /// clears it via `EVENT_CLR`. In scheduler mode ([`Self::uses_scheduler`]
    /// true) the walk skips this peripheral and the bus re-derives the level
    /// from [`Self::matrix_irq_sources`] instead; this reporter is a pure no-op
    /// on state, so a stray call is harmless. Matches the SYSTIMER/SARADC level
    /// delivery model.
    fn tick(&mut self) -> PeripheralTickResult {
        if self.event() != 0 {
            PeripheralTickResult {
                explicit_irqs: Some(vec![MAC_INTR_SOURCE]),
                ..PeripheralTickResult::default()
            }
        } else {
            PeripheralTickResult::default()
        }
    }

    fn legacy_tick_active(&self) -> bool {
        self.event() != 0
    }

    fn legacy_tick_dynamic(&self) -> bool {
        true
    }

    /// Walk-free plan: driven by the event scheduler once the bus has attached
    /// its cycle clock (production `add_peripheral` always does, under the
    /// `event-scheduler` feature). The per-cycle walk then skips this
    /// peripheral's interrupt-level reporter; the MAC level is exported through
    /// [`Self::matrix_irq_sources`] and re-derived by the bus. Without a clock
    /// (feature off, a hand-built bus, or `force_legacy_walk`) it stays on the
    /// legacy walk so those callers keep the old exact semantics.
    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    /// The MAC interrupt LEVEL — re-asserting source 0 every walk tick while an
    /// event is pending — is fully reproduced in scheduler mode by the level
    /// export ([`Self::matrix_irq_sources`]) + the write-choke re-derivation, so
    /// the walk is unnecessary there. The descriptor-ring pump does NOT need the
    /// legacy walk either: it rides the write-armed bus-tick path
    /// ([`Self::needs_bus_tick`]), which is orthogonal to the walk. In legacy
    /// mode (no clock / feature off) the walk does real level work and the
    /// conservative `true` stands.
    fn needs_legacy_walk(&self) -> bool {
        !self.scheduler_mode()
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    /// C3 interrupt-matrix level: the MAC source (0) while an event is pending —
    /// the exact condition [`Self::tick`] pushes on the legacy walk. In
    /// scheduler mode the walk no longer re-emits it, so the bus re-derives the
    /// level from here (`refresh_esp32c3_sched_sources`, polled on the event
    /// path and at the MMIO write choke) so the level-sensitive IRQ stays routed
    /// and de-asserts the tick/write after firmware clears the event via
    /// `EVENT_CLR`.
    fn matrix_irq_sources_into(&self, out: &mut Vec<u32>) {
        if self.event() != 0 {
            out.push(MAC_INTR_SOURCE);
        }
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

    #[test]
    fn mac_ready_bit_set() {
        let m = Esp32c3WifiMac::new();
        assert_eq!(m.read_u32(MAC_READY).unwrap() & 1, 1);
    }

    #[test]
    fn event_clear_is_w1c() {
        let mut m = Esp32c3WifiMac::new();
        m.set_event(EVENT_RX_DONE);
        assert_eq!(m.read_u32(EVENT_GET).unwrap(), EVENT_RX_DONE);
        m.write_u32(EVENT_CLR, EVENT_RX_DONE).unwrap();
        assert_eq!(m.read_u32(EVENT_GET).unwrap(), 0);
    }

    #[test]
    fn tick_emits_mac_irq_while_event_pending() {
        let mut m = Esp32c3WifiMac::new();
        assert!(
            !m.legacy_tick_active(),
            "idle level-IRQ WiFi MAC must stay out of the legacy tick walk"
        );
        assert!(
            m.legacy_tick_dynamic(),
            "event-producing bus ticks and W1C clears must refresh tick membership"
        );
        assert!(m.tick().explicit_irqs.is_none());
        m.set_event(EVENT_RX_DONE);
        assert!(
            m.legacy_tick_active(),
            "pending MAC event needs level ticks"
        );
        assert_eq!(
            m.tick().explicit_irqs.as_deref(),
            Some(&[MAC_INTR_SOURCE][..])
        );
        m.write_u32(EVENT_CLR, EVENT_RX_DONE).unwrap();
        assert!(
            !m.legacy_tick_active(),
            "cleared MAC event can leave tick walk"
        );
    }

    #[test]
    fn analyzer_trace_captures_rx_and_caps() {
        let mut m = Esp32c3WifiMac::new();
        // RX capture path.
        m.trace_push("rx", &[0x80, 0x00, 0xde, 0xad], -42);
        m.trace_push("tx", &[0x40, 0x00], 0);
        let snap = m.trace_snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].dir, "tx"); // most-recent first
        assert_eq!(snap[1].dir, "rx");
        assert_eq!(snap[1].rssi, -42);
        assert_eq!(snap[1].bytes, vec![0x80, 0x00, 0xde, 0xad]);
        assert_eq!(snap[1].seq, 0);
        // Ring cap: oldest dropped beyond TRACE_CAP.
        for _ in 0..TRACE_CAP {
            m.trace_push("rx", &[0u8; 4], -60);
        }
        assert_eq!(m.trace_snapshot().len(), TRACE_CAP);
        m.trace_clear();
        assert!(m.trace_snapshot().is_empty());
    }

    #[test]
    fn rx_inject_walks_ring_and_fills_descriptor() {
        // Lay out a 1-entry RX ring in RAM: desc @ 0x3fca4904, buffer @ 0x3fca4980.
        let mut bus = SystemBus::new();
        bus.ram.base_addr = 0x3fc8_0000;
        bus.ram.data = vec![0u8; 0x40000];
        let desc = 0x3fca_4904u32;
        let buf = 0x3fca_4980u32;
        bus.write_u32(desc as u64, DESC_OWNER | 1600).unwrap(); // owner, cap=1600
        bus.write_u32(desc as u64 + 4, buf).unwrap(); // buffer ptr
        bus.write_u32(desc as u64 + 8, 0).unwrap(); // next = end

        let mut mac = Esp32c3WifiMac::new();
        mac.write_u32(RX_RING_BASE, desc).unwrap();
        let frame = vec![0xB0u8, 0x00, 0xAA, 0xBB]; // a tiny "802.11" frame
        mac.queue_rx_frame(frame.clone());
        mac.tick_with_bus(&mut bus);

        // Descriptor word0 (lldesc): owner cleared, eof set, length[23:12] =
        // total DMA bytes (48-byte rx-control header + 4-byte frame = 52),
        // size[11:0]=1600 preserved.
        let w0 = bus.read_u32(desc as u64).unwrap();
        assert_eq!(w0 & DESC_OWNER, 0, "owner cleared after RX");
        assert_ne!(w0 & DESC_EOF, 0, "eof set");
        assert_eq!(
            (w0 >> 12) & 0xFFF,
            (RX_CTRL_HDR_LEN as u32) + 4,
            "length field = header + frame bytes"
        );
        assert_eq!(w0 & 0xFFF, 1600, "buffer size preserved");
        // rssi/rate in the rx-control header; 802.11 frame at buf + 48.
        assert_eq!(bus.read_u8(buf as u64 + 44).unwrap(), 0xC4, "rssi byte");
        for (i, b) in frame.iter().enumerate() {
            assert_eq!(
                bus.read_u8(buf as u64 + RX_CTRL_HDR_LEN as u64 + i as u64)
                    .unwrap(),
                *b,
                "frame byte at header+offset"
            );
        }
        // RX event set → MAC IRQ asserted.
        assert_ne!(mac.event() & EVENT_RX_DONE, 0);
        assert_eq!(
            mac.tick().explicit_irqs.as_deref(),
            Some(&[MAC_INTR_SOURCE][..])
        );
    }
}
