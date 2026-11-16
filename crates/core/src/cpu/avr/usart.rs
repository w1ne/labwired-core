// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The four USARTs of the ATmega2560, in asynchronous mode: Arduino
//! `Serial`..`Serial3`. (The ATmega1281 shares the chapter but keeps the
//! 328P USART0 model for now.)
//!
//! Microchip DS2549, chapter 22 (USART) and the register summary:
//!
//! | USART | UCSRnA | UCSRnB | UCSRnC | UBRRnL/H | UDRn | RX / UDRE / TX vector |
//! |-------|--------|--------|--------|----------|------|-----------------------|
//! | 0     | 0xC0   | 0xC1   | 0xC2   | 0xC4/C5  | 0xC6 | 26 / 27 / 28          |
//! | 1     | 0xC8   | 0xC9   | 0xCA   | 0xCC/CD  | 0xCE | 37 / 38 / 39          |
//! | 2     | 0xD0   | 0xD1   | 0xD2   | 0xD4/D5  | 0xD6 | 52 / 53 / 54          |
//! | 3     | 0x130  | 0x131  | 0x132  | 0x134/35 | 0x136| 55 / 56 / 57          |
//!
//! Vector numbers are Table 14-1 slots (1-based).
//!
//! What is modelled:
//! - Baud: one bit is 16 (8 with `U2Xn`) clocks per `UBRRn + 1` (Table 22-1).
//! - Frame: start + 5..9 data bits (`UCSZn2:0`) + parity (`UPMn1`) + one or
//!   two stop bits (`USBSn`).
//! - Transmitter: `UDRn` is a one-byte buffer in front of the shift register.
//!   `UDREn` is set while the buffer is empty; a byte written while the shift
//!   register is idle (and `TXENn` is set) moves into it at once, so a second
//!   byte can be written right away. `TXCn` is set when a frame's last stop
//!   bit has left and the buffer is empty; it is cleared by writing one to it
//!   or by entering the TX-complete vector.
//! - Receiver: a two-character FIFO behind the shift register. A character
//!   whose stop bit arrives while the FIFO is full waits in the shift
//!   register; the start bit of the next one then overruns it: the waiting
//!   character is lost and `DORn` is reported with the next character read.
//!   `FEn`, `DORn`, `UPEn` and `RXB8n` belong to the character at the head
//!   of the FIFO, so they are read before `UDRn`. Clearing `RXENn` flushes
//!   the FIFO.
//! - Interrupts: RX complete (`RXCIEn`, level on `RXCn`), data register empty
//!   (`UDRIEn`, level on `UDREn`) and TX complete (`TXCIEn`, on `TXCn`).
//!
//! The host side is the chip descriptor's `uart` window at
//! `AVR_IO_MIRROR_BASE + UCSRnA` (`profile: avr`): the receiver takes host
//! bytes from its queue one frame at a time, and a transmitted frame is
//! written to its `UDR` offset once its stop bit has left, where the host
//! sink and stream peers see it. The host line is 8N1 at the programmed
//! baud: a receiver set to another format reads those bits with its own
//! frame, so a mismatched format shows up as `FEn`/`UPEn`/`RXB8n` the way it
//! does on a wire. USART0 also feeds the CPU's console (Arduino `Serial`),
//! as the 328P model does.
//!
//! Not modelled: synchronous and master SPI modes (`UMSELn`), multi-processor
//! communication mode (`MPCMn` is stored only), receiver clock recovery
//! (a host baud other than the programmed one), and the transmitter's ninth
//! bit on the host side (the host line carries 8 bits).

use super::{Avr, AvrModel, AVR_IO_MIRROR_BASE};
use crate::{Bus, SimResult};

/// UCSRnA data-space address of USART0..3.
const BASES: [u16; 4] = [0xC0, 0xC8, 0xD0, 0x130];
/// Pending bit of each USART's RX-complete vector; UDRE and TX follow.
/// USART0 keeps the 328P numbering (19..21) that `mega2560_vector` maps to
/// slots 26..28; USART1..3 use their Table 14-1 slot, which passes through.
const VEC_RX: [u32; 4] = [19, 37, 52, 55];

const OFF_UCSRA: u16 = 0;
const OFF_UCSRB: u16 = 1;
const OFF_UCSRC: u16 = 2;
const OFF_UBRRL: u16 = 4;
const OFF_UBRRH: u16 = 5;
const OFF_UDR: u16 = 6;

// UCSRnA
const RXC: u8 = 1 << 7;
const TXC: u8 = 1 << 6;
const UDRE: u8 = 1 << 5;
const FE: u8 = 1 << 4;
const DOR: u8 = 1 << 3;
const UPE: u8 = 1 << 2;
const U2X: u8 = 1 << 1;
const MPCM: u8 = 1 << 0;
// UCSRnB
const RXCIE: u8 = 1 << 7;
const TXCIE: u8 = 1 << 6;
const UDRIE: u8 = 1 << 5;
const RXEN: u8 = 1 << 4;
const TXEN: u8 = 1 << 3;
const UCSZ2: u8 = 1 << 2;
const RXB8: u8 = 1 << 1;
// UCSRnC
const UPM1: u8 = 1 << 5;
const UPM0: u8 = 1 << 4;
const USBS: u8 = 1 << 3;
/// UCSRnC reset value: 8-bit characters (UCSZn1:0 = 11).
const UCSRC_RESET: u8 = 0x06;

/// Steps between looks at the host queue while the receiver is idle.
const HOST_POLL_STEPS: u8 = 32;
/// Bits of one host frame: the host line is 8N1.
const HOST_FRAME_BITS: u64 = 10;

/// A character as the receiver assembled it.
#[derive(Debug, Clone, Copy, Default)]
struct RxChar {
    data: u8,
    bit8: bool,
    fe: bool,
    upe: bool,
    dor: bool,
}

#[derive(Debug, Clone)]
struct Channel {
    /// `U2Xn` and `MPCMn` as written.
    ucsra: u8,
    txc: bool,
    /// `UCSRnB` as written; `RXB8n` comes from the FIFO head.
    ucsrb: u8,
    ucsrc: u8,
    ubrr: u16,
    /// The transmit buffer (`UDREn` clear while it holds a byte).
    tx_buf: Option<u8>,
    /// The frame in the transmit shift register and the cycle it ends.
    tx_shift: Option<(u8, u64)>,
    /// Frames whose stop bit has left, not yet handed to the host.
    tx_out: Vec<u8>,
    /// The receive FIFO, head first (two characters at most).
    rx_fifo: [Option<RxChar>; 2],
    /// The frame in the receive shift register and the cycle its stop bit
    /// is sampled. Past that cycle it is a complete character waiting for
    /// room in the FIFO.
    rx_shift: Option<(RxChar, u64)>,
    /// Cycle at which the line can carry the next start bit.
    rx_line_free: u64,
    /// Last cycle the host queue was seen empty: a byte found later arrived
    /// after it.
    rx_empty_seen: u64,
    /// An overrun lost a character; the next one into the FIFO carries DOR.
    rx_lost: bool,
}

impl Default for Channel {
    fn default() -> Self {
        Self {
            ucsra: 0,
            txc: false,
            ucsrb: 0,
            ucsrc: UCSRC_RESET,
            ubrr: 0,
            tx_buf: None,
            tx_shift: None,
            tx_out: Vec::new(),
            rx_fifo: [None; 2],
            rx_shift: None,
            rx_line_free: 0,
            rx_empty_seen: 0,
            rx_lost: false,
        }
    }
}

impl Channel {
    fn bit_clocks(&self) -> u64 {
        let per = if self.ucsra & U2X != 0 { 8 } else { 16 };
        (u64::from(self.ubrr) + 1) * per
    }

    /// Data bits of the frame (`UCSZn2:0`; the reserved codes read as 8).
    fn data_bits(&self) -> u32 {
        let code = (self.ucsrb & UCSZ2) | ((self.ucsrc >> 1) & 0x03);
        match code {
            0..=3 => 5 + u32::from(code),
            7 => 9,
            _ => 8,
        }
    }

    fn parity(&self) -> bool {
        self.ucsrc & UPM1 != 0
    }

    fn tx_frame_clocks(&self) -> u64 {
        let stop = if self.ucsrc & USBS != 0 { 2 } else { 1 };
        let bits = 1 + u64::from(self.data_bits()) + u64::from(self.parity()) + stop;
        bits * self.bit_clocks()
    }

    /// Clocks from the start bit to the (first) stop bit the receiver
    /// samples.
    fn rx_frame_clocks(&self) -> u64 {
        let bits = 1 + u64::from(self.data_bits()) + u64::from(self.parity()) + 1;
        bits * self.bit_clocks()
    }

    /// Read a host 8N1 frame carrying `byte` with this receiver's format:
    /// line bit `i` after the start bit is data bit `i` for `i < 8`, the
    /// host's stop bit at 8, and idle (1) after it.
    fn decode(&self, byte: u8) -> RxChar {
        let line = |i: u32| if i < 8 { (byte >> i) & 1 } else { 1 };
        let n = self.data_bits();
        let mut ones = 0;
        for i in 0..n {
            ones += u32::from(line(i));
        }
        let mut pos = n;
        let mut upe = false;
        if self.parity() {
            let odd = self.ucsrc & UPM0 != 0;
            let want = (ones & 1 == 1) != odd;
            upe = (line(pos) == 1) != want;
            pos += 1;
        }
        let data = if n >= 8 {
            byte
        } else {
            byte & ((1u8 << n) - 1)
        };
        RxChar {
            data,
            bit8: n == 9 && line(8) == 1,
            fe: line(pos) == 0,
            upe,
            dor: false,
        }
    }

    fn fifo_push(&mut self, mut c: RxChar) {
        c.dor = std::mem::take(&mut self.rx_lost);
        if self.rx_fifo[0].is_none() {
            self.rx_fifo[0] = Some(c);
        } else {
            self.rx_fifo[1] = Some(c);
        }
    }

    fn fifo_full(&self) -> bool {
        self.rx_fifo[1].is_some()
    }

    fn fifo_pop(&mut self) -> Option<RxChar> {
        let head = self.rx_fifo[0].take();
        self.rx_fifo[0] = self.rx_fifo[1].take();
        head
    }

    /// Bring the transmitter up to cycle `now`.
    fn tx_advance(&mut self, now: u64) {
        loop {
            match self.tx_shift {
                Some((byte, end)) if end <= now => {
                    self.tx_out.push(byte);
                    // A buffered byte follows back to back; one written while
                    // the transmitter was being disabled still goes out.
                    match self.tx_buf.take() {
                        Some(next) => self.tx_shift = Some((next, end + self.tx_frame_clocks())),
                        None => {
                            self.tx_shift = None;
                            self.txc = true;
                            return;
                        }
                    }
                }
                Some(_) => return,
                None => {
                    if self.ucsrb & TXEN == 0 {
                        return;
                    }
                    match self.tx_buf.take() {
                        Some(next) => self.tx_shift = Some((next, now + self.tx_frame_clocks())),
                        None => return,
                    }
                }
            }
        }
    }

    /// Bring the receiver up to cycle `now`, taking host bytes from the
    /// window at `host` (UCSRnA mirror) as the line becomes free.
    fn rx_advance(&mut self, now: u64, bus: &dyn Bus, host: Option<u64>) -> SimResult<()> {
        if self.ucsrb & RXEN == 0 {
            return Ok(());
        }
        loop {
            if let Some((c, end)) = self.rx_shift {
                if end > now {
                    return Ok(());
                }
                if !self.fifo_full() {
                    self.fifo_push(c);
                    self.rx_shift = None;
                    continue;
                }
                // Complete and waiting: only a new start bit changes that.
            }
            let start = self.rx_line_free.max(self.rx_empty_seen);
            if start > now {
                return Ok(());
            }
            let Some(base) = host else {
                return Ok(());
            };
            if bus.read_u8(base + u64::from(OFF_UCSRA))? & RXC == 0 {
                self.rx_empty_seen = now;
                return Ok(());
            }
            let byte = bus.read_u8(base + u64::from(OFF_UDR))?;
            if self.rx_shift.is_some() {
                // Two characters in the FIFO, one waiting in the shift
                // register, and a new start bit: the waiting one is lost.
                self.rx_lost = true;
            }
            let c = self.decode(byte);
            let bit = self.bit_clocks();
            self.rx_shift = Some((c, start + self.rx_frame_clocks()));
            self.rx_line_free = start + self.rx_frame_clocks().max(HOST_FRAME_BITS * bit);
        }
    }

    fn sync(&mut self, now: u64, bus: &dyn Bus, host: Option<u64>) -> SimResult<()> {
        self.tx_advance(now);
        self.rx_advance(now, bus, host)
    }

    fn read(&mut self, off: u16) -> u8 {
        match off {
            OFF_UCSRA => {
                let mut v = self.ucsra & (U2X | MPCM);
                if let Some(head) = self.rx_fifo[0] {
                    v |= RXC;
                    v |= if head.fe { FE } else { 0 };
                    v |= if head.dor { DOR } else { 0 };
                    v |= if head.upe { UPE } else { 0 };
                }
                if self.txc {
                    v |= TXC;
                }
                if self.tx_buf.is_none() {
                    v |= UDRE;
                }
                v
            }
            OFF_UCSRB => {
                let bit8 = self.rx_fifo[0].is_some_and(|c| c.bit8);
                (self.ucsrb & !RXB8) | if bit8 { RXB8 } else { 0 }
            }
            OFF_UCSRC => self.ucsrc,
            OFF_UBRRL => self.ubrr as u8,
            OFF_UBRRH => (self.ubrr >> 8) as u8,
            _ => self.fifo_pop().map_or(0, |c| c.data),
        }
    }

    fn write(&mut self, off: u16, value: u8, now: u64) {
        match off {
            OFF_UCSRA => {
                self.ucsra = value & (U2X | MPCM);
                if value & TXC != 0 {
                    self.txc = false;
                }
            }
            OFF_UCSRB => {
                let was = self.ucsrb;
                self.ucsrb = value & !RXB8;
                if was & RXEN != 0 && value & RXEN == 0 {
                    // "The receiver buffer FIFO will be flushed when the
                    // receiver is disabled."
                    self.rx_fifo = [None; 2];
                    self.rx_shift = None;
                    self.rx_lost = false;
                } else if was & RXEN == 0 && value & RXEN != 0 {
                    self.rx_line_free = now;
                }
            }
            OFF_UCSRC => self.ucsrc = value,
            OFF_UBRRL => self.ubrr = (self.ubrr & 0x0F00) | u16::from(value),
            OFF_UBRRH => self.ubrr = (self.ubrr & 0x00FF) | (u16::from(value & 0x0F) << 8),
            _ => {
                // Written only while UDREn is set; otherwise the byte is lost.
                if self.tx_buf.is_none() {
                    self.tx_buf = Some(value);
                }
            }
        }
    }

    /// The RX / UDRE / TX interrupt levels, as bits 0..2.
    fn irq_levels(&self) -> u64 {
        let rx = self.ucsrb & RXCIE != 0 && self.rx_fifo[0].is_some();
        let udre = self.ucsrb & UDRIE != 0 && self.tx_buf.is_none();
        let tx = self.ucsrb & TXCIE != 0 && self.txc;
        u64::from(rx) | (u64::from(udre) << 1) | (u64::from(tx) << 2)
    }

    /// Something can still change without firmware touching a register.
    fn live(&self) -> bool {
        self.ucsrb & (RXEN | RXCIE | TXCIE | UDRIE) != 0
            || self.tx_shift.is_some()
            || self.tx_buf.is_some()
            || !self.tx_out.is_empty()
    }
}

/// The USARTs of one part.
#[derive(Debug, Default)]
pub(super) struct MegaUsarts {
    ch: [Channel; 4],
    count: usize,
    /// Steps since the host queues were last looked at.
    poll: u8,
}

impl MegaUsarts {
    pub(super) fn new(model: AvrModel) -> Self {
        Self {
            count: usart_count(model),
            ..Self::default()
        }
    }

    pub(super) fn reset(&mut self) {
        let count = self.count;
        *self = Self {
            count,
            ..Self::default()
        };
    }

    fn locate(&self, addr: u16) -> Option<(usize, u16)> {
        locate(self.count, addr)
    }

    fn any_live(&self) -> bool {
        self.ch[..self.count].iter().any(Channel::live)
    }

    /// The mask of every USART vector's pending bit.
    fn vector_mask(&self) -> u64 {
        VEC_RX[..self.count]
            .iter()
            .fold(0, |m, &v| m | (0b111u64 << v))
    }
}

/// USARTs this module models on `model`.
fn usart_count(model: AvrModel) -> usize {
    match model {
        AvrModel::Atmega2560 => 4,
        _ => 0,
    }
}

/// The USART and register offset of data-space `addr`, if it is a register
/// of one of the first `count` USARTs (offset 3 is reserved).
fn locate(count: usize, addr: u16) -> Option<(usize, u16)> {
    BASES[..count].iter().enumerate().find_map(|(n, &base)| {
        let off = addr.wrapping_sub(base);
        (off <= OFF_UDR && off != 3).then_some((n, off))
    })
}

fn host_window(bus: &dyn Bus, n: usize) -> Option<u64> {
    let base = AVR_IO_MIRROR_BASE + u64::from(BASES[n]);
    bus.has_mmio_window(base).then_some(base)
}

impl Avr {
    /// Whether data-space `addr` is a register of this part's USARTs.
    #[inline]
    pub(super) fn is_mega_usart_reg(&self, addr: u16) -> bool {
        matches!(addr, 0xC0..=0xD6 | 0x130..=0x136)
            && locate(usart_count(self.model), addr).is_some()
    }

    pub(super) fn mega_usart_read(&self, addr: u16, bus: &dyn Bus) -> SimResult<u8> {
        let mut u = self.musart.lock().expect("USART state");
        let Some((n, off)) = u.locate(addr) else {
            return Ok(0);
        };
        let host = host_window(bus, n);
        let ch = &mut u.ch[n];
        ch.sync(self.cycles, bus, host)?;
        let v = ch.read(off);
        if off == OFF_UDR {
            // The FIFO moved along: a character waiting in the shift
            // register takes the freed place.
            ch.sync(self.cycles, bus, host)?;
        }
        Ok(v)
    }

    pub(super) fn mega_usart_write(
        &mut self,
        addr: u16,
        value: u8,
        bus: &mut dyn Bus,
    ) -> SimResult<()> {
        let now = self.cycles;
        let u = self.musart.get_mut().expect("USART state");
        let Some((n, off)) = u.locate(addr) else {
            return Ok(());
        };
        let host = host_window(bus, n);
        let ch = &mut u.ch[n];
        ch.sync(now, bus, host)?;
        ch.write(off, value, now);
        ch.sync(now, bus, host)?;
        // The host window keeps the baud (UBRRn, U2Xn) and enables for its
        // peers; UDRn reaches it when the frame has been sent.
        if let Some(base) = host {
            if off != OFF_UDR {
                bus.write_u8(base + u64::from(off), value)?;
            }
        }
        self.mega_usart_flush(bus)
    }

    /// Hand finished frames to the host windows (and USART0's to the CPU
    /// console, as Arduino `Serial`).
    fn mega_usart_flush(&mut self, bus: &mut dyn Bus) -> SimResult<()> {
        let u = self.musart.get_mut().expect("USART state");
        for n in 0..u.count {
            if u.ch[n].tx_out.is_empty() {
                continue;
            }
            let out = std::mem::take(&mut u.ch[n].tx_out);
            let host = host_window(bus, n);
            for byte in out {
                let mut on_bus = false;
                if let Some(base) = host {
                    bus.write_u8(base + u64::from(OFF_UDR), byte)?;
                    // A byte an RS-485 transceiver put on its bus is a frame,
                    // not console text: the window says so at +7.
                    on_bus = n == 0 && bus.read_u8(base + 7)? & 1 != 0;
                }
                if n == 0 {
                    self.serial_tx.push(byte);
                    if !on_bus {
                        if let Some(sink) = &self.serial_sink {
                            if let Ok(mut g) = sink.lock() {
                                g.push(byte);
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Once per step: advance frames that end by now, look at the host
    /// queues of idle receivers, deliver finished frames and follow the
    /// interrupt levels.
    pub(super) fn mega_usart_service(&mut self, bus: &mut dyn Bus) -> SimResult<()> {
        let now = self.cycles;
        let sleeping = self.sleeping;
        let u = self.musart.get_mut().expect("USART state");
        if u.count == 0 || !u.any_live() {
            return Ok(());
        }
        u.poll = u.poll.wrapping_add(1);
        let poll_host = sleeping || u.poll % HOST_POLL_STEPS == 0;
        let mut want = 0u64;
        let count = u.count;
        for (n, (ch, &vec)) in u.ch.iter_mut().zip(VEC_RX.iter()).enumerate().take(count) {
            let tx_due = ch.tx_shift.is_some_and(|(_, end)| end <= now)
                || (ch.tx_buf.is_some() && ch.tx_shift.is_none());
            let rx_due = ch.ucsrb & RXEN != 0
                && match ch.rx_shift {
                    Some((_, end)) => end <= now && (poll_host || !ch.fifo_full()),
                    None => poll_host,
                };
            if tx_due || rx_due {
                let host = host_window(bus, n);
                ch.sync(now, bus, host)?;
            }
            want |= ch.irq_levels() << vec;
        }
        let mask = u.vector_mask();
        self.pending_irq = (self.pending_irq & !mask) | want;
        self.mega_usart_flush(bus)
    }

    /// Entering a TX-complete vector clears that USART's `TXCn`.
    pub(super) fn mega_usart_vector_entered(&mut self, vec: u32) {
        let u = self.musart.get_mut().expect("USART state");
        if let Some(n) = VEC_RX[..u.count].iter().position(|&v| vec == v + 2) {
            u.ch[n].txc = false;
            self.pending_irq &= !(1u64 << vec);
        }
    }

    /// A USART is busy or may receive: a sleeping core is not skipped ahead.
    pub(super) fn mega_usart_keeps_awake(&self) -> bool {
        self.musart.lock().map(|u| u.any_live()).unwrap_or(false)
    }

    pub(super) fn mega_usart_reset(&mut self) {
        if let Ok(u) = self.musart.get_mut() {
            u.reset();
        }
    }
}
