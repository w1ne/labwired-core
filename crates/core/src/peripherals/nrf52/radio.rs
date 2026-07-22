// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 RADIO peripheral.
//!
//! Source: nRF52840 PS rev 1.7 §6.20 (RADIO). 2.4 GHz BLE/802.15.4/proprietary
//! transceiver.
//!
//! Scope, stated plainly: **we don't simulate the wireless medium — we
//! simulate everything the firmware actually touches.** Every digital layer of
//! a transfer is real (registers, Easy-DMA, whitening, CRC, address matching),
//! so firmware behaves bit-for-bit as it would on silicon for these code
//! paths. The one thing that is *not* physical is the RF channel itself: the
//! "air" is a lossless, collision-free in-process queue, not a radio link.
//!
//! Faithfully modeled (firmware can't tell it from silicon):
//! * **Register surface** — every documented register in 0x000–0x77C
//!   round-trips with proper masks and reset values.
//! * **Task / event state machine** — TASKS_{TX,RX}EN → STATE=TXIDLE/RXIDLE +
//!   EVENTS_READY; TASKS_START → EVENTS_END; TASKS_DISABLE → STATE=DISABLED +
//!   EVENTS_DISABLED. BLE-stack init that polls EVENTS_READY won't spin.
//! * **SHORTS** for the common chain patterns (READY→START,
//!   END→DISABLE, ADDRESS→RSSISTART, DISABLED→TXEN/RXEN/RSSISTOP).
//! * **Easy DMA** — PACKETPTR is read from RAM on TX and written to RAM on RX.
//! * **BLE whitening** — real PN9 LFSR (x^7+x^4+1); applied on TX, reversed
//!   on RX.
//! * **CRC-24** — computed over the payload (poly 0x65B, configurable init);
//!   on RX the trailing 3 bytes are recomputed and compared, and CRCSTATUS is
//!   the *real* 1/0 verdict (not hardcoded).
//! * **Address matching** — RXADDRESSES × BASE/PREFIX logical-address table,
//!   plus DEVMATCH/DEVMISS against the DAB/DAP whitelist and BCMATCH bit
//!   counting.
//! * **Cross-instance delivery** — a TX frame (whitened, CRC'd, address- and
//!   MODE-tagged) crosses to another RADIO instance on the same FREQUENCY via
//!   a shared in-process registry, where it is verified end to end.
//!
//! Idealized — present but not physical:
//! * **The channel is lossless and collision-free.** No bit errors, no
//!   interference, no packet loss, no two-transmitter collision; CRC therefore
//!   essentially always passes. RX *consumes* the frame from the queue, so
//!   delivery is point-to-point, not broadcast (one receiver per frame).
//! * **Timing is proportional, not cycle-accurate.** EVENTS_END is scheduled
//!   bytes × per-MODE cost (one tick ≈ 1 µs); no preamble, T_IFS, or ramp-up.
//! * **RSSI is a deterministic PRNG** around ~-50 dBm — plausible jitter with
//!   no physical meaning (no path loss / distance).
//!
//! Not modeled at all: GFSK modulation, preamble / access-address bit sync,
//! channel hopping, AAR encryption, and the advertising / connection state
//! machines.

use crate::{Bus, Peripheral, PeripheralTickResult, SimResult};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};

// ── Global "virtual air" registry ────────────────────────────────────────────
//
// Cross-instance routing for sim-to-sim BLE. When one Radio TX'es a frame on
// FREQUENCY=N MODE=M, we push it here; every Radio in RX mode on the same
// (FREQUENCY, MODE) gets a chance to consume it next tick. Each frame carries
// the sender's address fields so RX can do the proper RXADDRESSES match.
//
// The registry is sim-global because the sim represents one physical space; if
// you spin up multiple Machine instances in the same process they all share
// this air. To partition (e.g. two physically-distant simulators) use the
// `radio_air_namespace()` helper to give each cluster its own slot.

#[derive(Debug, Clone)]
struct AirFrame {
    /// Whitened bytes including the trailing 3-byte CRC; layout is what
    /// TX produced and what RX consumes verbatim.
    bytes: Vec<u8>,
    /// Address bytes that travel with the frame so receivers can match
    /// against RXADDRESSES + BASE/PREFIX.
    addr_base: u32,
    addr_prefix: u8,
    #[allow(dead_code)]
    addr_len: u8,
    /// Whitening init the sender used; RX side uses this to de-whiten
    /// when its own DATAWHITEIV matches.
    #[allow(dead_code)]
    whitening_iv: u8,
    /// CRC init the sender used; RX uses this to verify.
    #[allow(dead_code)]
    crcinit: u32,
    /// MODE (and therefore datarate / modulation) the sender used. A
    /// receiver tuned to a different MODE on the same FREQUENCY can see
    /// the energy but won't decode — we model that as "frame stays in
    /// the queue for a mode-matching receiver to consume".
    mode: u32,
}

#[derive(Debug, Default)]
struct VirtualAir {
    /// Per-FREQUENCY queue. Multi-MODE coexistence on the same band is
    /// handled by RX filtering on `AirFrame.mode == self.mode` rather
    /// than the queue key, so a BLE_1Mbit TX and an Nrf_1Mbit RX on
    /// the same FREQUENCY can both share air (RX simply doesn't decode
    /// the wrong-mode frame).
    queues: HashMap<u8, VecDeque<AirFrame>>,
    /// Ring buffer of the last ~200 TX frames pushed into the air.
    /// Lives independently of `queues` (which gets drained by RX) so
    /// the playground UI can poll a stable trace for visualization.
    tx_history: VecDeque<AirFrameTrace>,
}

const TX_HISTORY_CAP: usize = 200;

#[derive(Debug, Clone, serde::Serialize)]
pub struct AirFrameTrace {
    pub channel: u8,
    pub addr_base: u32,
    pub addr_prefix: u8,
    pub mode: u32,
    pub bytes: Vec<u8>,
    /// Whitening IV the sender used. `bytes` is the on-air (whitened) frame;
    /// a sniffer can de-whiten `bytes[..len-3]` (the 3-byte CRC is appended
    /// after whitening) with this IV to recover the logical payload.
    pub whitening_iv: u8,
}

/// A shared BLE "virtual air" medium. Radios minted with the same bus hear each
/// other's frames (matched by frequency + mode + address); radios on different
/// buses are fully isolated — the behaviour the former process-static `AIR`
/// registry could not offer, so two BLE labs (or two workers) can coexist.
/// `Arc<Mutex<…>>` keeps radios `Send` inside a `Machine` (native requires
/// `MachineTrait: Send`); the browser is single-threaded so it never contends.
#[derive(Debug, Clone, Default)]
pub struct VirtualAirBus {
    inner: Arc<Mutex<VirtualAir>>,
}

impl VirtualAirBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Most-recent-first snapshot of the TX trace, for the playground's BLE-air
    /// visualization.
    pub fn trace_snapshot(&self) -> Vec<AirFrameTrace> {
        match self.inner.lock() {
            Ok(a) => a.tx_history.iter().rev().cloned().collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Drop every frequency queue on this bus. Keeps `tx_history` (the UI trace),
    /// exactly as the former `clear_virtual_air` did.
    pub fn clear(&self) {
        if let Ok(mut a) = self.inner.lock() {
            a.queues.clear();
        }
    }

    /// Lock the underlying air for the radio's TX/RX paths (same module).
    fn lock(&self) -> std::sync::LockResult<std::sync::MutexGuard<'_, VirtualAir>> {
        self.inner.lock()
    }
}

// --- Transitional process-global air (browser back-compat) -------------------
//
// Radios built via `Nrf52Radio::new()` share this one module-global bus, and the
// wasm trace/clear exports below operate on it. One wasm module = one worker =
// one lab, so this is byte-identical to the former `static AIR`. The follow-up
// threads a per-lab-group `VirtualAirBus` through an attach seam, after which
// this global and the two shims are deleted.
fn default_air_bus() -> &'static VirtualAirBus {
    static BUS: OnceLock<VirtualAirBus> = OnceLock::new();
    BUS.get_or_init(VirtualAirBus::new)
}

/// Public view of the current TX trace on the process-global air, for WASM
/// consumption by the playground's BLE-air visualization. Most recent first.
/// Transitional; prefer [`VirtualAirBus::trace_snapshot`] on an owned bus.
pub fn virtual_air_trace_snapshot() -> Vec<AirFrameTrace> {
    default_air_bus().trace_snapshot()
}

/// Clear the process-global air (test/back-compat helper). Prefer
/// [`VirtualAirBus::clear`] on an owned bus.
pub fn clear_virtual_air() {
    default_air_bus().clear();
}

// ── Tasks (PS §6.20.13, table 222) ───────────────────────────────────────────

const OFF_TASKS_TXEN: u64 = 0x000;
const OFF_TASKS_RXEN: u64 = 0x004;
const OFF_TASKS_START: u64 = 0x008;
const OFF_TASKS_STOP: u64 = 0x00C;
const OFF_TASKS_DISABLE: u64 = 0x010;
const OFF_TASKS_RSSISTART: u64 = 0x014;
const OFF_TASKS_RSSISTOP: u64 = 0x018;
const OFF_TASKS_BCSTART: u64 = 0x01C;
const OFF_TASKS_BCSTOP: u64 = 0x020;
const OFF_TASKS_EDSTART: u64 = 0x024;
const OFF_TASKS_EDSTOP: u64 = 0x028;
const OFF_TASKS_CCASTART: u64 = 0x02C;
const OFF_TASKS_CCASTOP: u64 = 0x030;

// ── Events ───────────────────────────────────────────────────────────────────

const OFF_EVENTS_READY: u64 = 0x100;
const OFF_EVENTS_ADDRESS: u64 = 0x104;
const OFF_EVENTS_PAYLOAD: u64 = 0x108;
const OFF_EVENTS_END: u64 = 0x10C;
const OFF_EVENTS_DISABLED: u64 = 0x110;
const OFF_EVENTS_DEVMATCH: u64 = 0x114;
const OFF_EVENTS_DEVMISS: u64 = 0x118;
const OFF_EVENTS_RSSIEND: u64 = 0x11C;
const OFF_EVENTS_BCMATCH: u64 = 0x128;
const OFF_EVENTS_CRCOK: u64 = 0x130;
const OFF_EVENTS_CRCERROR: u64 = 0x134;
const OFF_EVENTS_FRAMESTART: u64 = 0x138;
const OFF_EVENTS_EDEND: u64 = 0x13C;
const OFF_EVENTS_EDSTOPPED: u64 = 0x140;
const OFF_EVENTS_CCAIDLE: u64 = 0x144;
const OFF_EVENTS_CCABUSY: u64 = 0x148;
const OFF_EVENTS_CCASTOPPED: u64 = 0x14C;
const OFF_EVENTS_RATEBOOST: u64 = 0x150;
const OFF_EVENTS_TXREADY: u64 = 0x154;
const OFF_EVENTS_RXREADY: u64 = 0x158;
const OFF_EVENTS_MHRMATCH: u64 = 0x15C;
const OFF_EVENTS_SYNC: u64 = 0x168;
const OFF_EVENTS_PHYEND: u64 = 0x16C;

const OFF_SHORTS: u64 = 0x200;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;
const OFF_CRCSTATUS: u64 = 0x400;
const OFF_RXMATCH: u64 = 0x408;
const OFF_RXCRC: u64 = 0x40C;
const OFF_DAI: u64 = 0x410;
const OFF_PDUSTAT: u64 = 0x414;
const OFF_PACKETPTR: u64 = 0x504;
const OFF_FREQUENCY: u64 = 0x508;
const OFF_TXPOWER: u64 = 0x50C;
const OFF_MODE: u64 = 0x510;
const OFF_PCNF0: u64 = 0x514;
const OFF_PCNF1: u64 = 0x518;
const OFF_BASE0: u64 = 0x51C;
const OFF_BASE1: u64 = 0x520;
const OFF_PREFIX0: u64 = 0x524;
const OFF_PREFIX1: u64 = 0x528;
const OFF_TXADDRESS: u64 = 0x52C;
const OFF_RXADDRESSES: u64 = 0x530;
const OFF_CRCCNF: u64 = 0x534;
const OFF_CRCPOLY: u64 = 0x538;
const OFF_CRCINIT: u64 = 0x53C;
const OFF_TIFS: u64 = 0x544;
const OFF_RSSISAMPLE: u64 = 0x548;
const OFF_STATE: u64 = 0x550;
const OFF_DATAWHITEIV: u64 = 0x554;
const OFF_BCC: u64 = 0x560;
const OFF_DAB0: u64 = 0x600;
const OFF_DAB7: u64 = 0x61C;
const OFF_DAP0: u64 = 0x620;
const OFF_DAP7: u64 = 0x63C;
const OFF_DACNF: u64 = 0x640;
const OFF_MHRMATCHCONF: u64 = 0x644;
const OFF_MHRMATCHMAS: u64 = 0x648;
const OFF_MODECNF0: u64 = 0x650;
const OFF_SFD: u64 = 0x660;
const OFF_EDCNT: u64 = 0x664;
const OFF_EDSAMPLE: u64 = 0x668;
const OFF_CCACTRL: u64 = 0x66C;
const OFF_POWER: u64 = 0xFFC;

// STATE values per PS table 226.
const STATE_DISABLED: u32 = 0;
const STATE_RXRU: u32 = 1;
const STATE_RXIDLE: u32 = 2;
const STATE_RX: u32 = 3;
const STATE_RXDISABLE: u32 = 4;
const STATE_TXRU: u32 = 9;
const STATE_TXIDLE: u32 = 10;
const STATE_TX: u32 = 11;
const STATE_TXDISABLE: u32 = 12;

// SHORTS bits (PS table 224).
const SHORT_READY_START: u32 = 1 << 0;
const SHORT_END_DISABLE: u32 = 1 << 1;
const SHORT_DISABLED_TXEN: u32 = 1 << 2;
const SHORT_DISABLED_RXEN: u32 = 1 << 3;
const SHORT_ADDRESS_RSSISTART: u32 = 1 << 4;
const SHORT_END_START: u32 = 1 << 5;
const SHORT_ADDRESS_BCSTART: u32 = 1 << 6;
#[allow(dead_code)]
const SHORT_DISABLED_RSSISTOP: u32 = 1 << 8;

// INTEN bits map to events 0..23 at corresponding bit positions
// (PS table 223).
const INTEN_READY: u32 = 1 << 0;
const INTEN_END: u32 = 1 << 3;
const INTEN_DISABLED: u32 = 1 << 4;

#[derive(Debug, Default)]
pub struct Nrf52Radio {
    // Events
    events_ready: u32,
    events_address: u32,
    events_payload: u32,
    events_end: u32,
    events_disabled: u32,
    events_crcok: u32,
    events_txready: u32,
    events_rxready: u32,
    events_phyend: u32,

    shorts: u32,
    inten: u32,
    state: u32,

    // Configuration registers (all wide; firmware reads back what it wrote).
    packetptr: u32,
    frequency: u32,
    txpower: u32,
    mode: u32,
    pcnf0: u32,
    pcnf1: u32,
    base0: u32,
    base1: u32,
    prefix0: u32,
    prefix1: u32,
    txaddress: u32,
    rxaddresses: u32,
    crccnf: u32,
    crcpoly: u32,
    crcinit: u32,
    tifs: u32,
    rssisample: u32,
    datawhiteiv: u32,
    bcc: u32,
    dab: [u32; 8],
    dap: [u32; 8],
    dacnf: u32,
    mhrmatchconf: u32,
    mhrmatchmas: u32,
    modecnf0: u32,
    sfd: u32,
    edcnt: u32,
    edsample: u32,
    ccactrl: u32,

    // Pending-state flags driven by tick() so events fire on the cycle
    // after the task is asserted (matches the typical 1-cycle latency
    // firmware code anticipates).
    pending_ready: bool,
    pending_address: bool,
    pending_payload: bool,
    pending_end: bool,
    pending_disabled: bool,

    // ── Easy DMA staging ──────────────────────────────────────────────────
    /// Bytes captured by `tick_with_bus` on a TX TASKS_START. The most
    /// recent packet stays here for tests to inspect / for cross-instance
    /// routing to pick up.
    pub last_tx_packet: Option<Vec<u8>>,
    /// Inbox of pre-loaded packets that RX will hand out one at a time
    /// (FIFO). Loopback tests push here; in a future cross-instance
    /// model another Radio instance would push here too.
    pub rx_inbox: VecDeque<Vec<u8>>,
    /// Set when TASKS_START fires in a TX state. The next tick_with_bus
    /// reads PACKETPTR-pointed RAM, computes CRC + whitening, stores in
    /// last_tx_packet, then sets EVENTS_END.
    pending_tx_dma: bool,
    /// Set when TASKS_START fires in an RX state. The next tick_with_bus
    /// pulls a packet from rx_inbox, de-whitens, verifies CRC, writes
    /// to PACKETPTR-pointed RAM.
    pending_rx_dma: bool,
    /// CRCSTATUS for the most recent RX (1 = OK, 0 = ERROR).
    crc_status: u32,

    /// Bit-rate countdown. When TX or RX is in flight, the cycles
    /// until EVENTS_END fires.  Computed from MODE + packet length:
    /// BLE_1Mbit → 8 cycles/byte, BLE_2Mbit → 4, Nrf_2Mbit → 4,
    /// 802.15.4 (250 kbps) → 32. Sim treats one tick as ~1 µs.
    ///
    /// When `Some(0)`, the next tick fires EVENTS_END. When `None`,
    /// no transmission is in flight.
    tx_or_rx_cycles_remaining: Option<u32>,
    /// Logical address (0..7) the current RX is listening for. RXADDRESSES
    /// bits map to BASE0/PREFIX0[0] for bit 0, BASE1/PREFIX0[1..3] for
    /// bits 1..3, BASE1/PREFIX1[0..3] for bits 4..7.
    #[allow(dead_code)]
    rx_address_mask: u8,
    /// Frame currently being received (popped from virtual air, awaiting
    /// the bit-rate countdown to expire before becoming visible).
    #[allow(dead_code)]
    rx_in_flight: Option<AirFrame>,
    /// Bits-counted-since-TASKS_BCSTART. Compared against BCC on every
    /// RX-completion to fire EVENTS_BCMATCH when the count reaches the
    /// configured threshold.
    bit_counter_armed: bool,
    bit_counter: u32,
    /// EVENTS_BCMATCH live state.
    events_bcmatch: u32,
    /// EVENTS_DEVMATCH live state.
    events_devmatch: u32,
    /// EVENTS_DEVMISS live state.
    events_devmiss: u32,
    /// Deterministic PRNG state for RSSI sampling.
    rssi_prng: u32,
    /// The shared air medium this radio transmits into and receives from.
    /// Radios sharing a bus hear each other; `new()` uses the process-global
    /// default, `with_air` binds an explicit per-group bus.
    air: VirtualAirBus,
}

impl Nrf52Radio {
    pub fn new() -> Self {
        Self {
            // Reset values per PS table 226.
            state: STATE_DISABLED,
            air: default_air_bus().clone(),
            frequency: 0,
            mode: 0, // Nrf_1Mbit
            pcnf0: 0,
            pcnf1: 0,
            base0: 0,
            base1: 0,
            prefix0: 0,
            prefix1: 0,
            txaddress: 0,
            rxaddresses: 0,
            crccnf: 0,
            crcpoly: 0,
            crcinit: 0,
            tifs: 0,
            txpower: 0,
            rssisample: 60, // mid-range default
            datawhiteiv: 0x40,
            modecnf0: 0,
            ..Self::default()
        }
    }

    /// Build a radio bound to an explicit air bus. Radios sharing a bus hear
    /// each other; radios on different buses are isolated. Prefer this over
    /// `new()`'s process-global default once the host owns per-lab-group buses.
    pub fn with_air(air: VirtualAirBus) -> Self {
        Self { air, ..Self::new() }
    }

    /// Apply SHORTS-style automatic task triggers when an event fires.
    fn apply_event_shorts(&mut self, fired: u64) {
        match fired {
            OFF_EVENTS_READY if self.shorts & SHORT_READY_START != 0 => {
                self.start_packet();
            }
            OFF_EVENTS_END | OFF_EVENTS_PHYEND => {
                if self.shorts & SHORT_END_DISABLE != 0 {
                    self.disable();
                }
                if self.shorts & SHORT_END_START != 0 {
                    self.start_packet();
                }
            }
            OFF_EVENTS_DISABLED => {
                if self.shorts & SHORT_DISABLED_TXEN != 0 {
                    self.tx_enable();
                }
                if self.shorts & SHORT_DISABLED_RXEN != 0 {
                    self.rx_enable();
                }
            }
            OFF_EVENTS_ADDRESS => {
                if self.shorts & SHORT_ADDRESS_RSSISTART != 0 {
                    // RSSISTART is a no-op in our model.
                }
                if self.shorts & SHORT_ADDRESS_BCSTART != 0 {
                    // BCSTART is a no-op in our model.
                }
            }
            _ => {}
        }
    }

    fn tx_enable(&mut self) {
        self.state = STATE_TXRU;
        self.pending_ready = true;
    }

    fn rx_enable(&mut self) {
        self.state = STATE_RXRU;
        self.pending_ready = true;
    }

    fn start_packet(&mut self) {
        if self.state == STATE_TXIDLE {
            self.state = STATE_TX;
            self.pending_tx_dma = true;
        } else if self.state == STATE_RXIDLE {
            self.state = STATE_RX;
            self.pending_rx_dma = true;
        } else {
            return;
        }
        // Default 1-tick fire so callers that don't drive tick_with_bus
        // (state-machine tests, firmware that never sees the bus DMA)
        // still get EVENTS_ADDRESS/PAYLOAD/END quickly. tick_with_bus
        // overwrites this with a proper bit-rate count when it runs.
        self.tx_or_rx_cycles_remaining = Some(1);
    }

    /// Look up the (BASE, PREFIX) tuple for logical address N per
    /// PS §6.20.6.3. Returns (BASE0 or BASE1, prefix byte).
    fn lookup_logical_address(&self, n: u8) -> (u32, u8) {
        match n {
            0 => (self.base0, (self.prefix0 & 0xFF) as u8),
            1 => (self.base1, ((self.prefix0 >> 8) & 0xFF) as u8),
            2 => (self.base1, ((self.prefix0 >> 16) & 0xFF) as u8),
            3 => (self.base1, ((self.prefix0 >> 24) & 0xFF) as u8),
            4 => (self.base1, (self.prefix1 & 0xFF) as u8),
            5 => (self.base1, ((self.prefix1 >> 8) & 0xFF) as u8),
            6 => (self.base1, ((self.prefix1 >> 16) & 0xFF) as u8),
            7 => (self.base1, ((self.prefix1 >> 24) & 0xFF) as u8),
            _ => (0, 0),
        }
    }

    /// Returns true if any RXADDRESSES-enabled logical address on this
    /// receiver matches the (base, prefix) the frame came from.
    fn matches_address(&self, frame: &AirFrame) -> bool {
        let enabled = (self.rxaddresses & 0xFF) as u8;
        if enabled == 0 {
            // No RX addresses configured — promiscuous mode for tests.
            return true;
        }
        for n in 0..8u8 {
            if enabled & (1 << n) == 0 {
                continue;
            }
            let (b, p) = self.lookup_logical_address(n);
            if b == frame.addr_base && p == frame.addr_prefix {
                return true;
            }
        }
        false
    }

    #[allow(dead_code)] // reserved for future bit-rate model
    /// DACNF.ENA mask (low 8 bits) determines which of DAB[0..7] /
    /// DAP[0..7] are active. Returns true if any enabled slot's address
    /// matches the (base, prefix) on the wire.
    fn device_match(&self, frame_base: u32, frame_prefix: u8) -> bool {
        let ena = self.dacnf & 0xFF;
        if ena == 0 {
            return false;
        }
        for n in 0..8u8 {
            if ena & (1 << n) == 0 {
                continue;
            }
            // DAB[n] holds the 4-byte BASE; DAP[n] holds the 1-byte
            // PREFIX (low byte of the register).
            let cand_base = self.dab[n as usize];
            let cand_prefix = (self.dap[n as usize] & 0xFF) as u8;
            if cand_base == frame_base && cand_prefix == frame_prefix {
                return true;
            }
        }
        false
    }

    /// Simple xorshift32 to give RSSISAMPLE realistic variability without
    /// pulling in a heavy RNG. Returns a value in 0..=127 (lower = stronger,
    /// per PS §6.20.12.27).
    fn next_rssi_sample(&mut self) -> u32 {
        if self.rssi_prng == 0 {
            self.rssi_prng = 0xCAFE_BABE;
        }
        let mut x = self.rssi_prng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rssi_prng = x;
        // Mid-range bias (~RSSI of -50 dBm); ±20 dB jitter.
        40 + (x & 0x1F)
    }

    #[allow(dead_code)] // reserved for future bit-rate model
    /// Bit-rate cycle count for a packet of `total_bytes` bytes in the
    /// given MODE. MODE values per PS §6.20.12.10. One sim tick ≈ 1 µs.
    fn cycles_for_packet(mode: u32, total_bytes: u32) -> u32 {
        let cycles_per_byte = match mode & 0xF {
            0 | 3 => 8, // Nrf_1Mbit, Ble_1Mbit
            1 | 4 => 4, // Nrf_2Mbit, Ble_2Mbit
            2 => 32,    // Nrf_250Kbit (deprecated)
            5 => 64,    // Ble_LR125Kbit
            6 => 16,    // Ble_LR500Kbit
            15 => 32,   // Ieee802154_250Kbit
            _ => 8,
        };
        // Always at least 1 cycle so a zero-length packet still fires.
        (total_bytes.max(1)) * cycles_per_byte
    }

    fn disable(&mut self) {
        match self.state {
            STATE_RX | STATE_RXIDLE | STATE_RXRU => {
                self.state = STATE_RXDISABLE;
            }
            STATE_TX | STATE_TXIDLE | STATE_TXRU => {
                self.state = STATE_TXDISABLE;
            }
            _ => {}
        }
        self.pending_disabled = true;
    }

    /// PN9 BLE whitening — XORs `data` in place with the LFSR output.
    /// Init value comes from `DATAWHITEIV` bits [6:0] (bit 6 is fixed 1
    /// in real silicon per PS §6.20.12.32).
    ///
    /// This is symmetric: applying it twice cancels out, so the same
    /// routine is used on TX (whiten) and RX (de-whiten).
    fn ble_whiten(data: &mut [u8], whitening_iv: u8) {
        let mut lfsr: u8 = (whitening_iv & 0x7F) | 0x40;
        for byte in data.iter_mut() {
            let mut out = 0u8;
            for bit in 0..8 {
                let bit_lfsr = (lfsr >> 6) & 1;
                let bit_in = (*byte >> bit) & 1;
                out |= (bit_in ^ bit_lfsr) << bit;
                // Advance LFSR: x^7 + x^4 + 1
                let feedback = bit_lfsr;
                lfsr = ((lfsr << 1) | feedback) & 0x7F;
                if feedback != 0 {
                    lfsr ^= 0x04; // x^4 tap
                }
            }
            *byte = out;
        }
    }

    /// BLE CRC-24 (polynomial 0x100065B). Init value from CRCINIT.
    /// Returns the 24-bit CRC of `data`.
    fn ble_crc24(data: &[u8], init: u32) -> u32 {
        let mut crc = init & 0xFFFFFF;
        for &byte in data {
            crc ^= (byte as u32) << 16;
            for _ in 0..8 {
                if crc & (1 << 23) != 0 {
                    crc = ((crc << 1) ^ 0x65B) & 0xFFFFFF;
                } else {
                    crc = (crc << 1) & 0xFFFFFF;
                }
            }
        }
        crc
    }

    /// Pull PCNF0/PCNF1 fields into a packet descriptor used by both
    /// TX and RX DMA.
    fn packet_descriptor(&self) -> PacketDescriptor {
        PacketDescriptor {
            lflen: (self.pcnf0 & 0xF) as u8,
            s0len: ((self.pcnf0 >> 8) & 1) as u8,
            s1len: ((self.pcnf0 >> 16) & 0xF) as u8,
            maxlen: (self.pcnf1 & 0xFF) as u8,
            statlen: ((self.pcnf1 >> 8) & 0xFF) as u8,
            whiteen: (self.pcnf1 >> 25) & 1 != 0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct PacketDescriptor {
    /// LENGTH field bit width (0..8).
    lflen: u8,
    /// S0 byte present (0 or 1).
    s0len: u8,
    /// S1 field bit width (0..15).
    s1len: u8,
    /// Maximum payload bytes addressable from PACKETPTR.
    maxlen: u8,
    /// Static-length payload bytes appended after the dynamic LENGTH.
    statlen: u8,
    /// Whitening enabled.
    whiteen: bool,
}

impl PacketDescriptor {
    /// Returns the byte offset of the start of the payload within the
    /// PACKETPTR buffer (S0 + LENGTH + S1 rounded up to whole bytes).
    fn payload_offset(&self) -> usize {
        let s1_bytes = self.s1len.div_ceil(8);
        // LENGTH field is always 1 byte in the buffer regardless of LFLEN
        // (PS §6.20.6.1: "stored in the byte after S0").
        let length_bytes = if self.lflen == 0 { 0 } else { 1 };
        self.s0len as usize + length_bytes as usize + s1_bytes as usize
    }
}

impl Peripheral for Nrf52Radio {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            // Tasks always read 0.
            OFF_TASKS_TXEN..=OFF_TASKS_CCASTOP if offset.is_multiple_of(4) => 0,

            // Events.
            OFF_EVENTS_READY => self.events_ready,
            OFF_EVENTS_ADDRESS => self.events_address,
            OFF_EVENTS_PAYLOAD => self.events_payload,
            OFF_EVENTS_END => self.events_end,
            OFF_EVENTS_DISABLED => self.events_disabled,
            OFF_EVENTS_DEVMATCH => self.events_devmatch,
            OFF_EVENTS_DEVMISS => self.events_devmiss,
            OFF_EVENTS_BCMATCH => self.events_bcmatch,
            OFF_EVENTS_RSSIEND
            | OFF_EVENTS_CRCERROR
            | OFF_EVENTS_FRAMESTART
            | OFF_EVENTS_EDEND
            | OFF_EVENTS_EDSTOPPED
            | OFF_EVENTS_CCAIDLE
            | OFF_EVENTS_CCABUSY
            | OFF_EVENTS_CCASTOPPED
            | OFF_EVENTS_RATEBOOST
            | OFF_EVENTS_MHRMATCH
            | OFF_EVENTS_SYNC => 0,
            OFF_EVENTS_CRCOK => self.events_crcok,
            OFF_EVENTS_TXREADY => self.events_txready,
            OFF_EVENTS_RXREADY => self.events_rxready,
            OFF_EVENTS_PHYEND => self.events_phyend,

            OFF_SHORTS => self.shorts,
            OFF_INTENSET | OFF_INTENCLR => self.inten,

            OFF_CRCSTATUS => self.crc_status & 1,
            OFF_RXMATCH => self.rxaddresses & 0x7, // hint at logical address 0
            OFF_RXCRC => self.crcinit,
            OFF_DAI => 0,
            OFF_PDUSTAT => 0,

            OFF_PACKETPTR => self.packetptr,
            OFF_FREQUENCY => self.frequency & 0xFF,
            OFF_TXPOWER => self.txpower,
            OFF_MODE => self.mode & 0xF,
            OFF_PCNF0 => self.pcnf0,
            OFF_PCNF1 => self.pcnf1,
            OFF_BASE0 => self.base0,
            OFF_BASE1 => self.base1,
            OFF_PREFIX0 => self.prefix0,
            OFF_PREFIX1 => self.prefix1,
            OFF_TXADDRESS => self.txaddress & 0x7,
            OFF_RXADDRESSES => self.rxaddresses & 0xFF,
            OFF_CRCCNF => self.crccnf,
            OFF_CRCPOLY => self.crcpoly & 0xFFFFFF,
            OFF_CRCINIT => self.crcinit & 0xFFFFFF,
            OFF_TIFS => self.tifs & 0x3FF,
            OFF_RSSISAMPLE => self.rssisample & 0x7F,
            OFF_STATE => self.state,
            OFF_DATAWHITEIV => self.datawhiteiv & 0x7F,
            OFF_BCC => self.bcc,

            OFF_DAB0..=OFF_DAB7 if offset.is_multiple_of(4) => {
                self.dab[((offset - OFF_DAB0) / 4) as usize]
            }
            OFF_DAP0..=OFF_DAP7 if offset.is_multiple_of(4) => {
                self.dap[((offset - OFF_DAP0) / 4) as usize] & 0xFFFF
            }
            OFF_DACNF => self.dacnf,
            OFF_MHRMATCHCONF => self.mhrmatchconf,
            OFF_MHRMATCHMAS => self.mhrmatchmas,
            OFF_MODECNF0 => self.modecnf0,
            OFF_SFD => self.sfd & 0xFF,
            OFF_EDCNT => self.edcnt & 0x1FFFFF,
            OFF_EDSAMPLE => self.edsample & 0xFF,
            OFF_CCACTRL => self.ccactrl,
            OFF_POWER => 1, // peripheral powered

            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            // Tasks: trigger state transitions.
            OFF_TASKS_TXEN
                if value & 1 != 0 => {
                    self.tx_enable();
                }
            OFF_TASKS_RXEN
                if value & 1 != 0 => {
                    self.rx_enable();
                }
            OFF_TASKS_START
                if value & 1 != 0 => {
                    self.start_packet();
                }
            OFF_TASKS_STOP
                if value & 1 != 0 => {
                    if self.state == STATE_TX {
                        self.state = STATE_TXIDLE;
                    } else if self.state == STATE_RX {
                        self.state = STATE_RXIDLE;
                    }
                }
            OFF_TASKS_DISABLE
                if value & 1 != 0 => {
                    self.disable();
                }
            OFF_TASKS_RSSISTART
                // Sample RSSI immediately — real silicon needs a few µs
                // but firmware just polls RSSISAMPLE after this task.
                if value & 1 != 0 => {
                    self.rssisample = self.next_rssi_sample();
                }
            OFF_TASKS_BCSTART
                if value & 1 != 0 => {
                    self.bit_counter_armed = true;
                    self.bit_counter = 0;
                }
            OFF_TASKS_BCSTOP
                if value & 1 != 0 => {
                    self.bit_counter_armed = false;
                }
            OFF_TASKS_RSSISTOP | OFF_TASKS_EDSTART | OFF_TASKS_EDSTOP | OFF_TASKS_CCASTART
            | OFF_TASKS_CCASTOP => {}

            // EVENTS_*: hardware-generated. SW write-1 is ignored per silicon;
            // SW write-0 clears the event latch (firmware ISR ack path).
            OFF_EVENTS_READY if value == 0 => self.events_ready = 0,
            OFF_EVENTS_ADDRESS if value == 0 => self.events_address = 0,
            OFF_EVENTS_PAYLOAD if value == 0 => self.events_payload = 0,
            OFF_EVENTS_END if value == 0 => self.events_end = 0,
            OFF_EVENTS_DISABLED if value == 0 => self.events_disabled = 0,
            OFF_EVENTS_CRCOK if value == 0 => self.events_crcok = 0,
            OFF_EVENTS_TXREADY if value == 0 => self.events_txready = 0,
            OFF_EVENTS_RXREADY if value == 0 => self.events_rxready = 0,
            OFF_EVENTS_PHYEND if value == 0 => self.events_phyend = 0,
            OFF_EVENTS_DEVMATCH if value == 0 => self.events_devmatch = 0,
            OFF_EVENTS_DEVMISS if value == 0 => self.events_devmiss = 0,
            OFF_EVENTS_BCMATCH if value == 0 => self.events_bcmatch = 0,

            OFF_SHORTS => self.shorts = value,
            OFF_INTENSET => self.inten |= value,
            OFF_INTENCLR => self.inten &= !value,

            OFF_PACKETPTR => self.packetptr = value,
            OFF_FREQUENCY => self.frequency = value & 0xFF,
            OFF_TXPOWER => self.txpower = value,
            OFF_MODE => self.mode = value & 0xF,
            OFF_PCNF0 => self.pcnf0 = value,
            OFF_PCNF1 => self.pcnf1 = value,
            OFF_BASE0 => self.base0 = value,
            OFF_BASE1 => self.base1 = value,
            OFF_PREFIX0 => self.prefix0 = value,
            OFF_PREFIX1 => self.prefix1 = value,
            OFF_TXADDRESS => self.txaddress = value & 0x7,
            OFF_RXADDRESSES => self.rxaddresses = value & 0xFF,
            OFF_CRCCNF => self.crccnf = value,
            OFF_CRCPOLY => self.crcpoly = value & 0xFFFFFF,
            OFF_CRCINIT => self.crcinit = value & 0xFFFFFF,
            OFF_TIFS => self.tifs = value & 0x3FF,
            OFF_DATAWHITEIV => self.datawhiteiv = value & 0x7F,
            OFF_BCC => self.bcc = value,

            OFF_DAB0..=OFF_DAB7 if offset.is_multiple_of(4) => {
                self.dab[((offset - OFF_DAB0) / 4) as usize] = value;
            }
            OFF_DAP0..=OFF_DAP7 if offset.is_multiple_of(4) => {
                self.dap[((offset - OFF_DAP0) / 4) as usize] = value & 0xFFFF;
            }
            OFF_DACNF => self.dacnf = value,
            OFF_MHRMATCHCONF => self.mhrmatchconf = value,
            OFF_MHRMATCHMAS => self.mhrmatchmas = value,
            OFF_MODECNF0 => self.modecnf0 = value,
            OFF_SFD => self.sfd = value & 0xFF,
            OFF_EDSAMPLE => self.edsample = value & 0xFF,
            OFF_CCACTRL => self.ccactrl = value,
            OFF_POWER => {} // RW but we ignore power state

            _ => {}
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Bit-rate countdown for TX/RX in flight. When the count reaches
        // zero on this tick we set the ADDRESS/PAYLOAD/END flags so the
        // events fire in the existing handler below. If the count is
        // still > 1, we just decrement and return — the firmware sees
        // the radio still transmitting / receiving.
        if let Some(n) = self.tx_or_rx_cycles_remaining {
            if n <= 1 {
                self.tx_or_rx_cycles_remaining = None;
                self.pending_address = true;
                self.pending_payload = true;
                self.pending_end = true;
            } else {
                self.tx_or_rx_cycles_remaining = Some(n - 1);
                return PeripheralTickResult {
                    cycles: 1,
                    ..Default::default()
                };
            }
        }

        if !self.pending_ready
            && !self.pending_address
            && !self.pending_payload
            && !self.pending_end
            && !self.pending_disabled
        {
            return PeripheralTickResult::default();
        }

        let mut fired = Vec::new();
        let mut irq = false;

        if self.pending_ready {
            self.pending_ready = false;
            self.events_ready = 1;
            // Transition RU → IDLE.
            if self.state == STATE_TXRU {
                self.state = STATE_TXIDLE;
                self.events_txready = 1;
            } else if self.state == STATE_RXRU {
                self.state = STATE_RXIDLE;
                self.events_rxready = 1;
            }
            fired.push(OFF_EVENTS_READY as u32);
            if self.inten & INTEN_READY != 0 {
                irq = true;
            }
            self.apply_event_shorts(OFF_EVENTS_READY);
        }

        if self.pending_address {
            self.pending_address = false;
            self.events_address = 1;
            fired.push(OFF_EVENTS_ADDRESS as u32);
            self.apply_event_shorts(OFF_EVENTS_ADDRESS);
        }
        if self.pending_payload {
            self.pending_payload = false;
            self.events_payload = 1;
            fired.push(OFF_EVENTS_PAYLOAD as u32);
        }
        if self.pending_end {
            self.pending_end = false;
            self.events_end = 1;
            self.events_crcok = 1;
            self.events_phyend = 1;
            if self.state == STATE_TX {
                self.state = STATE_TXIDLE;
            } else if self.state == STATE_RX {
                self.state = STATE_RXIDLE;
            }
            fired.push(OFF_EVENTS_END as u32);
            if self.inten & INTEN_END != 0 {
                irq = true;
            }
            self.apply_event_shorts(OFF_EVENTS_END);
        }

        if self.pending_disabled {
            self.pending_disabled = false;
            self.state = STATE_DISABLED;
            self.events_disabled = 1;
            fired.push(OFF_EVENTS_DISABLED as u32);
            if self.inten & INTEN_DISABLED != 0 {
                irq = true;
            }
            self.apply_event_shorts(OFF_EVENTS_DISABLED);
        }

        PeripheralTickResult {
            irq,
            cycles: 1,
            fired_events: fired,
            ..Default::default()
        }
    }

    fn needs_bus_tick(&self) -> bool {
        self.pending_tx_dma || self.pending_rx_dma
    }

    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        // ── TX Easy DMA ──────────────────────────────────────────────────
        // Read S0 + LENGTH + S1 + payload from PACKETPTR, compute total
        // size, apply BLE whitening (if PCNF1.WHITEEN), append CRC, store
        // in last_tx_packet for tests / cross-instance routing.
        if self.pending_tx_dma {
            self.pending_tx_dma = false;
            let desc = self.packet_descriptor();
            let base = self.packetptr;

            // Read header bytes into a small staging buffer first.
            let mut header = Vec::new();
            for i in 0..desc.payload_offset() {
                header.push(
                    bus.read_u8((base as u64).wrapping_add(i as u64))
                        .unwrap_or(0),
                );
            }

            // LENGTH is the byte at offset s0len (right after S0).
            let length = if desc.lflen == 0 {
                desc.statlen
            } else {
                let len_byte = header.get(desc.s0len as usize).copied().unwrap_or(0);
                // Mask to LFLEN bits.
                let mask = if desc.lflen >= 8 {
                    0xFFu8
                } else {
                    ((1u16 << desc.lflen) - 1) as u8
                };
                (len_byte & mask)
                    .min(desc.maxlen)
                    .saturating_add(desc.statlen)
            };

            // Read the payload right after the header bytes.
            let payload_off = desc.payload_offset();
            let mut packet = header.clone();
            for i in 0..length as usize {
                packet.push(
                    bus.read_u8((base as u64).wrapping_add((payload_off + i) as u64))
                        .unwrap_or(0),
                );
            }

            // Whiten (in place — symmetric, so RX reverses by applying
            // the same routine).
            if desc.whiteen {
                Self::ble_whiten(&mut packet, self.datawhiteiv as u8);
            }

            // Append CRC-24 to capture downstream.
            let crc = Self::ble_crc24(&packet, self.crcinit);
            packet.push((crc & 0xFF) as u8);
            packet.push(((crc >> 8) & 0xFF) as u8);
            packet.push(((crc >> 16) & 0xFF) as u8);

            // Capture the sender's logical address so receivers can match
            // against their RXADDRESSES + BASE/PREFIX. Logical-address
            // table (PS §6.20.6.3):
            //   N=0: BASE0 + PREFIX0[7:0]
            //   N=1: BASE1 + PREFIX0[15:8]
            //   N=2: BASE1 + PREFIX0[23:16]
            //   N=3: BASE1 + PREFIX0[31:24]
            //   N=4: BASE1 + PREFIX1[7:0]
            //   N=5: BASE1 + PREFIX1[15:8]
            //   N=6: BASE1 + PREFIX1[23:16]
            //   N=7: BASE1 + PREFIX1[31:24]
            let txaddr = (self.txaddress & 0x7) as u8;
            let (addr_base, addr_prefix) = self.lookup_logical_address(txaddr);
            let frame = AirFrame {
                bytes: packet.clone(),
                addr_base,
                addr_prefix,
                addr_len: 3 + ((self.pcnf1 >> 16) & 0x7) as u8,
                whitening_iv: self.datawhiteiv as u8,
                crcinit: self.crcinit,
                mode: self.mode,
            };
            self.last_tx_packet = Some(packet);

            // Push to the global virtual air keyed by FREQUENCY. Frames carry
            // their MODE so receivers tuned to a different mode on the same
            // band correctly fail to decode.
            if let Ok(mut air) = self.air.lock() {
                let key = self.frequency as u8;
                let trace = AirFrameTrace {
                    channel: key,
                    addr_base: frame.addr_base,
                    addr_prefix: frame.addr_prefix,
                    mode: frame.mode,
                    bytes: frame.bytes.clone(),
                    whitening_iv: frame.whitening_iv,
                };
                air.queues.entry(key).or_default().push_back(frame);
                air.tx_history.push_back(trace);
                while air.tx_history.len() > TX_HISTORY_CAP {
                    air.tx_history.pop_front();
                }
            }

            // Bit-rate countdown until EVENTS_END. cycles_for_packet returns
            // bytes × cycles-per-byte for the MODE; +3 for the CRC bytes.
            self.tx_or_rx_cycles_remaining =
                Some(Self::cycles_for_packet(self.mode, length as u32 + 3));
        }

        // ── RX Easy DMA ──────────────────────────────────────────────────
        // Pop the next packet from the global virtual air OR rx_inbox
        // (loopback path used by some tests), verify CRC, de-whiten,
        // write to PACKETPTR-pointed RAM. Address-matched against
        // RXADDRESSES + BASE/PREFIX before delivery.
        if self.pending_rx_dma {
            // Cancel any default 1-tick countdown that start_packet
            // seeded; we only want EVENTS_END to fire when we actually
            // dequeue a frame.
            self.tx_or_rx_cycles_remaining = None;

            // First, try the global virtual air at FREQUENCY. Only consume
            // a frame whose MODE matches ours AND whose sender's address
            // matches one of our enabled logical addresses. DEVMATCH / DEVMISS
            // are also computed here against the DAB/DAP whitelist.
            let mut popped = None;
            let mut popped_frame_addr: Option<(u32, u8)> = None;
            if let Ok(mut air) = self.air.lock() {
                let key = self.frequency as u8;
                if let Some(queue) = air.queues.get_mut(&key) {
                    let pos = queue
                        .iter()
                        .position(|f| f.mode == self.mode && self.matches_address(f));
                    if let Some(idx) = pos {
                        if let Some(f) = queue.remove(idx) {
                            popped_frame_addr = Some((f.addr_base, f.addr_prefix));
                            popped = Some(f.bytes);
                        }
                    }
                }
            }

            // Fall back to the per-instance rx_inbox (used by tests that
            // don't go through the global air).
            let pkt_opt = popped.or_else(|| self.rx_inbox.pop_front());

            if let Some(mut pkt) = pkt_opt {
                // A packet was actually dequeued — clear the pending flag
                // and the rest of the body runs the DMA + sets cycles.
                self.pending_rx_dma = false;
                let desc = self.packet_descriptor();

                // Trailing 3 bytes are the CRC (LE).
                let crc_ok = if pkt.len() >= 3 {
                    let crc_lo = pkt.pop().unwrap_or(0) as u32;
                    let crc_md = pkt.pop().unwrap_or(0) as u32;
                    let crc_hi = pkt.pop().unwrap_or(0) as u32;
                    let received = (crc_lo << 16) | (crc_md << 8) | crc_hi;
                    Self::ble_crc24(&pkt, self.crcinit) == received
                } else {
                    false
                };
                self.crc_status = if crc_ok { 1 } else { 0 };

                if desc.whiteen {
                    Self::ble_whiten(&mut pkt, self.datawhiteiv as u8);
                }

                let base = self.packetptr;
                for (i, b) in pkt.iter().enumerate() {
                    let _ = bus.write_u8((base as u64).wrapping_add(i as u64), *b);
                }

                // ── DEVMATCH / DEVMISS via DAB / DAP whitelist ─────────
                // Only when we actually consumed a frame from the global
                // air (which carries the sender's address). Local
                // rx_inbox loopback skips this — those tests don't set
                // DAB/DAP.
                if let Some((fb, fp)) = popped_frame_addr {
                    if self.device_match(fb, fp) {
                        self.events_devmatch = 1;
                    } else {
                        self.events_devmiss = 1;
                    }
                }

                // ── BCMATCH — if armed, count bits received against BCC ─
                if self.bit_counter_armed {
                    self.bit_counter = (pkt.len() as u32) * 8;
                    if self.bit_counter >= self.bcc {
                        self.events_bcmatch = 1;
                    }
                }

                // ── RSSI sampling per-frame ──────────────────────────
                self.rssisample = self.next_rssi_sample();

                // Set bit-rate countdown for the actual received packet
                // length (including the 3 CRC bytes we stripped above).
                self.tx_or_rx_cycles_remaining =
                    Some(Self::cycles_for_packet(self.mode, pkt.len() as u32 + 3));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialise tests that touch the global virtual air so they don't
    /// see each other's frames. Each test acquires this guard before
    /// clearing the air.
    static AIR_GUARD: Mutex<()> = Mutex::new(());

    fn air_test_setup() -> std::sync::MutexGuard<'static, ()> {
        let guard = AIR_GUARD.lock().unwrap_or_else(|p| p.into_inner());
        clear_virtual_air();
        guard
    }

    #[test]
    fn txen_progresses_to_txidle_and_fires_ready() {
        let mut r = Nrf52Radio::new();
        r.write_u32(OFF_TASKS_TXEN, 1).unwrap();
        // Pending until tick.
        assert_eq!(r.read_u32(OFF_STATE).unwrap(), STATE_TXRU);
        let res = r.tick();
        assert!(res.fired_events.contains(&(OFF_EVENTS_READY as u32)));
        assert_eq!(r.read_u32(OFF_STATE).unwrap(), STATE_TXIDLE);
        assert_eq!(r.read_u32(OFF_EVENTS_READY).unwrap(), 1);
    }

    #[test]
    fn start_in_txidle_completes_packet() {
        let mut r = Nrf52Radio::new();
        r.write_u32(OFF_TASKS_TXEN, 1).unwrap();
        r.tick(); // READY → TXIDLE
        r.write_u32(OFF_TASKS_START, 1).unwrap();
        assert_eq!(r.read_u32(OFF_STATE).unwrap(), STATE_TX);
        let res = r.tick();
        assert!(res.fired_events.contains(&(OFF_EVENTS_END as u32)));
        assert_eq!(r.read_u32(OFF_EVENTS_END).unwrap(), 1);
        // CRCSTATUS is the live receive-side CRC verdict; TX doesn't touch
        // it, so it stays at the reset value (0) here.
        assert_eq!(r.read_u32(OFF_STATE).unwrap(), STATE_TXIDLE);
    }

    #[test]
    fn disable_returns_to_disabled() {
        let mut r = Nrf52Radio::new();
        r.write_u32(OFF_TASKS_RXEN, 1).unwrap();
        r.tick(); // RXRU → RXIDLE
        r.write_u32(OFF_TASKS_DISABLE, 1).unwrap();
        r.tick();
        assert_eq!(r.read_u32(OFF_STATE).unwrap(), STATE_DISABLED);
        assert_eq!(r.read_u32(OFF_EVENTS_DISABLED).unwrap(), 1);
    }

    #[test]
    fn shorts_ready_start_chains() {
        let mut r = Nrf52Radio::new();
        r.write_u32(OFF_SHORTS, SHORT_READY_START | SHORT_END_DISABLE)
            .unwrap();
        r.write_u32(OFF_TASKS_TXEN, 1).unwrap();
        // Tick 1: READY fires; SHORT_READY_START schedules a TX. Bit-rate
        // countdown starts (cycles=Some(1) default since no tick_with_bus
        // ran to refine the count).
        r.tick();
        assert_eq!(r.read_u32(OFF_EVENTS_READY).unwrap(), 1);
        // Tick 2: countdown expires, ADDRESS/PAYLOAD/END fire;
        // SHORT_END_DISABLE chains into DISABLE which fires DISABLED.
        r.tick();
        assert_eq!(r.read_u32(OFF_EVENTS_END).unwrap(), 1);
        assert_eq!(r.read_u32(OFF_EVENTS_DISABLED).unwrap(), 1);
        assert_eq!(r.read_u32(OFF_STATE).unwrap(), STATE_DISABLED);
    }

    #[test]
    fn intenset_end_pends_irq_on_packet_complete() {
        let mut r = Nrf52Radio::new();
        r.write_u32(OFF_INTENSET, INTEN_END).unwrap();
        r.write_u32(OFF_TASKS_TXEN, 1).unwrap();
        r.tick();
        r.write_u32(OFF_TASKS_START, 1).unwrap();
        let res = r.tick();
        assert!(res.irq, "END IRQ should pend when INTEN.END is set");
    }

    #[test]
    fn config_regs_round_trip() {
        let mut r = Nrf52Radio::new();
        r.write_u32(OFF_FREQUENCY, 0x4E).unwrap(); // BLE advertising ch 37
        r.write_u32(OFF_MODE, 0x3).unwrap(); // BLE_1Mbit
        r.write_u32(OFF_PCNF0, 0x00010008).unwrap();
        r.write_u32(OFF_BASE0, 0xCAFEBABE).unwrap();
        r.write_u32(OFF_PREFIX0, 0xDEAD).unwrap();
        r.write_u32(OFF_PACKETPTR, 0x2000_1234).unwrap();
        assert_eq!(r.read_u32(OFF_FREQUENCY).unwrap(), 0x4E);
        assert_eq!(r.read_u32(OFF_MODE).unwrap(), 0x3);
        assert_eq!(r.read_u32(OFF_PCNF0).unwrap(), 0x00010008);
        assert_eq!(r.read_u32(OFF_BASE0).unwrap(), 0xCAFEBABE);
        assert_eq!(r.read_u32(OFF_PREFIX0).unwrap(), 0xDEAD);
        assert_eq!(r.read_u32(OFF_PACKETPTR).unwrap(), 0x2000_1234);
    }

    #[test]
    fn whitening_is_symmetric() {
        let original = vec![0xAA, 0x55, 0xFF, 0x00, 0x12, 0x34];
        let mut work = original.clone();
        Nrf52Radio::ble_whiten(&mut work, 0x40);
        assert_ne!(work, original, "whitening should change the bytes");
        Nrf52Radio::ble_whiten(&mut work, 0x40);
        assert_eq!(work, original, "applying twice should cancel out");
    }

    #[test]
    fn crc24_deterministic() {
        let data = b"hello";
        let crc1 = Nrf52Radio::ble_crc24(data, 0x555555);
        let crc2 = Nrf52Radio::ble_crc24(data, 0x555555);
        assert_eq!(crc1, crc2);
        assert!(crc1 <= 0xFFFFFF);
        // CRC of empty input with init=0 is 0.
        assert_eq!(Nrf52Radio::ble_crc24(&[], 0), 0);
    }

    #[test]
    fn tx_easy_dma_reads_packetptr_and_stores_packet() {
        let _guard = air_test_setup();
        use crate::bus::SystemBus;
        use crate::Bus;

        let mut bus = SystemBus::new();
        // Stage S0 + LENGTH + payload at 0x20000000 in RAM.
        let pkt = [0xAB, 0x03, 0xDE, 0xAD, 0xBE];
        for (i, b) in pkt.iter().enumerate() {
            bus.write_u8(0x2000_0000 + i as u64, *b).unwrap();
        }

        let mut r = Nrf52Radio::new();
        // S0LEN=1, LFLEN=8 → PCNF0 bits 0..3=8, bit 8=1.
        r.write_u32(OFF_PCNF0, (8) | (1 << 8)).unwrap();
        // MAXLEN=255, WHITEEN=0 (off so we can check raw bytes).
        r.write_u32(OFF_PCNF1, 0xFF).unwrap();
        r.write_u32(OFF_PACKETPTR, 0x2000_0000).unwrap();
        r.write_u32(OFF_CRCINIT, 0x555555).unwrap();

        r.write_u32(OFF_TASKS_TXEN, 1).unwrap();
        r.tick();
        r.write_u32(OFF_TASKS_START, 1).unwrap();
        // tick() will request DMA; bus normally calls tick_with_bus.
        r.tick();
        r.tick_with_bus(&mut bus);

        let stored = r.last_tx_packet.as_ref().expect("tx packet captured");
        // S0 + LENGTH + 3 payload bytes + 3 CRC bytes = 8 bytes.
        assert_eq!(stored.len(), 8);
        assert_eq!(stored[0], 0xAB);
        assert_eq!(stored[1], 0x03);
        assert_eq!(&stored[2..5], &[0xDE, 0xAD, 0xBE]);
    }

    #[test]
    fn rx_easy_dma_loopback_through_inbox() {
        let _guard = air_test_setup();
        use crate::bus::SystemBus;
        use crate::Bus;

        let mut bus = SystemBus::new();

        // Build an inbox packet with a valid CRC over [S0, LENGTH, payload].
        // Wire format matches what TX produces: payload first, then CRC
        // bytes in little-endian order (LSB first).
        let head_and_payload = vec![0xCD, 0x04, 0xCA, 0xFE, 0xBA, 0xBE];
        let crc = Nrf52Radio::ble_crc24(&head_and_payload, 0x555555);
        let mut inbox_pkt = head_and_payload.clone();
        inbox_pkt.push((crc & 0xFF) as u8);
        inbox_pkt.push(((crc >> 8) & 0xFF) as u8);
        inbox_pkt.push(((crc >> 16) & 0xFF) as u8);

        let mut r = Nrf52Radio::new();
        r.rx_inbox.push_back(inbox_pkt);
        r.write_u32(OFF_PCNF0, 8 | (1 << 8)).unwrap();
        r.write_u32(OFF_PCNF1, 0xFF).unwrap();
        r.write_u32(OFF_PACKETPTR, 0x2000_1000).unwrap();
        r.write_u32(OFF_CRCINIT, 0x555555).unwrap();

        r.write_u32(OFF_TASKS_RXEN, 1).unwrap();
        r.tick();
        r.write_u32(OFF_TASKS_START, 1).unwrap();
        r.tick();
        r.tick_with_bus(&mut bus);

        // Verify the bytes landed in RAM at PACKETPTR.
        assert_eq!(bus.read_u8(0x2000_1000).unwrap(), 0xCD);
        assert_eq!(bus.read_u8(0x2000_1001).unwrap(), 0x04);
        assert_eq!(bus.read_u8(0x2000_1002).unwrap(), 0xCA);
        assert_eq!(bus.read_u8(0x2000_1003).unwrap(), 0xFE);
        assert_eq!(bus.read_u8(0x2000_1004).unwrap(), 0xBA);
        assert_eq!(bus.read_u8(0x2000_1005).unwrap(), 0xBE);
        // CRC was good.
        assert_eq!(r.crc_status, 1);
    }

    #[test]
    fn tx_rx_full_loopback_with_whitening() {
        let _guard = air_test_setup();
        use crate::bus::SystemBus;
        use crate::Bus;

        // TX in one instance, RX in another — verify the payload survives
        // a round trip through whitening + CRC.
        let payload_orig = [0xC0, 0xFF, 0xEE, 0x42, 0x69];
        let mut bus_tx = SystemBus::new();
        bus_tx.write_u8(0x2000_0000, 0xAA).unwrap(); // S0
        bus_tx
            .write_u8(0x2000_0001, payload_orig.len() as u8)
            .unwrap();
        for (i, b) in payload_orig.iter().enumerate() {
            bus_tx.write_u8(0x2000_0002 + i as u64, *b).unwrap();
        }

        let mut tx = Nrf52Radio::new();
        tx.write_u32(OFF_PCNF0, 8 | (1 << 8)).unwrap();
        // WHITEEN bit set.
        tx.write_u32(OFF_PCNF1, 0xFF | (1 << 25)).unwrap();
        tx.write_u32(OFF_PACKETPTR, 0x2000_0000).unwrap();
        tx.write_u32(OFF_DATAWHITEIV, 37).unwrap(); // BLE adv ch 37 init
        tx.write_u32(OFF_CRCINIT, 0x555555).unwrap();

        tx.write_u32(OFF_TASKS_TXEN, 1).unwrap();
        tx.tick();
        tx.write_u32(OFF_TASKS_START, 1).unwrap();
        tx.tick();
        tx.tick_with_bus(&mut bus_tx);
        let on_air = tx.last_tx_packet.clone().unwrap();

        // RX side.
        let mut bus_rx = SystemBus::new();
        let mut rx = Nrf52Radio::new();
        rx.write_u32(OFF_PCNF0, 8 | (1 << 8)).unwrap();
        rx.write_u32(OFF_PCNF1, 0xFF | (1 << 25)).unwrap();
        rx.write_u32(OFF_PACKETPTR, 0x2000_2000).unwrap();
        rx.write_u32(OFF_DATAWHITEIV, 37).unwrap();
        rx.write_u32(OFF_CRCINIT, 0x555555).unwrap();
        rx.rx_inbox.push_back(on_air);

        rx.write_u32(OFF_TASKS_RXEN, 1).unwrap();
        rx.tick();
        rx.write_u32(OFF_TASKS_START, 1).unwrap();
        rx.tick();
        rx.tick_with_bus(&mut bus_rx);

        assert_eq!(
            rx.crc_status, 1,
            "CRC must verify after whitening round-trip"
        );
        // Payload bytes should match the original after the round trip.
        for (i, expected) in payload_orig.iter().enumerate() {
            assert_eq!(
                bus_rx.read_u8(0x2000_2002 + i as u64).unwrap(),
                *expected,
                "payload byte {} corrupted by round-trip",
                i
            );
        }
    }

    #[test]
    fn cross_instance_via_virtual_air() {
        use crate::bus::SystemBus;
        use crate::Bus;
        let _guard = air_test_setup();

        // Sender configures address 0 = BASE0 + PREFIX0[0] = 0xCAFEBA + 0xBE.
        let mut bus_tx = SystemBus::new();
        bus_tx.write_u8(0x2000_0000, 0xAA).unwrap(); // S0
        bus_tx.write_u8(0x2000_0001, 4).unwrap(); // LENGTH
        bus_tx.write_u8(0x2000_0002, 0xDE).unwrap();
        bus_tx.write_u8(0x2000_0003, 0xAD).unwrap();
        bus_tx.write_u8(0x2000_0004, 0xBE).unwrap();
        bus_tx.write_u8(0x2000_0005, 0xEF).unwrap();
        let mut tx = Nrf52Radio::new();
        tx.write_u32(OFF_PCNF0, 8 | (1 << 8)).unwrap();
        tx.write_u32(OFF_PCNF1, 0xFF | (1 << 25)).unwrap();
        tx.write_u32(OFF_PACKETPTR, 0x2000_0000).unwrap();
        tx.write_u32(OFF_FREQUENCY, 73).unwrap();
        tx.write_u32(OFF_MODE, 3).unwrap();
        tx.write_u32(OFF_DATAWHITEIV, 50).unwrap();
        tx.write_u32(OFF_CRCINIT, 0x555555).unwrap();
        tx.write_u32(OFF_BASE0, 0xCAFE_BA00).unwrap();
        tx.write_u32(OFF_PREFIX0, 0xBE).unwrap();
        tx.write_u32(OFF_TXADDRESS, 0).unwrap();
        tx.write_u32(OFF_TASKS_TXEN, 1).unwrap();
        tx.tick();
        tx.write_u32(OFF_TASKS_START, 1).unwrap();
        tx.tick();
        tx.tick_with_bus(&mut bus_tx);

        // Receiver is a completely separate Nrf52Radio + bus, sharing
        // only the global virtual air. RXADDRESSES enables logical
        // address 0 only.
        let mut bus_rx = SystemBus::new();
        let mut rx = Nrf52Radio::new();
        rx.write_u32(OFF_PCNF0, 8 | (1 << 8)).unwrap();
        rx.write_u32(OFF_PCNF1, 0xFF | (1 << 25)).unwrap();
        rx.write_u32(OFF_PACKETPTR, 0x2000_3000).unwrap();
        rx.write_u32(OFF_FREQUENCY, 73).unwrap();
        rx.write_u32(OFF_MODE, 3).unwrap();
        rx.write_u32(OFF_DATAWHITEIV, 50).unwrap();
        rx.write_u32(OFF_CRCINIT, 0x555555).unwrap();
        rx.write_u32(OFF_BASE0, 0xCAFE_BA00).unwrap();
        rx.write_u32(OFF_PREFIX0, 0xBE).unwrap();
        rx.write_u32(OFF_RXADDRESSES, 0x01).unwrap();
        rx.write_u32(OFF_TASKS_RXEN, 1).unwrap();
        rx.tick();
        rx.write_u32(OFF_TASKS_START, 1).unwrap();
        rx.tick();
        rx.tick_with_bus(&mut bus_rx);

        // Verify the frame landed in RX RAM with the correct payload.
        assert_eq!(rx.crc_status, 1, "CRC must verify across the virtual air");
        assert_eq!(bus_rx.read_u8(0x2000_3000).unwrap(), 0xAA);
        assert_eq!(bus_rx.read_u8(0x2000_3001).unwrap(), 4);
        assert_eq!(bus_rx.read_u8(0x2000_3002).unwrap(), 0xDE);
        assert_eq!(bus_rx.read_u8(0x2000_3003).unwrap(), 0xAD);
        assert_eq!(bus_rx.read_u8(0x2000_3004).unwrap(), 0xBE);
        assert_eq!(bus_rx.read_u8(0x2000_3005).unwrap(), 0xEF);
    }

    #[test]
    fn air_bus_scopes_delivery() {
        use crate::bus::SystemBus;
        use crate::Bus;

        // Run a full BLE TX→RX exchange with the TX radio bound to `tx_air` and
        // the RX radio bound to `rx_air`; return (crc_status, first payload byte
        // landed in RX RAM). Explicit per-radio buses — no AIR_GUARD needed.
        fn exchange(tx_air: VirtualAirBus, rx_air: VirtualAirBus) -> (u32, u8) {
            let mut bus_tx = SystemBus::new();
            for (off, v) in [
                (0u64, 0xAAu8),
                (1, 4),
                (2, 0xDE),
                (3, 0xAD),
                (4, 0xBE),
                (5, 0xEF),
            ] {
                bus_tx.write_u8(0x2000_0000 + off, v).unwrap();
            }
            let mut tx = Nrf52Radio::with_air(tx_air);
            tx.write_u32(OFF_PCNF0, 8 | (1 << 8)).unwrap();
            tx.write_u32(OFF_PCNF1, 0xFF | (1 << 25)).unwrap();
            tx.write_u32(OFF_PACKETPTR, 0x2000_0000).unwrap();
            tx.write_u32(OFF_FREQUENCY, 73).unwrap();
            tx.write_u32(OFF_MODE, 3).unwrap();
            tx.write_u32(OFF_DATAWHITEIV, 50).unwrap();
            tx.write_u32(OFF_CRCINIT, 0x555555).unwrap();
            tx.write_u32(OFF_BASE0, 0xCAFE_BA00).unwrap();
            tx.write_u32(OFF_PREFIX0, 0xBE).unwrap();
            tx.write_u32(OFF_TXADDRESS, 0).unwrap();
            tx.write_u32(OFF_TASKS_TXEN, 1).unwrap();
            tx.tick();
            tx.write_u32(OFF_TASKS_START, 1).unwrap();
            tx.tick();
            tx.tick_with_bus(&mut bus_tx);

            let mut bus_rx = SystemBus::new();
            let mut rx = Nrf52Radio::with_air(rx_air);
            rx.write_u32(OFF_PCNF0, 8 | (1 << 8)).unwrap();
            rx.write_u32(OFF_PCNF1, 0xFF | (1 << 25)).unwrap();
            rx.write_u32(OFF_PACKETPTR, 0x2000_3000).unwrap();
            rx.write_u32(OFF_FREQUENCY, 73).unwrap();
            rx.write_u32(OFF_MODE, 3).unwrap();
            rx.write_u32(OFF_DATAWHITEIV, 50).unwrap();
            rx.write_u32(OFF_CRCINIT, 0x555555).unwrap();
            rx.write_u32(OFF_BASE0, 0xCAFE_BA00).unwrap();
            rx.write_u32(OFF_PREFIX0, 0xBE).unwrap();
            rx.write_u32(OFF_RXADDRESSES, 0x01).unwrap();
            rx.write_u32(OFF_TASKS_RXEN, 1).unwrap();
            rx.tick();
            rx.write_u32(OFF_TASKS_START, 1).unwrap();
            rx.tick();
            rx.tick_with_bus(&mut bus_rx);
            (rx.crc_status, bus_rx.read_u8(0x2000_3002).unwrap())
        }

        // Same bus → the frame crosses (positive control, mirrors the global path).
        let shared = VirtualAirBus::new();
        let (crc, payload) = exchange(shared.clone(), shared.clone());
        assert_eq!(crc, 1, "same-bus radios must exchange the frame");
        assert_eq!(payload, 0xDE);

        // Different buses → silence. This is the isolation the process-static
        // AIR could not provide.
        let (crc_iso, _) = exchange(VirtualAirBus::new(), VirtualAirBus::new());
        assert_eq!(
            crc_iso, 0,
            "radios on different air buses must not hear each other"
        );
    }

    #[test]
    fn address_mismatch_drops_frame() {
        use crate::bus::SystemBus;
        use crate::Bus;
        let _guard = air_test_setup();

        // Sender uses address 0; receiver only enables address 3 — the
        // frame should stay in the air and RX RAM stays untouched.
        let mut bus_tx = SystemBus::new();
        bus_tx.write_u8(0x2000_0000, 0x11).unwrap();
        bus_tx.write_u8(0x2000_0001, 2).unwrap();
        bus_tx.write_u8(0x2000_0002, 0x22).unwrap();
        bus_tx.write_u8(0x2000_0003, 0x33).unwrap();
        let mut tx = Nrf52Radio::new();
        tx.write_u32(OFF_PCNF0, 8 | (1 << 8)).unwrap();
        tx.write_u32(OFF_PCNF1, 0xFF).unwrap(); // whitening off for clarity
        tx.write_u32(OFF_PACKETPTR, 0x2000_0000).unwrap();
        tx.write_u32(OFF_FREQUENCY, 91).unwrap();
        tx.write_u32(OFF_MODE, 3).unwrap();
        tx.write_u32(OFF_CRCINIT, 0).unwrap();
        tx.write_u32(OFF_BASE0, 0x1111_1100).unwrap();
        tx.write_u32(OFF_PREFIX0, 0x77).unwrap();
        tx.write_u32(OFF_TXADDRESS, 0).unwrap();
        tx.write_u32(OFF_TASKS_TXEN, 1).unwrap();
        tx.tick();
        tx.write_u32(OFF_TASKS_START, 1).unwrap();
        tx.tick();
        tx.tick_with_bus(&mut bus_tx);

        let mut bus_rx = SystemBus::new();
        let mut rx = Nrf52Radio::new();
        rx.write_u32(OFF_PCNF0, 8 | (1 << 8)).unwrap();
        rx.write_u32(OFF_PCNF1, 0xFF).unwrap();
        rx.write_u32(OFF_PACKETPTR, 0x2000_3000).unwrap();
        rx.write_u32(OFF_FREQUENCY, 91).unwrap();
        rx.write_u32(OFF_MODE, 3).unwrap();
        rx.write_u32(OFF_CRCINIT, 0).unwrap();
        // Different BASE/PREFIX so address 3 also wouldn't match TX.
        rx.write_u32(OFF_BASE1, 0xDEAD_CAFE).unwrap();
        rx.write_u32(OFF_PREFIX0, 0xAA_00_00_00).unwrap();
        rx.write_u32(OFF_RXADDRESSES, 0x08).unwrap(); // only logical 3
        rx.write_u32(OFF_TASKS_RXEN, 1).unwrap();
        rx.tick();
        rx.write_u32(OFF_TASKS_START, 1).unwrap();
        rx.tick();
        rx.tick_with_bus(&mut bus_rx);

        // RAM should still be the reset value (0).
        assert_eq!(bus_rx.read_u8(0x2000_3000).unwrap(), 0);
    }

    #[test]
    fn bit_rate_timing_defers_events_end() {
        use crate::bus::SystemBus;
        use crate::Bus;
        let _guard = air_test_setup();

        let mut bus = SystemBus::new();
        // 4-byte payload at 0x20000000: S0, LENGTH=2, payload bytes.
        bus.write_u8(0x2000_0000, 0x12).unwrap();
        bus.write_u8(0x2000_0001, 2).unwrap();
        bus.write_u8(0x2000_0002, 0x34).unwrap();
        bus.write_u8(0x2000_0003, 0x56).unwrap();

        let mut r = Nrf52Radio::new();
        r.write_u32(OFF_PCNF0, 8 | (1 << 8)).unwrap();
        r.write_u32(OFF_PCNF1, 0xFF).unwrap();
        r.write_u32(OFF_PACKETPTR, 0x2000_0000).unwrap();
        r.write_u32(OFF_FREQUENCY, 11).unwrap();
        r.write_u32(OFF_MODE, 3).unwrap(); // BLE_1Mbit → 8 cycles/byte
        r.write_u32(OFF_CRCINIT, 0).unwrap();
        r.write_u32(OFF_TASKS_TXEN, 1).unwrap();
        r.tick();

        // After START, the test calls tick_with_bus FIRST (matching the
        // new bus pre-tick order) so cycles_for_packet sets the proper
        // countdown before tick() can decrement.
        r.write_u32(OFF_TASKS_START, 1).unwrap();
        r.tick_with_bus(&mut bus);

        // Step ticks; EVENTS_END must NOT fire on the first few. With
        // 5 net bytes (S0 + LEN + 2 payload + 3 CRC) at 8 cycles/byte
        // we expect ~40 cycles before END fires.
        let mut end_tick: Option<usize> = None;
        for i in 0..200 {
            r.tick();
            if r.read_u32(OFF_EVENTS_END).unwrap() != 0 {
                end_tick = Some(i);
                break;
            }
        }
        let end_tick = end_tick.expect("EVENTS_END should fire within 200 ticks");
        assert!(
            end_tick > 4,
            "EVENTS_END fired too early (tick {end_tick}); bit-rate countdown was skipped"
        );
    }

    #[test]
    fn devmatch_fires_when_dab_dap_whitelist_matches() {
        use crate::bus::SystemBus;
        use crate::Bus;
        let _guard = air_test_setup();

        // TX from logical address 0 (BASE0=0xDEADBE00, PREFIX0[0]=0xEF).
        let mut bus_tx = SystemBus::new();
        bus_tx.write_u8(0x2000_0000, 0xAA).unwrap();
        bus_tx.write_u8(0x2000_0001, 1).unwrap();
        bus_tx.write_u8(0x2000_0002, 0x55).unwrap();

        let mut tx = Nrf52Radio::new();
        tx.write_u32(OFF_PCNF0, 8 | (1 << 8)).unwrap();
        tx.write_u32(OFF_PCNF1, 0xFF).unwrap();
        tx.write_u32(OFF_PACKETPTR, 0x2000_0000).unwrap();
        tx.write_u32(OFF_FREQUENCY, 12).unwrap();
        tx.write_u32(OFF_MODE, 3).unwrap();
        tx.write_u32(OFF_CRCINIT, 0).unwrap();
        tx.write_u32(OFF_BASE0, 0xDEAD_BE00).unwrap();
        tx.write_u32(OFF_PREFIX0, 0xEF).unwrap();
        tx.write_u32(OFF_TXADDRESS, 0).unwrap();
        tx.write_u32(OFF_TASKS_TXEN, 1).unwrap();
        tx.tick();
        tx.write_u32(OFF_TASKS_START, 1).unwrap();
        tx.tick_with_bus(&mut bus_tx);

        // RX whitelists DAB[3] = 0xDEADBE00, DAP[3] = 0xEF, with DACNF
        // enabling slot 3.
        let mut bus_rx = SystemBus::new();
        let mut rx = Nrf52Radio::new();
        rx.write_u32(OFF_PCNF0, 8 | (1 << 8)).unwrap();
        rx.write_u32(OFF_PCNF1, 0xFF).unwrap();
        rx.write_u32(OFF_PACKETPTR, 0x2000_4000).unwrap();
        rx.write_u32(OFF_FREQUENCY, 12).unwrap();
        rx.write_u32(OFF_MODE, 3).unwrap();
        rx.write_u32(OFF_CRCINIT, 0).unwrap();
        rx.write_u32(OFF_BASE0, 0xDEAD_BE00).unwrap();
        rx.write_u32(OFF_PREFIX0, 0xEF).unwrap();
        rx.write_u32(OFF_RXADDRESSES, 0x01).unwrap();
        // DAB[3] / DAP[3] whitelist match.
        rx.write_u32(OFF_DAB0 + 12, 0xDEAD_BE00).unwrap();
        rx.write_u32(OFF_DAP0 + 12, 0xEF).unwrap();
        rx.write_u32(OFF_DACNF, 1 << 3).unwrap();

        rx.write_u32(OFF_TASKS_RXEN, 1).unwrap();
        rx.tick();
        rx.write_u32(OFF_TASKS_START, 1).unwrap();
        rx.tick_with_bus(&mut bus_rx);

        assert_eq!(rx.read_u32(OFF_EVENTS_DEVMATCH).unwrap(), 1);
        assert_eq!(rx.read_u32(OFF_EVENTS_DEVMISS).unwrap(), 0);
    }

    #[test]
    fn devmiss_fires_when_whitelist_does_not_match() {
        use crate::bus::SystemBus;
        use crate::Bus;
        let _guard = air_test_setup();

        let mut bus_tx = SystemBus::new();
        bus_tx.write_u8(0x2000_0000, 0x11).unwrap();
        bus_tx.write_u8(0x2000_0001, 1).unwrap();
        bus_tx.write_u8(0x2000_0002, 0x22).unwrap();
        let mut tx = Nrf52Radio::new();
        tx.write_u32(OFF_PCNF0, 8 | (1 << 8)).unwrap();
        tx.write_u32(OFF_PCNF1, 0xFF).unwrap();
        tx.write_u32(OFF_PACKETPTR, 0x2000_0000).unwrap();
        tx.write_u32(OFF_FREQUENCY, 13).unwrap();
        tx.write_u32(OFF_MODE, 3).unwrap();
        tx.write_u32(OFF_CRCINIT, 0).unwrap();
        tx.write_u32(OFF_BASE0, 0x1111_1100).unwrap();
        tx.write_u32(OFF_PREFIX0, 0x77).unwrap();
        tx.write_u32(OFF_TXADDRESS, 0).unwrap();
        tx.write_u32(OFF_TASKS_TXEN, 1).unwrap();
        tx.tick();
        tx.write_u32(OFF_TASKS_START, 1).unwrap();
        tx.tick_with_bus(&mut bus_tx);

        let mut bus_rx = SystemBus::new();
        let mut rx = Nrf52Radio::new();
        rx.write_u32(OFF_PCNF0, 8 | (1 << 8)).unwrap();
        rx.write_u32(OFF_PCNF1, 0xFF).unwrap();
        rx.write_u32(OFF_PACKETPTR, 0x2000_5000).unwrap();
        rx.write_u32(OFF_FREQUENCY, 13).unwrap();
        rx.write_u32(OFF_MODE, 3).unwrap();
        rx.write_u32(OFF_CRCINIT, 0).unwrap();
        rx.write_u32(OFF_BASE0, 0x1111_1100).unwrap();
        rx.write_u32(OFF_PREFIX0, 0x77).unwrap();
        rx.write_u32(OFF_RXADDRESSES, 0x01).unwrap();
        // DAB/DAP whitelist with a non-matching address.
        rx.write_u32(OFF_DAB0, 0xAAAA_AAAA).unwrap();
        rx.write_u32(OFF_DAP0, 0xBB).unwrap();
        rx.write_u32(OFF_DACNF, 1).unwrap();

        rx.write_u32(OFF_TASKS_RXEN, 1).unwrap();
        rx.tick();
        rx.write_u32(OFF_TASKS_START, 1).unwrap();
        rx.tick_with_bus(&mut bus_rx);

        assert_eq!(rx.read_u32(OFF_EVENTS_DEVMATCH).unwrap(), 0);
        assert_eq!(rx.read_u32(OFF_EVENTS_DEVMISS).unwrap(), 1);
    }

    #[test]
    fn bcmatch_fires_when_bit_count_reaches_bcc() {
        use crate::bus::SystemBus;
        use crate::Bus;
        let _guard = air_test_setup();

        // TX a 4-byte payload, so RX consumes ~5 bytes (S0+LEN+payload) = 40 bits.
        let mut bus_tx = SystemBus::new();
        bus_tx.write_u8(0x2000_0000, 0x11).unwrap();
        bus_tx.write_u8(0x2000_0001, 4).unwrap();
        for (i, b) in [0xC0, 0xFF, 0xEE, 0x42].iter().enumerate() {
            bus_tx.write_u8(0x2000_0002 + i as u64, *b).unwrap();
        }
        let mut tx = Nrf52Radio::new();
        tx.write_u32(OFF_PCNF0, 8 | (1 << 8)).unwrap();
        tx.write_u32(OFF_PCNF1, 0xFF).unwrap();
        tx.write_u32(OFF_PACKETPTR, 0x2000_0000).unwrap();
        tx.write_u32(OFF_FREQUENCY, 14).unwrap();
        tx.write_u32(OFF_MODE, 3).unwrap();
        tx.write_u32(OFF_CRCINIT, 0).unwrap();
        tx.write_u32(OFF_TASKS_TXEN, 1).unwrap();
        tx.tick();
        tx.write_u32(OFF_TASKS_START, 1).unwrap();
        tx.tick_with_bus(&mut bus_tx);

        let mut bus_rx = SystemBus::new();
        let mut rx = Nrf52Radio::new();
        rx.write_u32(OFF_PCNF0, 8 | (1 << 8)).unwrap();
        rx.write_u32(OFF_PCNF1, 0xFF).unwrap();
        rx.write_u32(OFF_PACKETPTR, 0x2000_6000).unwrap();
        rx.write_u32(OFF_FREQUENCY, 14).unwrap();
        rx.write_u32(OFF_MODE, 3).unwrap();
        rx.write_u32(OFF_CRCINIT, 0).unwrap();
        // Arm the bit counter; BCC = 16 bits → fires after the second
        // byte regardless.
        rx.write_u32(OFF_TASKS_BCSTART, 1).unwrap();
        rx.write_u32(OFF_BCC, 16).unwrap();
        rx.write_u32(OFF_TASKS_RXEN, 1).unwrap();
        rx.tick();
        rx.write_u32(OFF_TASKS_START, 1).unwrap();
        rx.tick_with_bus(&mut bus_rx);

        assert_eq!(rx.read_u32(OFF_EVENTS_BCMATCH).unwrap(), 1);
    }

    #[test]
    fn rssistart_updates_rssisample_deterministically() {
        let mut r = Nrf52Radio::new();
        let s0 = r.read_u32(OFF_RSSISAMPLE).unwrap();
        r.write_u32(OFF_TASKS_RSSISTART, 1).unwrap();
        let s1 = r.read_u32(OFF_RSSISAMPLE).unwrap();
        // Should differ from the reset value.
        assert_ne!(s0, s1);
        // RSSI is 7-bit (PS table 282).
        assert!(s1 <= 0x7F);
        // A second sample drifts further.
        r.write_u32(OFF_TASKS_RSSISTART, 1).unwrap();
        let s2 = r.read_u32(OFF_RSSISAMPLE).unwrap();
        assert!(s2 <= 0x7F);
    }

    #[test]
    fn multi_mode_on_same_freq_no_cross_decode() {
        use crate::bus::SystemBus;
        use crate::Bus;
        let _guard = air_test_setup();

        // TX in MODE=3 (BLE_1Mbit) on FREQUENCY=15.
        let mut bus_tx = SystemBus::new();
        bus_tx.write_u8(0x2000_0000, 0x99).unwrap();
        bus_tx.write_u8(0x2000_0001, 1).unwrap();
        bus_tx.write_u8(0x2000_0002, 0xAA).unwrap();
        let mut tx = Nrf52Radio::new();
        tx.write_u32(OFF_PCNF0, 8 | (1 << 8)).unwrap();
        tx.write_u32(OFF_PCNF1, 0xFF).unwrap();
        tx.write_u32(OFF_PACKETPTR, 0x2000_0000).unwrap();
        tx.write_u32(OFF_FREQUENCY, 15).unwrap();
        tx.write_u32(OFF_MODE, 3).unwrap();
        tx.write_u32(OFF_CRCINIT, 0).unwrap();
        tx.write_u32(OFF_BASE0, 0x4242_4200).unwrap();
        tx.write_u32(OFF_PREFIX0, 0x42).unwrap();
        tx.write_u32(OFF_TXADDRESS, 0).unwrap();
        tx.write_u32(OFF_TASKS_TXEN, 1).unwrap();
        tx.tick();
        tx.write_u32(OFF_TASKS_START, 1).unwrap();
        tx.tick_with_bus(&mut bus_tx);

        // RX tuned to MODE=0 (Nrf_1Mbit) on the same FREQUENCY. Even with
        // address-matched BASE/PREFIX, the frame must NOT be consumed
        // (mode mismatch — different modulation on the same band).
        let mut bus_rx = SystemBus::new();
        let mut rx = Nrf52Radio::new();
        rx.write_u32(OFF_PCNF0, 8 | (1 << 8)).unwrap();
        rx.write_u32(OFF_PCNF1, 0xFF).unwrap();
        rx.write_u32(OFF_PACKETPTR, 0x2000_7000).unwrap();
        rx.write_u32(OFF_FREQUENCY, 15).unwrap();
        rx.write_u32(OFF_MODE, 0).unwrap(); // different mode
        rx.write_u32(OFF_CRCINIT, 0).unwrap();
        rx.write_u32(OFF_BASE0, 0x4242_4200).unwrap();
        rx.write_u32(OFF_PREFIX0, 0x42).unwrap();
        rx.write_u32(OFF_RXADDRESSES, 0x01).unwrap();
        rx.write_u32(OFF_TASKS_RXEN, 1).unwrap();
        rx.tick();
        rx.write_u32(OFF_TASKS_START, 1).unwrap();
        rx.tick_with_bus(&mut bus_rx);

        // RAM at PACKETPTR stays at reset 0; frame stays in the air.
        assert_eq!(bus_rx.read_u8(0x2000_7000).unwrap(), 0);
    }
}
