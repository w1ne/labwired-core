//! TI CC2538 2.4-GHz IEEE 802.15.4 RF core (`0x4008_8000`). Alone, a radio
//! in an empty channel; on a shared [`Ieee802154AirBus`] (a world's 802.15.4
//! medium), a transceiver that sends and receives frames.
//!
//! Source: SWRU319C §23 "Radio": Table 23-9 "RFCORE_XREG Register Summary"
//! (reset values), Table 23-10 / Table 12-1 "RFCORE_SFR Register Summary",
//! §23.9.1 RX control (`RXENABLE`), §23.9.2 RX state timing, §23.10 TX,
//! Figure 23-18 "Main FSM" and Table 23-4 (FSM state numbers), §23.14.9
//! command strobes; §12 for the MAC timer's `MTCTRL`.
//!
//! Window layout: `0x000-0x5FF` RAM + FFSM (FIFOs, address/PAN filter
//! registers), `0x600-0x7FF` XREG, `0x800-0x83F` SFR.
//!
//! Modelled:
//! - every XREG at its Table 23-9 reset value, stored and read back;
//! - the immediate command strobes written to `RFST`: `ISRXON` (sets
//!   `RXENABLE[7]`), `ISRFOFF` (clears `RXENABLE`, aborts TX), `ISTXON` /
//!   `ISTXONCCA` (send the TX FIFO; `FRMCTRL1.SET_RXENMASK_ON_TX` sets
//!   `RXENABLE[6]`), `ISFLUSHTX`, `ISFLUSHRX`; `RXMASKSET`/`RXMASKCLR` set and
//!   clear `RXENABLE` bits;
//! - the main FSM as `FSMSTAT0.FSM_FFCTRL_STATE`, timed from the bus cycle
//!   clock: idle 0; RX calibration 2 for the 192-µs turnaround, then SFD wait
//!   3; TX calibration 32 for 192 µs, then TX 34 for the frame (5-byte SHR +
//!   the TX FIFO, 32 µs per byte, + the 2-byte FCS when `FRMCTRL0.AUTOCRC`),
//!   then back to RX calibration if `RXENABLE != 0`, else idle;
//! - `FSMSTAT1`: `RX_ACTIVE`, `TX_ACTIVE`, `LOCK_STATUS` after calibration,
//!   and `CCA`/`SAMPLED_CCA` from the RSSI against `CCACTRL0.CCA_THR`;
//! - `RSSISTAT.RSSI_VALID` 8 symbols (128 µs) after RX is ready, and `RSSI`
//!   = the noise floor then (-100 dBm, raw `-100 + 73`; `0x80` = invalid
//!   otherwise): there is no other transmitter, so the channel is clear;
//! - `RFRND` I/Q noise bits from a fixed-seed LFSR, one step per read, so a
//!   run is reproducible;
//! - the TX FIFO as a byte count (`TXFIFOCNT`, `RFERRF.TXOVERF` past 128 B)
//!   with the last frame sent kept for inspection;
//! - `MTCTRL.STATE` follows `MTCTRL.RUN` at once.
//!
//!
//! On a shared air (attached with `attach_ieee802154_air`; SWRU319C §23.9
//! "Receive Mode", §23.9.5 frame filtering, §23.9.8 AUTOACK, §23.10 TX):
//! - `ISTXON` puts the TX FIFO's MPDU (the bytes after the length byte, plus
//!   the FCS when `FRMCTRL0.AUTOCRC`) on the air, its first bit 192 µs after
//!   the strobe;
//! - a frame from another radio on the same `FREQCTRL` channel, which started
//!   after this receiver finished its RX calibration and did not overlap this
//!   radio's own transmission, is filtered with `FRMFILT0/1` and the FFSM
//!   `PAN_ID`/`SHORT_ADDR`/`EXT_ADDR` and, if accepted, appended to the
//!   128-byte RX FIFO once its last bit has passed: length byte, MPDU, then
//!   (AUTOCRC) RSSI and `CRC_OK | correlation` instead of the FCS, Figure
//!   23-13. `RFDATA` pops it, `RXFIFOCNT`/`RXFIRST` report it, `FSMSTAT1`
//!   `FIFO`/`FIFOP` follow it (a complete frame is always in the FIFO, so
//!   `FIFOP` = `FIFO`), `ISFLUSHRX` empties it, and a frame that does not fit
//!   sets `RFERRF.RXOVERF` and is dropped;
//! - `RFIRQF0` `SFD | FIFOP | RXPKTDONE | FRAME_ACCEPTED` on reception and
//!   `RFIRQF1.TXDONE` when a transmission ends; the RF TX/RX interrupt is the
//!   level `RFIRQF0 & RFIRQM0 | RFIRQF1 & RFIRQM1`;
//! - `FSMSTAT1.SFD` while another radio's frame is on the air past its SFD,
//!   or this radio's own frame is past its SFD; `RSSI`/`CCA` see another
//!   radio's frame on the channel as a -40 dBm signal (channel busy);
//! - AUTOACK: the air sends the ACK on this radio's behalf from its published
//!   filter settings (see [`crate::peripherals::ieee802154_air`]).
//!
//! NOT modelled: the CSP (`RFST` program instructions other than the
//! immediate strobes above), the MAC timer count (`MTM*`/`MTMOVF*` are
//! storage), the RF error interrupt (`RFERRF` flags are set but raise no
//! IRQ), `RFIRQF0`/`RFIRQF1` sources other than the ones listed, source
//! matching and AUTOPEND, `FIFOPCTRL` thresholds, uDMA requests (firmware
//! triggers the uDMA by software), the analog front end (`TXPOWER`/AGC are
//! storage) and calibration failures. Alone (no air), nothing is received and
//! a sent frame goes nowhere.

use super::{lane_merge, lane_read};
use crate::peripherals::ieee802154_air::{self, Ieee802154AirBus, RxFilter};
use crate::{Bus, CycleClock, Peripheral, SimResult};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

/// Window size (4 KiB block; the documented registers end at `0x83C`).
pub const RFCORE_SIZE: usize = 0x1000;

// FFSM local address information (Table 23-2).
const EXT_ADDR0: u64 = 0x5A8;
const PAN_ID0: u64 = 0x5C8;
const PAN_ID1: u64 = 0x5CC;
const SHORT_ADDR0: u64 = 0x5D0;
const SHORT_ADDR1: u64 = 0x5D4;
// XREG (Table 23-9), offsets in the 4-KiB window.
const FRMFILT0: u64 = 0x600;
const FRMFILT1: u64 = 0x604;
const FRMCTRL0: u64 = 0x624;
const FRMCTRL1: u64 = 0x628;
const RXENABLE: u64 = 0x62C;
const RXMASKSET: u64 = 0x630;
const RXMASKCLR: u64 = 0x634;
const FREQCTRL: u64 = 0x63C;
const FSMSTAT0: u64 = 0x648;
const FSMSTAT1: u64 = 0x64C;
const CCACTRL0: u64 = 0x658;
const RSSI: u64 = 0x660;
const RSSISTAT: u64 = 0x664;
const RXFIRST: u64 = 0x668;
const RXFIFOCNT: u64 = 0x66C;
const TXFIFOCNT: u64 = 0x670;
const RFIRQM0: u64 = 0x68C;
const RFIRQM1: u64 = 0x690;
const RFRND: u64 = 0x69C;
const FREQEST: u64 = 0x6A8;
// SFR (Table 23-10 / 12-1).
const MTCTRL: u64 = 0x804;
const RFDATA: u64 = 0x828;
const RFERRF: u64 = 0x82C;
const RFIRQF1: u64 = 0x830;
const RFIRQF0: u64 = 0x834;
const RFST: u64 = 0x838;

/// XREG reset values that are not 0 (Table 23-9).
const XREG_RESETS: &[(u64, u32)] = &[
    (0x600, 0x0D), // FRMFILT0
    (0x604, 0x78), // FRMFILT1
    (0x608, 0x07), // SRCMATCH
    (FRMCTRL0, 0x40),
    (FRMCTRL1, 0x01),
    (0x638, 0x0F), // FREQTUNE
    (0x63C, 0x0B), // FREQCTRL
    (0x640, 0xF5), // TXPOWER
    (0x644, 0x69), // TXCTRL
    (0x650, 0x40), // FIFOPCTRL
    (0x654, 0x01), // FSMCTRL
    (CCACTRL0, 0xF8),
    (0x65C, 0x1A), // CCACTRL1
    (0x6A0, 0x85), // MDMCTRL0
    (0x6A4, 0x14), // MDMCTRL1
    (0x6AC, 0x3F), // RXCTRL
    (0x6B0, 0x5A), // FSCTRL
    (0x6B4, 0x24), // FSCAL0
    (0x6BC, 0x20), // FSCAL2
    (0x6C0, 0x2A), // FSCAL3
    (0x6C4, 0x5F), // AGCCTRL0
    (0x6C8, 0x11), // AGCCTRL1
    (0x6CC, 0xFE), // AGCCTRL2
    (0x6D0, 0x2E), // AGCCTRL3
    (0x6D4, 0x10), // ADCTEST0
    (0x6D8, 0x0E), // ADCTEST1
    (0x6DC, 0x03), // ADCTEST2
    (0x6E0, 0x75), // MDMTEST0
    (0x6E4, 0x08), // MDMTEST1
    (0x6F0, 0x28), // DACTEST2
    (0x794, 0xFF), // CSPT
    (0x7E8, 0x0F), // TXFILTCFG
    (MTCTRL, 0x02),
    (RFST, 0xD0),
];
/// `CSPPROG0`-`CSPPROG23` (`0x700`-`0x75C`) reset to `0xD0`.
const CSPPROG: std::ops::RangeInclusive<u64> = 0x700..=0x75C;

const FRMCTRL0_AUTOACK: u32 = 1 << 5;
const FRMCTRL0_AUTOCRC: u32 = 1 << 6;
const FRMCTRL1_SET_RXENMASK_ON_TX: u32 = 1 << 0;
const FRMCTRL1_PENDING_OR: u32 = 1 << 2;
const FSMSTAT0_CAL_DONE: u32 = 1 << 7;
const FSMSTAT1_FIFO: u32 = 1 << 7;
const FSMSTAT1_FIFOP: u32 = 1 << 6;
const FSMSTAT1_SFD: u32 = 1 << 5;
const FSMSTAT1_CCA: u32 = 1 << 4;
const FSMSTAT1_SAMPLED_CCA: u32 = 1 << 3;
const FSMSTAT1_LOCK_STATUS: u32 = 1 << 2;
const FSMSTAT1_TX_ACTIVE: u32 = 1 << 1;
const FSMSTAT1_RX_ACTIVE: u32 = 1 << 0;
const RFERRF_TXUNDERF: u32 = 1 << 5;
const RFERRF_TXOVERF: u32 = 1 << 4;
const RFERRF_RXOVERF: u32 = 1 << 2;
const RFIRQF0_SFD: u32 = 1 << 1;
const RFIRQF0_FIFOP: u32 = 1 << 2;
const RFIRQF0_FRAME_ACCEPTED: u32 = 1 << 5;
const RFIRQF0_RXPKTDONE: u32 = 1 << 6;
const RFIRQF1_TXDONE: u32 = 1 << 1;
const MTCTRL_RUN: u32 = 1 << 0;
const MTCTRL_STATE: u32 = 1 << 2;
/// `MTCTRL` writable bits: RUN, SYNC, LATCH_MODE.
const MTCTRL_RW: u32 = 0b1011;

// Immediate command strobes (§23.14.9).
const ISRXON: u8 = 0xE3;
const ISTXON: u8 = 0xE9;
const ISTXONCCA: u8 = 0xEA;
const ISFLUSHRX: u8 = 0xED;
const ISFLUSHTX: u8 = 0xEE;
const ISRFOFF: u8 = 0xEF;

// FSM state numbers (Figure 23-18, Table 23-4).
const FSM_IDLE: u32 = 0;
const FSM_RX_CALIBRATION: u32 = 2;
const FSM_SFD_WAIT: u32 = 3;
const FSM_TX_CALIBRATION: u32 = 32;
const FSM_TX: u32 = 34;

/// RX/TX turnaround (§23.9.2) and one 802.15.4 symbol (16 µs; 32 µs per byte).
const TURNAROUND_US: u64 = 192;
const RSSI_SETTLE_US: u64 = 8 * 16;
const BYTE_US: u64 = 32;
/// SHR: 4-byte preamble + SFD.
const SHR_BYTES: u64 = 5;
const FCS_BYTES: u64 = 2;
const TXFIFO_BYTES: usize = 128;
const RXFIFO_BYTES: usize = 128;
/// RSSI of another radio's frame (a chosen strong in-range value), and the
/// correlation value appended with `CRC_OK` (a typical good-link figure).
const SIGNAL_DBM: i32 = -40;
const RX_CORRELATION: u8 = 108;
/// Noise floor reported when RSSI is valid (a chosen quiet-channel value),
/// and the RSSI register offset (§23.10.3: "with an offset of 73 dB, reading
/// an RSSI value of -10 ... means -83 dBm"; Contiki-NG `RSSI_OFFSET`).
const NOISE_FLOOR_DBM: i32 = -100;
const RSSI_OFFSET_DB: i32 = 73;
const RSSI_INVALID: u32 = 0x80;
/// CC2538 reset SYS clock: 16-MHz HF-RC with `SYS_DIV` = 1 (Table 7-3).
pub const DEFAULT_CLOCK_HZ: u64 = 16_000_000;
const LFSR_SEED: u16 = 0xACE1;

/// One transmission: TX calibration from `start`, frame from `start` + 192 µs,
/// ending at `end`.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
struct TxBurst {
    start: u64,
    end: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Cc2538RfCore {
    clock_hz: u64,
    regs: Vec<u32>,
    rxenable: u32,
    /// Cycle at which the current RX calibration starts.
    rx_since: u64,
    tx: Option<TxBurst>,
    /// `RFIRQF1.TXDONE` already raised for the current burst.
    tx_done_flagged: bool,
    tx_fifo: Vec<u8>,
    last_tx_frame: Vec<u8>,
    /// Received frames as Figure 23-13 lays them out; `RFDATA` reads pop.
    rx_fifo: RefCell<VecDeque<u8>>,
    lfsr: Cell<u16>,
    #[serde(skip)]
    clock: Option<CycleClock>,
    #[serde(skip)]
    air: Option<(Ieee802154AirBus, usize)>,
}

impl Cc2538RfCore {
    pub fn new(clock_hz: u64) -> Self {
        let mut regs = vec![0u32; RFCORE_SIZE / 4];
        for &(off, val) in XREG_RESETS {
            regs[(off / 4) as usize] = val;
        }
        for off in CSPPROG.step_by(4) {
            regs[(off / 4) as usize] = 0xD0;
        }
        Self {
            clock_hz: clock_hz.max(1),
            regs,
            rxenable: 0,
            rx_since: 0,
            tx: None,
            tx_done_flagged: false,
            tx_fifo: Vec::new(),
            last_tx_frame: Vec::new(),
            rx_fifo: RefCell::new(VecDeque::new()),
            lfsr: Cell::new(LFSR_SEED),
            clock: None,
            air: None,
        }
    }

    /// The last frame `ISTXON` sent (TX FIFO contents: PHR length byte then
    /// the PSDU without the hardware FCS).
    pub fn last_tx_frame(&self) -> &[u8] {
        &self.last_tx_frame
    }

    fn reg(&self, off: u64) -> u32 {
        self.regs[(off / 4) as usize]
    }

    fn now(&self) -> u64 {
        self.clock.as_ref().map_or(0, CycleClock::now)
    }

    /// `cycles` of this radio's clock in ns of world time.
    fn ns(&self, cycles: u64) -> u64 {
        (u128::from(cycles) * 1_000_000_000 / u128::from(self.clock_hz)) as u64
    }

    fn channel(&self) -> u32 {
        self.reg(FREQCTRL) & 0x7F
    }

    /// Another radio's frame on this radio's channel at `now`, if any, as
    /// world-time `(start_ns, end_ns)`.
    fn air_signal(&self, now: u64) -> Option<(u64, u64)> {
        let (air, node) = self.air.as_ref()?;
        air.on_air(*node, self.channel(), self.ns(now))
    }

    /// This radio's receive settings as the air needs them for AUTOACK.
    fn rx_filter(&self) -> RxFilter {
        let f0 = self.reg(FRMFILT0);
        let mut ext = [0u8; 8];
        for (i, b) in ext.iter_mut().enumerate() {
            *b = self.reg(EXT_ADDR0 + 4 * i as u64) as u8;
        }
        RxFilter {
            listening: self.rxenable != 0,
            channel: self.channel(),
            frame_filter: f0 & 1 != 0,
            pan_coordinator: f0 & 2 != 0,
            max_frame_version: ((f0 >> 2) & 3) as u8,
            fcf_reserved_mask: ((f0 >> 4) & 7) as u8,
            // FRMFILT1 ACCEPT_FT_0..3 at bits 3..6; ACCEPT_FT_4TO7 is not
            // modelled (reserved frame types are always rejected).
            accept_types: ((self.reg(FRMFILT1) >> 3) & 0xF) as u8,
            autoack: self.reg(FRMCTRL0) & FRMCTRL0_AUTOACK != 0,
            ack_pending: self.reg(FRMCTRL1) & FRMCTRL1_PENDING_OR != 0,
            pan_id: (self.reg(PAN_ID0) & 0xFF | (self.reg(PAN_ID1) & 0xFF) << 8) as u16,
            short_addr: (self.reg(SHORT_ADDR0) & 0xFF | (self.reg(SHORT_ADDR1) & 0xFF) << 8) as u16,
            ext_addr: ext,
        }
    }

    fn publish_filter(&self) {
        if let Some((air, node)) = &self.air {
            air.set_filter(*node, &self.rx_filter());
        }
    }

    /// Level of the RF TX/RX interrupt.
    fn irq_level(&self) -> bool {
        (self.reg(RFIRQF0) & self.reg(RFIRQM0)) | (self.reg(RFIRQF1) & self.reg(RFIRQM1)) != 0
    }

    /// Cycle from which a frame's first bit can be detected: RX on, the 192-µs
    /// calibration done, and after this radio's own burst.
    fn rx_ready_cycle(&self) -> Option<u64> {
        if self.rxenable == 0 {
            return None;
        }
        let start = match self.tx {
            Some(b) => self.rx_since.max(b.end),
            None => self.rx_since,
        };
        Some(start + self.cycles(TURNAROUND_US))
    }

    /// Take the frames whose last bit has passed and queue the accepted ones.
    fn receive(&mut self, now: u64) {
        let Some((air, node)) = self.air.clone() else {
            return;
        };
        let frames = air.take_received(node, 0, self.ns(now));
        if frames.is_empty() {
            return;
        }
        let filter = self.rx_filter();
        let own = self.tx.map(|b| {
            (
                self.ns(b.start + self.cycles(TURNAROUND_US)),
                self.ns(b.end),
            )
        });
        let ready = self.rx_ready_cycle().map(|c| self.ns(c));
        for f in frames {
            if f.channel != filter.channel || ready.is_none_or(|r| f.start_ns < r) {
                continue;
            }
            if own.is_some_and(|(s, e)| f.start_ns < e && f.end_ns > s) {
                continue;
            }
            if !ieee802154_air::filter(&filter, &f.mpdu).accepted {
                continue;
            }
            let mut fifo = self.rx_fifo.borrow_mut();
            if fifo.len() + 1 + f.mpdu.len() > RXFIFO_BYTES {
                self.regs[(RFERRF / 4) as usize] |= RFERRF_RXOVERF;
                continue;
            }
            let n = f.mpdu.len();
            fifo.push_back(n as u8);
            fifo.extend(&f.mpdu[..n.saturating_sub(2)]);
            if self.reg(FRMCTRL0) & FRMCTRL0_AUTOCRC != 0 {
                let (body, tail) = f.mpdu.split_at(n.saturating_sub(2));
                let crc_ok = tail.len() == 2
                    && ieee802154_air::fcs(body) == u16::from_le_bytes([tail[0], tail[1]]);
                fifo.push_back((SIGNAL_DBM + RSSI_OFFSET_DB) as i8 as u8);
                fifo.push_back(if crc_ok { 0x80 } else { 0 } | RX_CORRELATION);
            } else {
                fifo.extend(&f.mpdu[n.saturating_sub(2)..]);
            }
            drop(fifo);
            self.regs[(RFIRQF0 / 4) as usize] |=
                RFIRQF0_SFD | RFIRQF0_FIFOP | RFIRQF0_FRAME_ACCEPTED | RFIRQF0_RXPKTDONE;
        }
    }

    /// `us` microseconds in CPU cycles. With no cycle clock attached, time
    /// does not advance, so every delay collapses to 0 and transitions are
    /// immediate.
    fn cycles(&self, us: u64) -> u64 {
        if self.clock.is_none() {
            return 0;
        }
        (u128::from(us) * u128::from(self.clock_hz)).div_ceil(1_000_000) as u64
    }

    /// The burst still on the air at `now`, if any.
    fn tx_at(&self, now: u64) -> Option<TxBurst> {
        self.tx.filter(|b| now < b.end)
    }

    /// When the receiver's current calibration started, if RX is on at `now`
    /// (a burst still on the air holds RX off; RX recalibrates after it).
    fn rx_start_at(&self, now: u64) -> Option<u64> {
        if self.rxenable == 0 || self.tx_at(now).is_some() {
            return None;
        }
        Some(match self.tx {
            Some(b) => self.rx_since.max(b.end),
            None => self.rx_since,
        })
    }

    fn fsm_state(&self, now: u64) -> u32 {
        if let Some(b) = self.tx_at(now) {
            return if now < b.start + self.cycles(TURNAROUND_US) {
                FSM_TX_CALIBRATION
            } else {
                FSM_TX
            };
        }
        match self.rx_start_at(now) {
            Some(s) if now < s + self.cycles(TURNAROUND_US) => FSM_RX_CALIBRATION,
            Some(_) => FSM_SFD_WAIT,
            None => FSM_IDLE,
        }
    }

    fn rssi_valid(&self, now: u64) -> bool {
        self.rx_start_at(now)
            .is_some_and(|s| now >= s + self.cycles(TURNAROUND_US + RSSI_SETTLE_US))
    }

    fn rssi_raw(&self, now: u64) -> u32 {
        if self.rssi_valid(now) {
            let dbm = if self.air_signal(now).is_some() {
                SIGNAL_DBM
            } else {
                NOISE_FLOOR_DBM
            };
            ((dbm + RSSI_OFFSET_DB) as i8 as u8).into()
        } else {
            RSSI_INVALID
        }
    }

    /// CCA: RSSI valid and below `CCACTRL0.CCA_THR` (both two's complement).
    fn cca(&self, now: u64) -> bool {
        self.rssi_valid(now) && (self.rssi_raw(now) as u8 as i8) < (self.reg(CCACTRL0) as u8 as i8)
    }

    fn fsmstat1(&self, now: u64) -> u32 {
        let state = self.fsm_state(now);
        let mut v = 0;
        if self.tx_at(now).is_some() {
            v |= FSMSTAT1_TX_ACTIVE;
        }
        if matches!(state, FSM_RX_CALIBRATION | FSM_SFD_WAIT) {
            v |= FSMSTAT1_RX_ACTIVE;
        }
        if matches!(state, FSM_SFD_WAIT | FSM_TX) {
            v |= FSMSTAT1_LOCK_STATUS;
        }
        if self.cca(now) {
            v |= FSMSTAT1_CCA | FSMSTAT1_SAMPLED_CCA;
        }
        if !self.rx_fifo.borrow().is_empty() {
            v |= FSMSTAT1_FIFO | FSMSTAT1_FIFOP;
        }
        let own_sfd = self
            .tx_at(now)
            .is_some_and(|b| now >= b.start + self.cycles(TURNAROUND_US + SHR_BYTES * BYTE_US));
        let rx_sfd = state == FSM_SFD_WAIT
            && self
                .air_signal(now)
                .is_some_and(|(s, _)| self.ns(now) >= s + ieee802154_air::SFD_NS);
        if own_sfd || rx_sfd {
            v |= FSMSTAT1_SFD;
        }
        v
    }

    /// One LFSR step (x^16 + x^14 + x^13 + x^11 + 1); the two low bits stand
    /// in for the I and Q ADC LSBs.
    fn next_rnd(&self) -> u32 {
        let x = self.lfsr.get();
        let bit = (x ^ (x >> 2) ^ (x >> 3) ^ (x >> 5)) & 1;
        let x = (x >> 1) | (bit << 15);
        self.lfsr.set(x);
        u32::from(x & 0b11)
    }

    fn read_reg(&self, off: u64) -> u32 {
        let now = self.now();
        match off {
            RXENABLE => self.rxenable,
            FSMSTAT0 => {
                let state = self.fsm_state(now);
                let cal = if state == FSM_IDLE {
                    0
                } else {
                    FSMSTAT0_CAL_DONE
                };
                cal | state
            }
            FSMSTAT1 => self.fsmstat1(now),
            RSSI => self.rssi_raw(now),
            RSSISTAT => u32::from(self.rssi_valid(now)),
            TXFIFOCNT => self.tx_fifo.len() as u32,
            RXFIFOCNT => self.rx_fifo.borrow().len() as u32,
            RXFIRST => self.rx_fifo.borrow().front().copied().map_or(0, u32::from),
            FREQEST => 0,
            RFRND => self.next_rnd(),
            MTCTRL => {
                let v = self.reg(MTCTRL);
                if v & MTCTRL_RUN != 0 {
                    v | MTCTRL_STATE
                } else {
                    v
                }
            }
            // Pops the RX FIFO (0 once empty; RX underflow is not flagged).
            RFDATA => self.rx_fifo.borrow_mut().pop_front().map_or(0, u32::from),
            o if (o as usize) < RFCORE_SIZE => self.reg(o),
            _ => 0,
        }
    }

    fn start_tx(&mut self, now: u64) {
        if self.tx_fifo.is_empty() {
            self.regs[(RFERRF / 4) as usize] |= RFERRF_TXUNDERF;
            return;
        }
        let fcs = if self.reg(FRMCTRL0) & FRMCTRL0_AUTOCRC != 0 {
            FCS_BYTES
        } else {
            0
        };
        let on_air = (SHR_BYTES + self.tx_fifo.len() as u64 + fcs) * BYTE_US;
        let start = now;
        let end = start + self.cycles(TURNAROUND_US) + self.cycles(on_air);
        self.tx = Some(TxBurst { start, end });
        self.tx_done_flagged = false;
        self.last_tx_frame = self.tx_fifo.clone();
        if let Some((air, node)) = &self.air {
            // The PHR length counts the FCS; the FIFO holds the rest.
            let len = (self.tx_fifo[0] & 0x7F) as usize;
            let body_len = len.saturating_sub(fcs as usize);
            let mut mpdu: Vec<u8> = self.tx_fifo[1..].iter().take(body_len).copied().collect();
            if fcs != 0 {
                let c = ieee802154_air::fcs(&mpdu);
                mpdu.extend_from_slice(&c.to_le_bytes());
            }
            let first_bit = self.ns(start + self.cycles(TURNAROUND_US));
            air.transmit(*node, self.channel(), mpdu, first_bit);
        }
        if self.reg(FRMCTRL1) & FRMCTRL1_SET_RXENMASK_ON_TX != 0 {
            self.rxenable |= 1 << 6;
        }
    }

    fn strobe(&mut self, cmd: u8) {
        let now = self.now();
        match cmd {
            ISRXON => {
                // Aborts TX and forces an RX calibration (§23.9.1).
                self.tx = None;
                self.rxenable |= 1 << 7;
                self.rx_since = now;
            }
            ISTXON => self.start_tx(now),
            ISTXONCCA if self.cca(now) => self.start_tx(now),
            ISRFOFF => {
                self.rxenable = 0;
                self.tx = None;
            }
            ISFLUSHTX => self.tx_fifo.clear(),
            ISFLUSHRX => self.rx_fifo.borrow_mut().clear(),
            // CSP program instructions and the other strobes: not modelled.
            _ => {}
        }
    }

    fn write_reg(&mut self, off: u64, value: u32) {
        let byte = value & 0xFF;
        match off {
            RFST => {
                self.regs[(RFST / 4) as usize] = byte;
                self.strobe(byte as u8);
            }
            RFDATA => {
                if self.tx_fifo.len() < TXFIFO_BYTES {
                    self.tx_fifo.push(byte as u8);
                } else {
                    self.regs[(RFERRF / 4) as usize] |= RFERRF_TXOVERF;
                }
            }
            RXMASKSET => self.rxenable |= byte,
            RXMASKCLR => self.rxenable &= !byte,
            MTCTRL => self.regs[(MTCTRL / 4) as usize] = byte & MTCTRL_RW,
            RFIRQF0 | RFIRQF1 => self.regs[(off / 4) as usize] = byte,
            // Read-only status and computed registers.
            RXENABLE | FSMSTAT0 | FSMSTAT1 | RSSI | RSSISTAT | RXFIRST | RXFIFOCNT | TXFIFOCNT
            | RFRND | FREQEST => {}
            o if CSPPROG.contains(&o) => {}
            o if (o as usize) < RFCORE_SIZE => self.regs[(o / 4) as usize] = value,
            _ => {}
        }
    }
}

impl Default for Cc2538RfCore {
    fn default() -> Self {
        Self::new(DEFAULT_CLOCK_HZ)
    }
}

impl Peripheral for Cc2538RfCore {
    /// State is a function of the bus cycle clock, evaluated on access; it
    /// raises no IRQ, so there is nothing for the per-cycle walk to do.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(lane_read(self.read_reg(offset & !3), offset))
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        // A debugger read must not pop the RX FIFO.
        if offset & !3 == RFDATA {
            return Some(self.rx_fifo.borrow().front().copied().unwrap_or(0));
        }
        Some(lane_read(self.read_reg(offset & !3), offset))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        match reg {
            // Strobes and the FIFO port act on the byte written, once.
            RFST | RFDATA if offset % 4 == 0 => self.write_reg(reg, u32::from(value)),
            RFST | RFDATA => {}
            _ => {
                let merged = lane_merge(self.read_reg(reg), offset, value);
                self.write_reg(reg, merged);
            }
        }
        if reg != RFDATA {
            self.publish_filter();
        }
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        if offset & !3 != RFDATA {
            self.publish_filter();
        }
        Ok(())
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn attach_ieee802154_air(&mut self, air: Ieee802154AirBus) {
        self.air = air.register().map(|node| (air, node));
        self.publish_filter();
    }

    /// On an air: receive frames and raise `TXDONE` while RX is on, a burst
    /// is in flight or the RF interrupt is asserted.
    fn needs_bus_tick(&self) -> bool {
        self.air.is_some()
            && (self.rxenable != 0
                || (self.tx.is_some() && !self.tx_done_flagged)
                || self.irq_level())
    }

    /// Frames from other nodes must keep arriving while the CPU sleeps in
    /// WFI, so idle fast-forward is capped to the bus-tick poll quantum while
    /// the receiver is on.
    fn idle_poll_bus_tick(&self) -> bool {
        self.air.is_some() && self.rxenable != 0
    }

    fn tick_with_bus(&mut self, _bus: &mut dyn Bus) {
        let now = self.now();
        if let Some(b) = self.tx {
            if !self.tx_done_flagged && now >= b.end {
                self.tx_done_flagged = true;
                self.regs[(RFIRQF1 / 4) as usize] |= RFIRQF1_TXDONE;
            }
        }
        self.receive(now);
    }

    fn irq_line_level(&self) -> Option<bool> {
        self.air.as_ref().map(|_| self.irq_level())
    }

    fn on_node_reset(&mut self) {
        let clock = self.clock.take();
        let air = self.air.take();
        *self = Self::new(self.clock_hz);
        self.clock = clock;
        self.air = air;
        self.publish_filter();
    }

    snapshot_impl!(Cc2538RfCore, "Cc2538RfCore", keep[clock, air]);
}

#[cfg(test)]
mod tests {
    use super::*;

    const US: u64 = DEFAULT_CLOCK_HZ / 1_000_000;

    fn radio() -> (Cc2538RfCore, CycleClock) {
        let clock = CycleClock::default();
        let mut rf = Cc2538RfCore::new(DEFAULT_CLOCK_HZ);
        rf.attach_cycle_clock(clock.clone());
        (rf, clock)
    }

    fn state(rf: &Cc2538RfCore) -> u32 {
        rf.read_u32(FSMSTAT0).unwrap() & 0x3F
    }

    #[test]
    fn resets_to_table_values_and_idle() {
        let (rf, _) = radio();
        assert_eq!(rf.read_u32(FRMCTRL0).unwrap(), 0x40);
        assert_eq!(rf.read_u32(0x63C).unwrap(), 0x0B, "FREQCTRL");
        assert_eq!(rf.read_u32(CCACTRL0).unwrap(), 0xF8);
        assert_eq!(rf.read_u32(0x700).unwrap(), 0xD0, "CSPPROG0");
        assert_eq!(rf.read_u32(RFST).unwrap(), 0xD0);
        assert_eq!(rf.read_u32(MTCTRL).unwrap(), 0x02);
        assert_eq!(rf.read_u32(FSMSTAT0).unwrap(), 0);
        assert_eq!(rf.read_u32(FSMSTAT1).unwrap(), 0);
        assert_eq!(rf.read_u32(RSSI).unwrap(), RSSI_INVALID);
        assert_eq!(rf.read_u32(RSSISTAT).unwrap(), 0);
    }

    #[test]
    fn isrxon_calibrates_then_waits_for_sfd_with_a_clear_channel() {
        let (mut rf, clock) = radio();
        rf.write_u32(RFST, u32::from(ISRXON)).unwrap();
        assert_eq!(rf.read_u32(RXENABLE).unwrap(), 0x80);
        assert_eq!(state(&rf), FSM_RX_CALIBRATION);
        assert_eq!(rf.read_u32(RSSISTAT).unwrap(), 0);
        clock.publish(192 * US);
        assert_eq!(state(&rf), FSM_SFD_WAIT);
        assert_eq!(
            rf.read_u32(RSSISTAT).unwrap(),
            0,
            "RSSI settles 8 symbols later"
        );
        clock.publish((192 + 128) * US);
        assert_eq!(rf.read_u32(RSSISTAT).unwrap(), 1);
        // -100 dBm noise floor: raw -27, under the -8 (0xF8) CCA threshold.
        assert_eq!(rf.read_u32(RSSI).unwrap(), 0xE5);
        let s1 = rf.read_u32(FSMSTAT1).unwrap();
        assert_ne!(s1 & FSMSTAT1_CCA, 0);
        assert_ne!(s1 & FSMSTAT1_RX_ACTIVE, 0);
        assert_eq!(s1 & FSMSTAT1_TX_ACTIVE, 0);
        rf.write_u32(RFST, u32::from(ISRFOFF)).unwrap();
        assert_eq!(state(&rf), FSM_IDLE);
        assert_eq!(rf.read_u32(RXENABLE).unwrap(), 0);
    }

    #[test]
    fn a_raised_cca_threshold_below_the_noise_reports_busy() {
        let (mut rf, clock) = radio();
        rf.write_u32(CCACTRL0, 0xC0).unwrap(); // raw -64 < -27
        rf.write_u32(RFST, u32::from(ISRXON)).unwrap();
        clock.publish(1_000 * US);
        assert_eq!(rf.read_u32(FSMSTAT1).unwrap() & FSMSTAT1_CCA, 0);
    }

    #[test]
    fn istxon_holds_tx_active_for_turnaround_plus_frame_then_returns_to_rx() {
        let (mut rf, clock) = radio();
        rf.write_u32(RFST, u32::from(ISRXON)).unwrap();
        rf.write_u32(RFST, u32::from(ISFLUSHTX)).unwrap();
        // PHR length 12 (10 payload + 2 FCS), then 10 payload bytes.
        rf.write_u32(RFDATA, 12).unwrap();
        for b in 0..10u32 {
            rf.write_u32(RFDATA, b).unwrap();
        }
        assert_eq!(rf.read_u32(TXFIFOCNT).unwrap(), 11);
        clock.publish(10_000 * US);
        rf.write_u32(RFST, u32::from(ISTXON)).unwrap();
        let t0 = 10_000 * US;
        assert_ne!(rf.read_u32(FSMSTAT1).unwrap() & FSMSTAT1_TX_ACTIVE, 0);
        assert_eq!(state(&rf), FSM_TX_CALIBRATION);
        clock.publish(t0 + 192 * US);
        assert_eq!(state(&rf), FSM_TX);
        // 5 SHR + 11 FIFO + 2 FCS = 18 bytes = 576 µs after the turnaround.
        clock.publish(t0 + (192 + 576) * US - 1);
        assert_ne!(rf.read_u32(FSMSTAT1).unwrap() & FSMSTAT1_TX_ACTIVE, 0);
        clock.publish(t0 + (192 + 576) * US);
        assert_eq!(rf.read_u32(FSMSTAT1).unwrap() & FSMSTAT1_TX_ACTIVE, 0);
        // SET_RXENMASK_ON_TX (reset 1) keeps RX on: back to RX calibration.
        assert_eq!(state(&rf), FSM_RX_CALIBRATION);
        assert_eq!(rf.last_tx_frame().len(), 11);
        assert_eq!(rf.last_tx_frame()[0], 12);
    }

    #[test]
    fn tx_with_an_empty_fifo_underflows() {
        let (mut rf, _) = radio();
        rf.write_u32(RFST, u32::from(ISTXON)).unwrap();
        assert_eq!(state(&rf), FSM_IDLE);
        assert_ne!(rf.read_u32(RFERRF).unwrap() & RFERRF_TXUNDERF, 0);
    }

    #[test]
    fn rxmask_set_and_clear_drive_rxenable() {
        let (mut rf, _) = radio();
        rf.write_u32(RXMASKSET, 0x20).unwrap();
        assert_eq!(rf.read_u32(RXENABLE).unwrap(), 0x20);
        assert_eq!(state(&rf), FSM_RX_CALIBRATION);
        rf.write_u32(RXMASKCLR, 0x20).unwrap();
        assert_eq!(state(&rf), FSM_IDLE);
    }

    #[test]
    fn mac_timer_state_follows_run() {
        let (mut rf, _) = radio();
        rf.write_u32(MTCTRL, 0x03).unwrap();
        assert_ne!(rf.read_u32(MTCTRL).unwrap() & MTCTRL_STATE, 0);
        rf.write_u32(MTCTRL, 0x02).unwrap();
        assert_eq!(rf.read_u32(MTCTRL).unwrap() & MTCTRL_STATE, 0);
    }

    #[test]
    fn rfrnd_is_deterministic_and_not_constant() {
        let (a, _) = radio();
        let (b, _) = radio();
        let sa: Vec<u32> = (0..32).map(|_| a.read_u32(RFRND).unwrap()).collect();
        let sb: Vec<u32> = (0..32).map(|_| b.read_u32(RFRND).unwrap()).collect();
        assert_eq!(sa, sb);
        assert!(sa.iter().any(|&v| v != sa[0]));
        assert!(sa.iter().all(|&v| v <= 0b11));
    }

    /// Two radios on one air with Contiki-NG's settings: PAN 0xABCD,
    /// extended address `id`, AUTOCRC + AUTOACK, FIFOP unmasked, RX on.
    fn on_air(air: &Ieee802154AirBus, clock: &CycleClock, id: u8) -> Cc2538RfCore {
        let mut rf = Cc2538RfCore::new(DEFAULT_CLOCK_HZ);
        rf.attach_cycle_clock(clock.clone());
        rf.attach_ieee802154_air(air.clone());
        rf.write_u32(FRMCTRL0, FRMCTRL0_AUTOCRC | FRMCTRL0_AUTOACK)
            .unwrap();
        rf.write_u32(PAN_ID0, 0xCD).unwrap();
        rf.write_u32(PAN_ID1, 0xAB).unwrap();
        rf.write_u32(EXT_ADDR0, u32::from(id)).unwrap();
        rf.write_u32(RFIRQM0, RFIRQF0_FIFOP).unwrap();
        rf.write_u32(RFST, u32::from(ISRXON)).unwrap();
        rf
    }

    /// Data frame to `dst` from `src` (extended, PAN-ID compressed), AR set.
    fn send(rf: &mut Cc2538RfCore, dst: u8, src: u8, seq: u8, payload: &[u8]) {
        let fcf: u16 = 0x0001 | (1 << 5) | (1 << 6) | (3 << 10) | (1 << 12) | (3 << 14);
        let mut mhr = fcf.to_le_bytes().to_vec();
        mhr.push(seq);
        mhr.extend_from_slice(&0xABCDu16.to_le_bytes());
        mhr.extend_from_slice(&[dst, 0, 0, 0, 0, 0, 0, 0]);
        mhr.extend_from_slice(&[src, 0, 0, 0, 0, 0, 0, 0]);
        mhr.extend_from_slice(payload);
        rf.write_u32(RFST, u32::from(ISFLUSHTX)).unwrap();
        rf.write_u32(RFDATA, mhr.len() as u32 + 2).unwrap();
        for b in mhr {
            rf.write_u32(RFDATA, u32::from(b)).unwrap();
        }
        rf.write_u32(RFST, u32::from(ISTXON)).unwrap();
    }

    fn tick(rf: &mut Cc2538RfCore) {
        let mut bus = crate::bus::SystemBus::new();
        rf.tick_with_bus(&mut bus);
    }

    #[test]
    fn a_unicast_frame_reaches_the_addressee_fifo_and_is_auto_acked() {
        let air = Ieee802154AirBus::new();
        let clock = CycleClock::default();
        let mut a = on_air(&air, &clock, 1);
        let mut b = on_air(&air, &clock, 2);
        let mut c = on_air(&air, &clock, 3);
        assert!(b.needs_bus_tick() && b.idle_poll_bus_tick());
        clock.publish(1_000 * US);
        send(&mut a, 2, 1, 0x5A, b"hi");
        // MHR = FCF 2 + seq 1 + PAN 2 + dst 8 + src 8 = 21; MPDU = 21 + 2 + FCS 2.
        let first_bit = 1_000 + 192;
        let end = first_bit + (6 + 25) * 32;
        clock.publish((first_bit + 5 * 32) * US);
        assert_ne!(
            b.read_u32(FSMSTAT1).unwrap() & FSMSTAT1_SFD,
            0,
            "SFD while the frame is on the air"
        );
        assert_eq!(b.read_u32(FSMSTAT1).unwrap() & FSMSTAT1_CCA, 0, "busy");
        clock.publish(end * US - 1);
        tick(&mut b);
        assert_eq!(b.read_u32(RXFIFOCNT).unwrap(), 0, "not before the last bit");
        clock.publish(end * US);
        tick(&mut b);
        tick(&mut c);
        assert_eq!(c.read_u32(RXFIFOCNT).unwrap(), 0, "filtered: not for c");
        // Length byte (MPDU incl. FCS), 23 body bytes, RSSI, CRC_OK|corr.
        assert_eq!(b.read_u32(RXFIFOCNT).unwrap(), 1 + 23 + 2);
        let s1 = b.read_u32(FSMSTAT1).unwrap();
        assert_ne!(s1 & FSMSTAT1_FIFOP, 0);
        assert_ne!(s1 & FSMSTAT1_FIFO, 0);
        assert_eq!(b.irq_line_level(), Some(true), "FIFOP unmasked");
        let frame: Vec<u8> = (0..26).map(|_| b.read_u32(RFDATA).unwrap() as u8).collect();
        assert_eq!(frame[0], 25);
        assert_eq!(frame[3], 0x5A, "sequence number");
        assert_eq!(&frame[22..24], b"hi");
        assert_eq!(frame[24] as i8, (SIGNAL_DBM + RSSI_OFFSET_DB) as i8);
        assert_eq!(frame[25], 0x80 | RX_CORRELATION, "CRC_OK");
        assert_eq!(b.read_u32(FSMSTAT1).unwrap() & FSMSTAT1_FIFOP, 0);
        b.write_u32(RFIRQF0, 0).unwrap();
        assert_eq!(b.irq_line_level(), Some(false));
        // The ACK from b, 192 µs after the frame, back at a.
        let ack_end = end + 192 + (6 + 5) * 32;
        clock.publish(ack_end * US);
        tick(&mut a);
        assert_eq!(a.read_u32(RXFIFOCNT).unwrap(), 1 + 3 + 2);
        let ack: Vec<u8> = (0..4).map(|_| a.read_u32(RFDATA).unwrap() as u8).collect();
        assert_eq!(ack, vec![5, 0x02, 0x00, 0x5A]);
        // a's own frame never comes back to a.
        a.write_u32(RFST, u32::from(ISFLUSHRX)).unwrap();
        assert_eq!(a.read_u32(RXFIFOCNT).unwrap(), 0);
        assert_ne!(a.read_u32(RFIRQF1).unwrap() & RFIRQF1_TXDONE, 0);
    }

    #[test]
    fn a_receiver_that_is_off_or_on_another_channel_hears_nothing() {
        let air = Ieee802154AirBus::new();
        let clock = CycleClock::default();
        let mut a = on_air(&air, &clock, 1);
        let mut b = on_air(&air, &clock, 2);
        let mut c = on_air(&air, &clock, 3);
        b.write_u32(RFST, u32::from(ISRFOFF)).unwrap();
        assert!(!b.needs_bus_tick() && !b.idle_poll_bus_tick());
        c.write_u32(FREQCTRL, 0x0B + 5).unwrap();
        clock.publish(1_000 * US);
        send(&mut a, 2, 1, 1, b"x");
        send(&mut a, 3, 1, 2, b"x");
        clock.publish(100_000 * US);
        tick(&mut b);
        tick(&mut c);
        assert_eq!(b.read_u32(RXFIFOCNT).unwrap(), 0);
        assert_eq!(c.read_u32(RXFIFOCNT).unwrap(), 0);
        // No ACK either: nobody was listening on the channel.
        tick(&mut a);
        assert_eq!(a.read_u32(RXFIFOCNT).unwrap(), 0);
    }

    #[test]
    fn without_an_air_no_irq_line_is_driven() {
        let (rf, _) = radio();
        assert_eq!(rf.irq_line_level(), None);
        assert!(!rf.needs_bus_tick());
    }

    #[test]
    fn without_a_clock_transitions_are_immediate() {
        let mut rf = Cc2538RfCore::new(DEFAULT_CLOCK_HZ);
        rf.write_u32(RFST, u32::from(ISRXON)).unwrap();
        assert_eq!(state(&rf), FSM_SFD_WAIT);
        assert_eq!(rf.read_u32(RSSISTAT).unwrap(), 1);
    }
}
