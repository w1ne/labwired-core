//! TI CC2538 2.4-GHz IEEE 802.15.4 RF core (`0x4008_8000`), radio alone in an
//! empty channel.
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
//! NOT modelled: reception of any kind (there is no RF medium: `RXFIFOCNT`
//! stays 0, `FIFO`/`FIFOP`/`SFD` never set, nothing is ever received, and a
//! sent frame goes nowhere), frame filtering and source matching, the CSP
//! (`RFST` program instructions other than the immediate strobes above), the
//! MAC timer count (`MTM*`/`MTMOVF*` are storage), RF interrupts (no
//! `RFIRQF*`/`RFERRF` flag raises an IRQ, and `TXDONE`/`RXPKTDONE` are never
//! set), uDMA requests, the analog front end (`FREQCTRL`/`TXPOWER`/AGC are
//! storage) and calibration failures.

use super::{lane_merge, lane_read};
use crate::{CycleClock, Peripheral, SimResult};
use std::cell::Cell;

/// Window size (4 KiB block; the documented registers end at `0x83C`).
pub const RFCORE_SIZE: usize = 0x1000;

// XREG (Table 23-9), offsets in the 4-KiB window.
const FRMCTRL0: u64 = 0x624;
const FRMCTRL1: u64 = 0x628;
const RXENABLE: u64 = 0x62C;
const RXMASKSET: u64 = 0x630;
const RXMASKCLR: u64 = 0x634;
const FSMSTAT0: u64 = 0x648;
const FSMSTAT1: u64 = 0x64C;
const CCACTRL0: u64 = 0x658;
const RSSI: u64 = 0x660;
const RSSISTAT: u64 = 0x664;
const RXFIRST: u64 = 0x668;
const RXFIFOCNT: u64 = 0x66C;
const TXFIFOCNT: u64 = 0x670;
const RFRND: u64 = 0x69C;
const FREQEST: u64 = 0x6A8;
// SFR (Table 23-10 / 12-1).
const MTCTRL: u64 = 0x804;
const RFDATA: u64 = 0x828;
const RFERRF: u64 = 0x82C;
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

const FRMCTRL0_AUTOCRC: u32 = 1 << 6;
const FRMCTRL1_SET_RXENMASK_ON_TX: u32 = 1 << 0;
const FSMSTAT0_CAL_DONE: u32 = 1 << 7;
const FSMSTAT1_CCA: u32 = 1 << 4;
const FSMSTAT1_SAMPLED_CCA: u32 = 1 << 3;
const FSMSTAT1_LOCK_STATUS: u32 = 1 << 2;
const FSMSTAT1_TX_ACTIVE: u32 = 1 << 1;
const FSMSTAT1_RX_ACTIVE: u32 = 1 << 0;
const RFERRF_TXUNDERF: u32 = 1 << 5;
const RFERRF_TXOVERF: u32 = 1 << 4;
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
    tx_fifo: Vec<u8>,
    last_tx_frame: Vec<u8>,
    lfsr: Cell<u16>,
    #[serde(skip)]
    clock: Option<CycleClock>,
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
            tx_fifo: Vec::new(),
            last_tx_frame: Vec::new(),
            lfsr: Cell::new(LFSR_SEED),
            clock: None,
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
            ((NOISE_FLOOR_DBM + RSSI_OFFSET_DB) as i8 as u8).into()
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
            RXFIRST | RXFIFOCNT | FREQEST => 0,
            RFRND => self.next_rnd(),
            MTCTRL => {
                let v = self.reg(MTCTRL);
                if v & MTCTRL_RUN != 0 {
                    v | MTCTRL_STATE
                } else {
                    v
                }
            }
            // The RX FIFO is always empty.
            RFDATA => 0,
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
        self.last_tx_frame = self.tx_fifo.clone();
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
            ISFLUSHRX => {}
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
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn on_node_reset(&mut self) {
        let clock = self.clock.take();
        *self = Self::new(self.clock_hz);
        self.clock = clock;
    }

    snapshot_impl!(Cc2538RfCore, "Cc2538RfCore", keep[clock]);
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

    #[test]
    fn without_a_clock_transitions_are_immediate() {
        let mut rf = Cc2538RfCore::new(DEFAULT_CLOCK_HZ);
        rf.write_u32(RFST, u32::from(ISRXON)).unwrap();
        assert_eq!(state(&rf), FSM_SFD_WAIT);
        assert_eq!(rf.read_u32(RSSISTAT).unwrap(), 1);
    }
}
