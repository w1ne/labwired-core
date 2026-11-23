// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Infineon CYW43439 Wi-Fi/Bluetooth combo chip on its gSPI host bus — the
//! radio of the Raspberry Pi Pico W and Pico 2 W.
//!
//! On those boards the host talks to the chip over a half-duplex SPI that
//! the pico-sdk drives with a PIO program (`cyw43_bus_pio_spi.pio`): one
//! data pin (`DIO`, GP24, also the host-wake interrupt), one clock (GP29,
//! side-set) and a chip select (GP25, plain SIO GPIO). The board LED hangs
//! off the chip's own `WL_GPIO0`, so `digitalWrite(LED_BUILTIN)` on a Pico W
//! is a full chip bring-up: firmware download, then a `gpioout` iovar.
//!
//! # What is modelled
//!
//! * **The wire.** The model follows the clock line of whichever PIO block
//!   drives it and the chip-select pad. On each rising clock edge it samples
//!   `DIO` while the host drives it, and after a read command presents the
//!   next response bit on `DIO` (gSPI high-speed mode: the device launches
//!   data on the rising edge, the host samples on the falling one). Bytes
//!   go out MSB first, in the order the host's DMA hands them over, so the
//!   16-bit start-up word order (`SWAP32` in the driver) and the 32-bit
//!   mode both fall out of the same shift register.
//! * **F0, the gSPI bus registers** (`cyw43_spi.h`): bus control (word
//!   length), the `0xFEEDBEAD` test register, interrupt status/enable
//!   (`F2_PACKET_AVAILABLE` is live while a frame waits), the status word
//!   (`F2_RX_READY`, packet available and its length), function info and the
//!   per-function response delays. The F1 response delay is honoured: a
//!   backplane read returns that many pad bytes before the data.
//! * **F1, the backplane.** The 32 KiB window registers, the chip clock CSR
//!   (ALP always available, HT once the Arm core runs), the sleep CSR
//!   (`KEEP_SDIO_ON` reads back with `DEVICE_ON`), the Arm and SOCSRAM core
//!   wrappers (`IOCTRL`, `RESETCTRL`; both cores start in reset), the 512 KiB
//!   SOCSRAM the firmware is downloaded into, and a register file for the
//!   rest.
//! * **F2, the WLAN function.** Once the host releases the WLAN Arm core with
//!   a firmware image at RAM 0 and the NVRAM length token in the last RAM
//!   word, `F2_RX_READY` rises and the chip answers SDPCM control frames:
//!   each ioctl is answered with the same command, length and id, status 0,
//!   and bus-data credit for the next frames. `gpioout` drives `WL_GPIO0..2`;
//!   `cur_etheraddr` reads the configured MAC; `clmload_status` reads 0.
//!
//! # What is not modelled
//!
//! The firmware image is **not executed**: the chip's own Cortex-M3 and
//! radio are out of scope. Only the host interface of a running firmware is
//! modelled, so there is no scan, no join, no data traffic (data frames are
//! counted and dropped) and no async events. Bluetooth (F3) is absent. Other
//! `GET` ioctls answer zeros of the requested length. GPIO interrupts on the
//! `DIO` pad see a host-wake change at the next SIO pad update, not at once.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};
use crate::peripherals::pad_lines::{LineListener, PadLines};

/// `TEST_PATTERN` at `SPI_READ_TEST_REGISTER` (cyw43_spi.h).
const TEST_PATTERN: u32 = 0xFEED_BEAD;

// F0 register byte offsets (cyw43_spi.h).
const SPI_BUS_CONTROL: u32 = 0x00;
const SPI_INTERRUPT_REGISTER: u32 = 0x04;
const SPI_INTERRUPT_ENABLE_REGISTER: u32 = 0x06;
const SPI_STATUS_REGISTER: u32 = 0x08;
const SPI_FUNCTION1_INFO: u32 = 0x0C;
const SPI_FUNCTION2_INFO: u32 = 0x0E;
const SPI_READ_TEST_REGISTER: u32 = 0x14;
const SPI_RESP_DELAY_F1: u32 = 0x1D;

const WORD_LENGTH_32: u8 = 0x01;
const INTERRUPT_POLARITY_HIGH: u8 = 0x20;

const F2_PACKET_AVAILABLE: u16 = 0x0020;
const STATUS_F2_RX_READY: u32 = 0x0000_0020;
const STATUS_F2_PKT_AVAILABLE: u32 = 0x0000_0100;
const STATUS_F2_PKT_LEN_SHIFT: u32 = 9;
const SPI_FUNCTIONX_ENABLED: u16 = 1 << 0;
const SPI_FUNCTIONX_READY: u16 = 1 << 1;

// F1 SDIO-core registers (cyw43_ll.c).
const SDIO_BACKPLANE_ADDRESS_LOW: u32 = 0x1000A;
const SDIO_BACKPLANE_ADDRESS_MID: u32 = 0x1000B;
const SDIO_BACKPLANE_ADDRESS_HIGH: u32 = 0x1000C;
const SDIO_CHIP_CLOCK_CSR: u32 = 0x1000E;
const SDIO_SLEEP_CSR: u32 = 0x1001F;
const SBSDIO_ALP_AVAIL: u8 = 0x40;
const SBSDIO_HT_AVAIL: u8 = 0x80;
const SBSDIO_SLPCSR_KEEP_SDIO_ON: u8 = 1 << 0;
const SBSDIO_SLPCSR_DEVICE_ON: u8 = 1 << 1;
/// F1 addresses below this are a backplane window access.
const F1_REGISTER_SPACE: u32 = 0x1_0000;
const BACKPLANE_ADDR_MASK: u32 = 0x7FFF;

// Backplane map (cyw43_ll.c).
const CHIPCOMMON_BASE_ADDRESS: u32 = 0x1800_0000;
const WLAN_ARMCM3_WRAPPER: u32 = 0x1800_3000 + 0x10_0000;
const SOCSRAM_WRAPPER: u32 = 0x1800_4000 + 0x10_0000;
const AI_IOCTRL_OFFSET: u32 = 0x408;
const AI_RESETCTRL_OFFSET: u32 = 0x800;
const AIRC_RESET: u32 = 1;
const SICF_CPUHALT: u32 = 0x0020;
const SICF_CLOCK_EN: u32 = 0x0001;
/// SOCSRAM size the driver downloads into (`CYW43_RAM_SIZE`, 512 KiB).
const RAM_SIZE: usize = 512 * 1024;
/// Chip ID word at CHIPCOMMON offset 0: the part number in bits [15:0].
/// The revision and package fields above it are not modelled (read 0).
const CHIP_ID_43439: u32 = 43439;

// SDPCM (cyw43_ll.c).
const SDPCM_HEADER_LEN: usize = 12;
const CONTROL_HEADER: u8 = 0;
const DATA_HEADER: u8 = 2;
const IOCTL_HEADER_LEN: usize = 16;
const WLC_GET_VAR: u32 = 262;
const WLC_SET_VAR: u32 = 263;
/// Credit window granted past the last sequence number received. The driver
/// accepts a credit step of up to 20 (`sdpcm_process_rx_packet`).
const CREDIT_WINDOW: u8 = 8;

const DEFAULT_MAC: [u8; 6] = [0x28, 0xCD, 0xC1, 0x00, 0x00, 0x01];

fn dbg_trace(msg: &str) {
    use std::io::Write;
    if let Ok(path) = std::env::var("LABWIRED_CYW43_TRACE") {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(f, "{msg}");
        }
    }
}

/// gSPI command word fields (`make_cmd` in cyw43_bus_pio_spi.c).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Command {
    write: bool,
    incr: bool,
    function: u8,
    addr: u32,
    len: u32,
}

impl Command {
    fn decode(word: u32) -> Self {
        let len = word & 0x7FF;
        Self {
            write: word & (1 << 31) != 0,
            incr: word & (1 << 30) != 0,
            function: ((word >> 28) & 0x3) as u8,
            addr: (word >> 11) & 0x1_FFFF,
            // A size field of 0 means 2048 bytes (gSPI F2 packet length).
            len: if len == 0 { 0x800 } else { len },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Command,
    WriteData(Command),
    ReadData,
}

/// The chip: gSPI shift engine, register spaces and the firmware's host
/// interface. Pure state — the listeners below feed it edges.
#[derive(Debug)]
pub struct Cyw43439Chip {
    selected: bool,
    phase: Phase,
    shift: u8,
    bits: u8,
    /// Bytes clocked in this transaction (command word first).
    rx: Vec<u8>,
    /// Response bytes and the next bit to present.
    tx: Vec<u8>,
    tx_bit: usize,
    /// Level the chip drives on DIO while it answers a read.
    dio_out: bool,

    f0: [u8; 0x20],
    int_status: u16,

    bp_window: u32,
    f1_regs: HashMap<u32, u8>,
    clock_csr: u8,
    sleep_csr: u8,
    ram: Vec<u8>,
    backplane: HashMap<u32, u32>,
    wlan_ioctrl: u32,
    wlan_resetctrl: u32,
    socram_ioctrl: u32,
    socram_resetctrl: u32,
    firmware_running: bool,

    /// Frames the firmware has for the host (F2 reads pop them).
    to_host: VecDeque<Vec<u8>>,
    device_seq: u8,
    last_host_seq: Option<u8>,
    gpioout: u32,
    mac: [u8; 6],

    /// Counters for inspection and tests.
    pub ioctls: u64,
    pub data_frames_dropped: u64,
    pub led_edges: u64,
}

impl Default for Cyw43439Chip {
    fn default() -> Self {
        Self::new(DEFAULT_MAC)
    }
}

impl Cyw43439Chip {
    pub fn new(mac: [u8; 6]) -> Self {
        Self {
            selected: false,
            phase: Phase::Idle,
            shift: 0,
            bits: 0,
            rx: Vec::new(),
            tx: Vec::new(),
            tx_bit: 0,
            dio_out: false,
            f0: [0; 0x20],
            int_status: 0,
            bp_window: 0,
            f1_regs: HashMap::new(),
            clock_csr: 0,
            sleep_csr: 0,
            ram: vec![0; RAM_SIZE],
            backplane: HashMap::new(),
            wlan_ioctrl: 0,
            wlan_resetctrl: AIRC_RESET,
            socram_ioctrl: 0,
            socram_resetctrl: AIRC_RESET,
            firmware_running: false,
            to_host: VecDeque::new(),
            device_seq: 0,
            last_host_seq: None,
            gpioout: 0,
            mac,
            ioctls: 0,
            data_frames_dropped: 0,
            led_edges: 0,
        }
    }

    /// `WL_GPIO0..2` output levels (`gpioout`).
    pub fn wl_gpio(&self, n: u8) -> bool {
        self.gpioout & (1 << n) != 0
    }

    pub fn firmware_running(&self) -> bool {
        self.firmware_running
    }

    fn word32(&self) -> bool {
        self.f0[SPI_BUS_CONTROL as usize] & WORD_LENGTH_32 != 0
    }

    /// Host-wake: the level the chip holds on DIO while deselected. Active
    /// while an enabled interrupt is pending, at the configured polarity.
    pub fn host_wake(&self) -> bool {
        // Unconfigured, the chip leaves the line to the board pull-down.
        if self.f0[SPI_BUS_CONTROL as usize] == 0 {
            return false;
        }
        let enable = u16::from_le_bytes([
            self.f0[SPI_INTERRUPT_ENABLE_REGISTER as usize],
            self.f0[SPI_INTERRUPT_ENABLE_REGISTER as usize + 1],
        ]);
        let pending = self.interrupts() & enable != 0;
        let active_high = self.f0[SPI_BUS_CONTROL as usize] & INTERRUPT_POLARITY_HIGH != 0;
        if active_high {
            pending
        } else {
            !pending
        }
    }

    fn interrupts(&self) -> u16 {
        let mut v = self.int_status;
        if !self.to_host.is_empty() {
            v |= F2_PACKET_AVAILABLE;
        }
        v
    }

    /// Chip select moved. Low starts a transaction; high ends it (a write is
    /// applied then) and returns DIO to the host-wake level.
    pub fn chip_select(&mut self, low: bool) {
        if low {
            self.selected = true;
            self.phase = Phase::Command;
            self.shift = 0;
            self.bits = 0;
            self.rx.clear();
            self.tx.clear();
            self.tx_bit = 0;
        } else {
            if self.selected {
                if let Phase::WriteData(cmd) = self.phase {
                    let data: Vec<u8> = self.rx[4..].to_vec();
                    self.apply_write(cmd, &data);
                }
            }
            self.selected = false;
            self.phase = Phase::Idle;
        }
    }

    /// Rising clock edge with the host's DIO level `dio` (meaningful while
    /// the host drives it). Returns the level the chip now presents on DIO,
    /// or `None` while it does not drive.
    pub fn clock_rise(&mut self, dio: bool) -> Option<bool> {
        match self.phase {
            Phase::Idle => None,
            Phase::Command | Phase::WriteData(_) => {
                self.shift = (self.shift << 1) | u8::from(dio);
                self.bits += 1;
                if self.bits == 8 {
                    self.rx.push(self.shift);
                    self.shift = 0;
                    self.bits = 0;
                    if self.phase == Phase::Command && self.rx.len() == 4 {
                        self.command_complete();
                    }
                }
                None
            }
            Phase::ReadData => {
                let byte = self.tx.get(self.tx_bit / 8).copied().unwrap_or(0);
                self.dio_out = byte & (0x80 >> (self.tx_bit % 8)) != 0;
                self.tx_bit += 1;
                Some(self.dio_out)
            }
        }
    }

    fn command_complete(&mut self) {
        let b = [self.rx[0], self.rx[1], self.rx[2], self.rx[3]];
        // 32-bit mode: the host's little-endian word, byte by byte. 16-bit
        // start-up mode: two 16-bit words, low word first, each MSB first.
        let word = if self.word32() {
            u32::from_le_bytes(b)
        } else {
            u32::from_le_bytes([b[1], b[0], b[3], b[2]])
        };
        let cmd = Command::decode(word);
        dbg_trace(&format!("cmd {word:08x} {cmd:?} w32={}", self.word32()));
        if cmd.write {
            self.phase = Phase::WriteData(cmd);
            return;
        }
        let mut out = Vec::new();
        if cmd.function == 1 {
            out.resize(usize::from(self.f0[SPI_RESP_DELAY_F1 as usize]), 0);
        }
        let data = self.read(cmd);
        if !self.word32() && cmd.function == 0 && data.len() == 4 {
            out.extend_from_slice(&[data[1], data[0], data[3], data[2]]);
        } else {
            out.extend_from_slice(&data);
        }
        dbg_trace(&format!("  read -> {:02x?}", &out[..out.len().min(40)]));
        self.tx = out;
        self.tx_bit = 0;
        self.phase = Phase::ReadData;
    }

    fn read(&mut self, cmd: Command) -> Vec<u8> {
        let len = cmd.len as usize;
        match cmd.function {
            0 => {
                let v = self.f0_read(cmd.addr);
                let mut out = v.to_le_bytes().to_vec();
                out.resize(len.max(4).next_multiple_of(4), 0);
                out
            }
            1 => {
                let mut out = Vec::with_capacity(len.next_multiple_of(4));
                for i in 0..len {
                    let addr = if cmd.incr {
                        cmd.addr + i as u32
                    } else {
                        cmd.addr
                    };
                    out.push(self.f1_read_byte(addr));
                }
                out.resize(len.next_multiple_of(4), 0);
                out
            }
            2 => {
                let mut frame = self.to_host.pop_front().unwrap_or_default();
                frame.resize(len.next_multiple_of(4), 0);
                frame
            }
            _ => vec![0; len.next_multiple_of(4)],
        }
    }

    fn apply_write(&mut self, cmd: Command, data: &[u8]) {
        if cmd.function != 1 || cmd.len <= 4 {
            dbg_trace(&format!("  write {:02x?}", &data[..data.len().min(32)]));
        }
        let len = (cmd.len as usize).min(data.len());
        match cmd.function {
            0 => {
                let word = if self.word32() {
                    data.get(..4)
                        .map(|d| u32::from_le_bytes([d[0], d[1], d[2], d[3]]))
                } else {
                    data.get(..4)
                        .map(|d| u32::from_le_bytes([d[1], d[0], d[3], d[2]]))
                };
                if let Some(word) = word {
                    self.f0_write(cmd.addr, &word.to_le_bytes()[..len.min(4)]);
                }
            }
            1 => {
                for (i, &byte) in data[..len].iter().enumerate() {
                    let addr = if cmd.incr {
                        cmd.addr + i as u32
                    } else {
                        cmd.addr
                    };
                    self.f1_write_byte(addr, byte);
                }
            }
            2 => self.host_frame(&data[..len]),
            _ => {}
        }
    }

    fn f0_read(&self, addr: u32) -> u32 {
        match addr {
            SPI_READ_TEST_REGISTER => TEST_PATTERN,
            SPI_INTERRUPT_REGISTER => {
                u32::from(self.interrupts())
                    | (u32::from(self.f0[SPI_INTERRUPT_ENABLE_REGISTER as usize]) << 16)
                    | (u32::from(self.f0[SPI_INTERRUPT_ENABLE_REGISTER as usize + 1]) << 24)
            }
            SPI_STATUS_REGISTER => self.status(),
            SPI_FUNCTION1_INFO => {
                let f1 = SPI_FUNCTIONX_ENABLED | SPI_FUNCTIONX_READY | (64 << 2);
                let f2 = if self.firmware_running {
                    SPI_FUNCTIONX_ENABLED | SPI_FUNCTIONX_READY | (2048 << 2)
                } else {
                    0
                };
                u32::from(f1) | (u32::from(f2) << 16)
            }
            SPI_FUNCTION2_INFO => u32::from(if self.firmware_running {
                SPI_FUNCTIONX_ENABLED | SPI_FUNCTIONX_READY
            } else {
                0
            }),
            a if (a as usize) < self.f0.len() => {
                let a = a as usize;
                let b = |i: usize| self.f0.get(a + i).copied().unwrap_or(0);
                u32::from_le_bytes([b(0), b(1), b(2), b(3)])
            }
            _ => 0,
        }
    }

    fn status(&self) -> u32 {
        let mut v = 0;
        if self.firmware_running {
            v |= STATUS_F2_RX_READY;
        }
        if let Some(frame) = self.to_host.front() {
            v |=
                STATUS_F2_PKT_AVAILABLE | ((frame.len() as u32 & 0x7FF) << STATUS_F2_PKT_LEN_SHIFT);
        }
        v
    }

    fn f0_write(&mut self, addr: u32, bytes: &[u8]) {
        for (i, &b) in bytes.iter().enumerate() {
            let a = addr + i as u32;
            match a {
                // Interrupt status is write-1-to-clear.
                0x04 => self.int_status &= !u16::from(b),
                0x05 => self.int_status &= !(u16::from(b) << 8),
                // Status, function info and the test register are read-only.
                0x08..=0x17 => {}
                a if (a as usize) < self.f0.len() => self.f0[a as usize] = b,
                _ => {}
            }
        }
    }

    fn backplane_addr(&self, addr: u32) -> u32 {
        (self.bp_window & !BACKPLANE_ADDR_MASK) | (addr & BACKPLANE_ADDR_MASK)
    }

    fn f1_read_byte(&mut self, addr: u32) -> u8 {
        if addr >= F1_REGISTER_SPACE {
            return match addr {
                SDIO_CHIP_CLOCK_CSR => {
                    let mut v = self.clock_csr | SBSDIO_ALP_AVAIL;
                    if self.firmware_running {
                        v |= SBSDIO_HT_AVAIL;
                    }
                    v
                }
                SDIO_SLEEP_CSR => {
                    let mut v = self.sleep_csr;
                    if v & SBSDIO_SLPCSR_KEEP_SDIO_ON != 0 {
                        v |= SBSDIO_SLPCSR_DEVICE_ON;
                    }
                    v
                }
                SDIO_BACKPLANE_ADDRESS_LOW => (self.bp_window >> 8) as u8,
                SDIO_BACKPLANE_ADDRESS_MID => (self.bp_window >> 16) as u8,
                SDIO_BACKPLANE_ADDRESS_HIGH => (self.bp_window >> 24) as u8,
                a => self.f1_regs.get(&a).copied().unwrap_or(0),
            };
        }
        let bp = self.backplane_addr(addr);
        self.backplane_read_byte(bp)
    }

    fn f1_write_byte(&mut self, addr: u32, byte: u8) {
        if addr >= F1_REGISTER_SPACE {
            match addr {
                SDIO_BACKPLANE_ADDRESS_LOW => {
                    self.bp_window = (self.bp_window & !0x0000_FF00) | (u32::from(byte) << 8)
                }
                SDIO_BACKPLANE_ADDRESS_MID => {
                    self.bp_window = (self.bp_window & !0x00FF_0000) | (u32::from(byte) << 16)
                }
                SDIO_BACKPLANE_ADDRESS_HIGH => {
                    self.bp_window = (self.bp_window & !0xFF00_0000) | (u32::from(byte) << 24)
                }
                SDIO_CHIP_CLOCK_CSR => self.clock_csr = byte & 0x3F,
                SDIO_SLEEP_CSR => self.sleep_csr = byte & SBSDIO_SLPCSR_KEEP_SDIO_ON,
                a => {
                    self.f1_regs.insert(a, byte);
                }
            }
            return;
        }
        let bp = self.backplane_addr(addr);
        self.backplane_write_byte(bp, byte);
    }

    fn backplane_word(&self, word_addr: u32) -> u32 {
        match word_addr {
            CHIPCOMMON_BASE_ADDRESS => CHIP_ID_43439,
            a if a == WLAN_ARMCM3_WRAPPER + AI_IOCTRL_OFFSET => self.wlan_ioctrl,
            a if a == WLAN_ARMCM3_WRAPPER + AI_RESETCTRL_OFFSET => self.wlan_resetctrl,
            a if a == SOCSRAM_WRAPPER + AI_IOCTRL_OFFSET => self.socram_ioctrl,
            a if a == SOCSRAM_WRAPPER + AI_RESETCTRL_OFFSET => self.socram_resetctrl,
            a => self.backplane.get(&a).copied().unwrap_or(0),
        }
    }

    fn backplane_read_byte(&self, addr: u32) -> u8 {
        if (addr as usize) < RAM_SIZE {
            return self.ram[addr as usize];
        }
        let word = self.backplane_word(addr & !3);
        (word >> ((addr & 3) * 8)) as u8
    }

    fn backplane_write_byte(&mut self, addr: u32, byte: u8) {
        if (addr as usize) < RAM_SIZE {
            self.ram[addr as usize] = byte;
            return;
        }
        let word_addr = addr & !3;
        let shift = (addr & 3) * 8;
        let old = self.backplane_word(word_addr);
        let new = (old & !(0xFF << shift)) | (u32::from(byte) << shift);
        match word_addr {
            CHIPCOMMON_BASE_ADDRESS => {}
            a if a == WLAN_ARMCM3_WRAPPER + AI_IOCTRL_OFFSET => self.wlan_ioctrl = new,
            a if a == WLAN_ARMCM3_WRAPPER + AI_RESETCTRL_OFFSET => {
                self.wlan_resetctrl = new;
                self.maybe_boot_firmware();
            }
            a if a == SOCSRAM_WRAPPER + AI_IOCTRL_OFFSET => self.socram_ioctrl = new,
            a if a == SOCSRAM_WRAPPER + AI_RESETCTRL_OFFSET => self.socram_resetctrl = new,
            a => {
                self.backplane.insert(a, new);
            }
        }
    }

    /// The WLAN Arm core leaves reset with its clock on and not halted:
    /// the downloaded firmware starts if it is there — an image at RAM 0
    /// (a non-zero reset vector; the 43439A0 image's initial stack pointer
    /// word is 0, its ROM sets the stack) and the NVRAM length token
    /// (`~n << 16 | n`, in words) the driver writes to the last RAM word.
    fn maybe_boot_firmware(&mut self) {
        if self.firmware_running
            || self.wlan_resetctrl & AIRC_RESET != 0
            || self.wlan_ioctrl & SICF_CLOCK_EN == 0
            || self.wlan_ioctrl & SICF_CPUHALT != 0
        {
            return;
        }
        let word = |a: usize| u32::from_le_bytes(self.ram[a..a + 4].try_into().unwrap());
        let token = word(RAM_SIZE - 4);
        let nvram_words = token & 0xFFFF;
        let token_ok = nvram_words != 0 && (!token >> 16) & 0xFFFF == nvram_words;
        if word(4) != 0 && token_ok {
            self.firmware_running = true;
        }
    }

    /// One SDPCM frame from the host on F2.
    fn host_frame(&mut self, frame: &[u8]) {
        if !self.firmware_running || frame.len() < SDPCM_HEADER_LEN {
            return;
        }
        let size = usize::from(u16::from_le_bytes([frame[0], frame[1]]));
        let size_com = u16::from_le_bytes([frame[2], frame[3]]);
        if size as u16 != !size_com || size < SDPCM_HEADER_LEN || size > frame.len() {
            return;
        }
        let seq = frame[4];
        let channel = frame[5] & 0x0F;
        let header_len = usize::from(frame[7]);
        self.last_host_seq = Some(seq);
        match channel {
            CONTROL_HEADER => self.control_frame(&frame[header_len.min(size)..size]),
            DATA_HEADER => self.data_frames_dropped += 1,
            _ => {}
        }
    }

    fn control_frame(&mut self, body: &[u8]) {
        if body.len() < IOCTL_HEADER_LEN {
            return;
        }
        let word = |i: usize| u32::from_le_bytes(body[i..i + 4].try_into().unwrap());
        let cmd = word(0);
        let len = (word(4) & 0xFFFF) as usize;
        let flags = word(8);
        let payload = &body[IOCTL_HEADER_LEN..];
        let input = &payload[..len.min(payload.len())];
        self.ioctls += 1;
        let mut out = input.to_vec();
        out.resize(len, 0);
        match cmd {
            WLC_GET_VAR => self.get_var(input, &mut out),
            WLC_SET_VAR => self.set_var(input),
            _ => {
                // Other GETs answer zeros; SETs echo their argument.
                if flags & 0x2 == 0 {
                    out.iter_mut().for_each(|b| *b = 0);
                }
            }
        }
        let mut body = Vec::with_capacity(IOCTL_HEADER_LEN + out.len());
        body.extend_from_slice(&cmd.to_le_bytes());
        body.extend_from_slice(&(len as u32).to_le_bytes());
        body.extend_from_slice(&flags.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&out);
        self.queue_to_host(CONTROL_HEADER, &body);
    }

    /// Name of an iovar: the NUL-terminated string the payload starts with.
    fn var_name(input: &[u8]) -> (&[u8], &[u8]) {
        let end = input.iter().position(|&b| b == 0).unwrap_or(input.len());
        (&input[..end], input.get(end + 1..).unwrap_or(&[]))
    }

    fn get_var(&self, input: &[u8], out: &mut [u8]) {
        let (name, _) = Self::var_name(input);
        out.iter_mut().for_each(|b| *b = 0);
        let value: &[u8] = match name {
            b"cur_etheraddr" => &self.mac,
            b"ccgpioin" => &[0, 0, 0, 0],
            // CLM blob accepted (`cyw43_clm_load` checks for 0).
            b"clmload_status" => &[0, 0, 0, 0],
            b"gpioout" => &[0, 0, 0, 0],
            _ => &[],
        };
        let gpio_word;
        let value = if name == b"gpioout" {
            gpio_word = self.gpioout.to_le_bytes();
            &gpio_word[..]
        } else {
            value
        };
        let n = value.len().min(out.len());
        out[..n].copy_from_slice(&value[..n]);
    }

    fn set_var(&mut self, input: &[u8]) {
        let (name, args) = Self::var_name(input);
        if name == b"gpioout" && args.len() >= 8 {
            let mask = u32::from_le_bytes(args[0..4].try_into().unwrap());
            let value = u32::from_le_bytes(args[4..8].try_into().unwrap());
            let before = self.gpioout;
            self.gpioout = (self.gpioout & !mask) | (value & mask);
            self.led_edges += u64::from((before ^ self.gpioout) & 1);
        } else if name == b"cur_etheraddr" && args.len() >= 6 {
            self.mac.copy_from_slice(&args[..6]);
        }
    }

    fn queue_to_host(&mut self, channel: u8, body: &[u8]) {
        let size = SDPCM_HEADER_LEN + body.len();
        let credit = self
            .last_host_seq
            .map_or(CREDIT_WINDOW, |s| s.wrapping_add(CREDIT_WINDOW));
        let mut frame = Vec::with_capacity(size);
        frame.extend_from_slice(&(size as u16).to_le_bytes());
        frame.extend_from_slice(&(!(size as u16)).to_le_bytes());
        frame.push(self.device_seq);
        frame.push(channel);
        frame.push(0); // next_length
        frame.push(SDPCM_HEADER_LEN as u8);
        frame.push(0); // wireless_flow_control
        frame.push(credit);
        frame.extend_from_slice(&[0, 0]);
        frame.extend_from_slice(body);
        self.device_seq = self.device_seq.wrapping_add(1);
        self.to_host.push_back(frame);
    }
}

/// The pads the chip is wired to (pico-sdk `boards/pico_w.h` defaults).
#[derive(Debug, Clone, Copy)]
pub struct Cyw43439Pins {
    pub data: u8,
    pub clock: u8,
    pub cs: u8,
}

impl Default for Cyw43439Pins {
    fn default() -> Self {
        Self {
            data: 24,
            clock: 29,
            cs: 25,
        }
    }
}

/// The chip plus the wires it watches and drives.
#[derive(Debug)]
pub struct Cyw43439 {
    id: String,
    pins: Cyw43439Pins,
    chip: Mutex<Cyw43439Chip>,
    /// Every PIO block's pin wire: DIO is answered on all of them, the clock
    /// is followed on the one that drives it.
    pio_lines: Vec<Arc<PadLines>>,
    /// The SIO's held-pad view, so `gpio_get(DIO)` reads host-wake.
    held: Option<Arc<crate::peripherals::rp2040::sio::HeldPads>>,
}

impl Cyw43439 {
    pub fn new(
        id: &str,
        pins: Cyw43439Pins,
        mac: [u8; 6],
        pio_lines: Vec<Arc<PadLines>>,
        held: Option<Arc<crate::peripherals::rp2040::sio::HeldPads>>,
    ) -> Self {
        let dev = Self {
            id: id.to_string(),
            pins,
            chip: Mutex::new(Cyw43439Chip::new(mac)),
            pio_lines,
            held,
        };
        let wake = dev.chip().host_wake();
        dev.drive_dio(wake);
        dev
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn chip(&self) -> std::sync::MutexGuard<'_, Cyw43439Chip> {
        self.chip.lock().expect("cyw43439 poisoned")
    }

    fn drive_dio(&self, level: bool) {
        for lines in &self.pio_lines {
            lines.set_input(usize::from(self.pins.data), level);
        }
        if let Some(held) = &self.held {
            held.hold(self.pins.data, level);
        }
    }

    fn on_cs(&self, level: bool) {
        let wake = {
            let mut chip = self.chip();
            chip.chip_select(!level);
            level.then(|| chip.host_wake())
        };
        if let Some(wake) = wake {
            self.drive_dio(wake);
        }
    }

    fn on_clock(&self, lines: &PadLines) {
        let data = usize::from(self.pins.data);
        // The host's level while it drives DIO; while it listens, its own
        // output stage is released and the bit is ours.
        let dio = lines.drives(data) && lines.level(data);
        let out = self.chip().clock_rise(dio);
        if let Some(level) = out {
            self.drive_dio(level);
        }
    }
}

/// Chip select, from IO_BANK0's pad levels.
#[derive(Debug)]
struct CsListener(Arc<Cyw43439>);

impl LineListener for CsListener {
    fn line_edge(&self, _line: usize, level: bool, _at: Option<u64>) {
        self.0.on_cs(level);
    }
}

/// The clock line of one PIO block.
#[derive(Debug)]
struct ClockListener {
    dev: Arc<Cyw43439>,
    lines: Arc<PadLines>,
}

impl LineListener for ClockListener {
    fn line_edge(&self, line: usize, level: bool, _at: Option<u64>) {
        if level && line == usize::from(self.dev.pins.clock) && self.lines.drives(line) {
            self.dev.on_clock(&self.lines);
        }
    }
}

/// The chip's bus presence: no MMIO window; it exists so the run can watch
/// `WL_GPIO0..2` (`<id>:0` is the Pico W LED) and keep the listeners alive.
#[derive(Debug)]
pub struct Cyw43439Port {
    dev: Arc<Cyw43439>,
    _listeners: Vec<Arc<dyn LineListener>>,
}

impl Cyw43439Port {
    pub fn device(&self) -> &Arc<Cyw43439> {
        &self.dev
    }
}

impl crate::Peripheral for Cyw43439Port {
    fn read(&self, _offset: u64) -> crate::SimResult<u8> {
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> crate::SimResult<()> {
        Ok(())
    }

    fn read_gpio_pad(&self, pin: u8) -> Option<bool> {
        (pin < 3).then(|| self.dev.chip().wl_gpio(pin))
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        self.read_gpio_pad(pin)
    }

    // Edge-driven from the PIO clock and CS listeners: `tick()` is a no-op.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

static CYW43439_METADATA: KitMetadata = KitMetadata {
    inputs: std::borrow::Cow::Borrowed(&[]),
    device_type: std::borrow::Cow::Borrowed("cyw43439"),
    label: std::borrow::Cow::Borrowed("CYW43439 Wi-Fi/Bluetooth (Pico W)"),
    summary: std::borrow::Cow::Borrowed(
        "Infineon CYW43439 on the RP2040/RP2350 PIO gSPI bus: firmware download, \
         SDPCM ioctls and the WL_GPIO0 LED.",
    ),
    detail: std::borrow::Cow::Borrowed(
        "Bit-level gSPI device on the PIO clock line and a SIO chip select: F0 bus \
         registers (0xFEEDBEAD test word, 16/32-bit word modes, interrupt and status \
         words), F1 backplane (window, clock and sleep CSRs, core wrappers, 512 KiB \
         SOCSRAM) and the host interface of a running firmware on F2 (SDPCM control \
         frames, credit, gpioout, cur_etheraddr). The firmware is not executed: no \
         radio, scan, join, data or Bluetooth. Watch `<id>:0` for the Pico W LED.",
    ),
    transport: Transport::GpioGroup,
    category: Category::Misc,
    config_keys: std::borrow::Cow::Borrowed(&[
        ConfigKey {
            name: std::borrow::Cow::Borrowed("data_pin"),
            ty: ConfigType::Int,
            doc: std::borrow::Cow::Borrowed("DIO / host-wake GPIO. Default 24."),
        },
        ConfigKey {
            name: std::borrow::Cow::Borrowed("clock_pin"),
            ty: ConfigType::Int,
            doc: std::borrow::Cow::Borrowed("gSPI clock GPIO (PIO side-set). Default 29."),
        },
        ConfigKey {
            name: std::borrow::Cow::Borrowed("cs_pin"),
            ty: ConfigType::Int,
            doc: std::borrow::Cow::Borrowed("Chip-select GPIO (SIO). Default 25."),
        },
        ConfigKey {
            name: std::borrow::Cow::Borrowed("mac"),
            ty: ConfigType::Str,
            doc: std::borrow::Cow::Borrowed(
                "MAC address the chip reports, `aa:bb:cc:dd:ee:ff`. Default 28:cd:c1:00:00:01.",
            ),
        },
    ]),
    labs: std::borrow::Cow::Borrowed(&[]),
};

pub struct Cyw43439Kit;
pub static CYW43439_KIT: Cyw43439Kit = Cyw43439Kit;

fn parse_mac(text: &str) -> anyhow::Result<[u8; 6]> {
    let parts: Vec<&str> = text.split(':').collect();
    if parts.len() != 6 {
        anyhow::bail!("cyw43439: mac '{text}' is not six ':'-separated bytes");
    }
    let mut mac = [0u8; 6];
    for (b, p) in mac.iter_mut().zip(parts) {
        *b = u8::from_str_radix(p, 16)
            .map_err(|_| anyhow::anyhow!("cyw43439: mac '{text}' has a bad byte '{p}'"))?;
    }
    Ok(mac)
}

impl PeripheralKit for Cyw43439Kit {
    fn metadata(&self) -> &KitMetadata {
        &CYW43439_METADATA
    }

    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let pin = |key: &str, default: u8| -> anyhow::Result<u8> {
            match ctx.config_i64(key) {
                None => Ok(default),
                Some(v) if (0..30).contains(&v) => Ok(v as u8),
                Some(v) => anyhow::bail!("cyw43439: {key} {v} is not an RP2040/RP2350A GPIO"),
            }
        };
        let defaults = Cyw43439Pins::default();
        let pins = Cyw43439Pins {
            data: pin("data_pin", defaults.data)?,
            clock: pin("clock_pin", defaults.clock)?,
            cs: pin("cs_pin", defaults.cs)?,
        };
        let mac = match ctx.config_str("mac") {
            Some(text) => parse_mac(text)?,
            None => DEFAULT_MAC,
        };
        let id = ctx.device_id().to_string();
        let bus = &mut *ctx.bus;
        let unsupported = || {
            anyhow::anyhow!(
                "cyw43439 '{id}': needs an RP2040/RP2350 bus (io_bank0, sio and a pio block)"
            )
        };
        let (bank_irq, held) = bus.rp2040_part_pads().ok_or_else(unsupported)?;
        let mut pio_lines = Vec::new();
        for name in ["pio0", "pio1", "pio2"] {
            if let Some(i) = bus.find_peripheral_index_by_name(name) {
                if let Some(lines) = bus.peripherals[i].dev.timer_output_lines(false) {
                    if lines.names() == crate::peripherals::pio::LINE_NAMES {
                        pio_lines.push(lines);
                    }
                }
            }
        }
        if pio_lines.is_empty() {
            return Err(unsupported());
        }

        let dev = Arc::new(Cyw43439::new(&id, pins, mac, pio_lines.clone(), Some(held)));
        let mut listeners: Vec<Arc<dyn LineListener>> = Vec::new();
        let cs: Arc<dyn LineListener> = Arc::new(CsListener(dev.clone()));
        bank_irq.add_listener(pins.cs, &cs);
        listeners.push(cs);
        for lines in pio_lines {
            let clock: Arc<dyn LineListener> = Arc::new(ClockListener {
                dev: dev.clone(),
                lines: lines.clone(),
            });
            lines.add_listener(usize::from(pins.clock), &clock);
            listeners.push(clock);
        }
        bus.add_peripheral(
            &id,
            0,
            0,
            None,
            Box::new(Cyw43439Port {
                dev,
                _listeners: listeners,
            }),
        );
        Ok(())
    }
}

#[cfg(test)]
#[path = "cyw43439_tests.rs"]
mod tests;
