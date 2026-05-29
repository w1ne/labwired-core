// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 RADIO peripheral.
//!
//! Source: nRF52840 PS rev 1.7 §6.20 (RADIO). 2.4 GHz BLE/802.15.4/proprietary
//! transceiver. The hardware is enormous; this model implements:
//!
//! * **Register surface** — every documented register in 0x000–0x77C
//!   round-trips with proper masks and reset values.
//! * **Task / event state machine** — TASKS_{TX,RX}EN → STATE=TXIDLE/RXIDLE +
//!   EVENTS_READY; TASKS_START → EVENTS_END after the packet "transmits"
//!   (instant); TASKS_DISABLE → STATE=DISABLED + EVENTS_DISABLED. Enough
//!   that BLE-stack init code that polls EVENTS_READY before sending the
//!   next task does not spin.
//! * **SHORTS** for the common chain patterns (READY→START,
//!   END→DISABLE, ADDRESS→RSSISTART, DISABLED→TXEN/RXEN/RSSISTOP).
//! * **Easy DMA**: PACKETPTR is a pointer to a buffer in RAM; on
//!   TASKS_START in TX mode, real silicon DMAs the buffer to the air;
//!   we don't do air, but we mark EVENTS_ADDRESS, EVENTS_PAYLOAD,
//!   EVENTS_END in sequence so firmware progresses through its state
//!   machine.
//! * **CRCSTATUS** = 1 (OK) after every receive in this model; no
//!   actual CRC is computed.
//!
//! What is **not** modeled:
//! * Actual RF transmission / reception (no air, no peer).
//! * Whitening, BLE address resolution (deferred to AAR).
//! * Cyclic-bit-rate accuracy. EVENTS_END fires the next tick after
//!   TASKS_START regardless of MODE/DATARATE.

use crate::{Bus, Peripheral, PeripheralTickResult, SimResult};
use std::collections::VecDeque;

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
}

impl Nrf52Radio {
    pub fn new() -> Self {
        Self {
            // Reset values per PS table 226.
            state: STATE_DISABLED,
            frequency: 0,
            mode: 0,           // Nrf_1Mbit
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

    /// Apply SHORTS-style automatic task triggers when an event fires.
    fn apply_event_shorts(&mut self, fired: u64) {
        match fired {
            OFF_EVENTS_READY => {
                if self.shorts & SHORT_READY_START != 0 {
                    self.start_packet();
                }
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
        self.pending_address = true;
        self.pending_payload = true;
        self.pending_end = true;
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
            OFF_EVENTS_DEVMATCH | OFF_EVENTS_DEVMISS | OFF_EVENTS_RSSIEND
            | OFF_EVENTS_BCMATCH | OFF_EVENTS_CRCERROR | OFF_EVENTS_FRAMESTART
            | OFF_EVENTS_EDEND | OFF_EVENTS_EDSTOPPED | OFF_EVENTS_CCAIDLE
            | OFF_EVENTS_CCABUSY | OFF_EVENTS_CCASTOPPED | OFF_EVENTS_RATEBOOST
            | OFF_EVENTS_MHRMATCH | OFF_EVENTS_SYNC => 0,
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
            OFF_TASKS_TXEN => {
                if value & 1 != 0 {
                    self.tx_enable();
                }
            }
            OFF_TASKS_RXEN => {
                if value & 1 != 0 {
                    self.rx_enable();
                }
            }
            OFF_TASKS_START => {
                if value & 1 != 0 {
                    self.start_packet();
                }
            }
            OFF_TASKS_STOP => {
                if value & 1 != 0 {
                    if self.state == STATE_TX {
                        self.state = STATE_TXIDLE;
                    } else if self.state == STATE_RX {
                        self.state = STATE_RXIDLE;
                    }
                }
            }
            OFF_TASKS_DISABLE => {
                if value & 1 != 0 {
                    self.disable();
                }
            }
            OFF_TASKS_RSSISTART | OFF_TASKS_RSSISTOP | OFF_TASKS_BCSTART
            | OFF_TASKS_BCSTOP | OFF_TASKS_EDSTART | OFF_TASKS_EDSTOP
            | OFF_TASKS_CCASTART | OFF_TASKS_CCASTOP => {}

            OFF_EVENTS_READY => self.events_ready = value & 1,
            OFF_EVENTS_ADDRESS => self.events_address = value & 1,
            OFF_EVENTS_PAYLOAD => self.events_payload = value & 1,
            OFF_EVENTS_END => self.events_end = value & 1,
            OFF_EVENTS_DISABLED => self.events_disabled = value & 1,
            OFF_EVENTS_CRCOK => self.events_crcok = value & 1,
            OFF_EVENTS_TXREADY => self.events_txready = value & 1,
            OFF_EVENTS_RXREADY => self.events_rxready = value & 1,
            OFF_EVENTS_PHYEND => self.events_phyend = value & 1,

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
                header.push(bus.read_u8((base as u64).wrapping_add(i as u64)).unwrap_or(0));
            }

            // LENGTH is the byte at offset s0len (right after S0).
            let length = if desc.lflen == 0 {
                desc.statlen
            } else {
                let len_byte = header
                    .get(desc.s0len as usize)
                    .copied()
                    .unwrap_or(0);
                // Mask to LFLEN bits.
                let mask = if desc.lflen >= 8 {
                    0xFFu8
                } else {
                    ((1u16 << desc.lflen) - 1) as u8
                };
                (len_byte & mask).min(desc.maxlen).saturating_add(desc.statlen)
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

            self.last_tx_packet = Some(packet);
        }

        // ── RX Easy DMA ──────────────────────────────────────────────────
        // Pop the next packet from rx_inbox, strip the trailing CRC and
        // verify it (in whitened form, since the air-side carries the
        // whitened frame). De-whiten the payload, write back to RAM at
        // PACKETPTR + payload_offset; copy header bytes verbatim.
        if self.pending_rx_dma {
            self.pending_rx_dma = false;
            if let Some(mut pkt) = self.rx_inbox.pop_front() {
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
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // Single tick walks the whole cascade: TXEN → READY (SHORT_READY_START)
        // → START → END (SHORT_END_DISABLE) → DISABLE → DISABLED. This is the
        // sim-side collapse of an effect that takes microseconds on real
        // silicon but is observably the same end state for firmware.
        r.tick();
        assert_eq!(r.read_u32(OFF_EVENTS_READY).unwrap(), 1);
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
        r.write_u32(OFF_MODE, 0x3).unwrap();       // BLE_1Mbit
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
        use crate::bus::SystemBus;
        use crate::Bus;

        // TX in one instance, RX in another — verify the payload survives
        // a round trip through whitening + CRC.
        let payload_orig = vec![0xC0, 0xFF, 0xEE, 0x42, 0x69];
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

        assert_eq!(rx.crc_status, 1, "CRC must verify after whitening round-trip");
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
}
