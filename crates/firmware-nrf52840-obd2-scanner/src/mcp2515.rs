//! MCP2515 over nRF52840 SPIM2 EasyDMA (16 MHz oscillator, 500 kbit/s CAN).

use crate::CanFrame;
use core::{
    cell::UnsafeCell,
    ptr::{read_volatile, write_volatile},
    sync::atomic::{AtomicBool, Ordering},
};

const SPIM: usize = 0x4002_3000;
const GPIO: usize = 0x5000_0000;
const CS: u32 = 1 << 12;
const WAIT_LIMIT: u32 = 100_000;
const RESET: u8 = 0xc0;
const READ: u8 = 0x03;
const WRITE: u8 = 0x02;
const BIT_MODIFY: u8 = 0x05;
pub const LOAD_TXB0: u8 = 0x40;
pub const RTS_TXB0: u8 = 0x81;
pub const SPIM_EVENTS_STOPPED: usize = 0x104;
pub const SPIM_EVENTS_END: usize = 0x118;
pub const MCP_500K_16MHZ_CNF: [u8; 3] = [0x00, 0xbc, 0x01];
pub const SPIM_DMA_STATIC: bool = true;
const RESET_RECOVERY: u32 = 10_000;
const CONFIG_POLLS: u32 = 64;
const TX_POLLS: u32 = 256;

struct DmaCell(UnsafeCell<[u8; 14]>);
unsafe impl Sync for DmaCell {}
static SPIM_TX: DmaCell = DmaCell(UnsafeCell::new([0; 14]));
static SPIM_RX: DmaCell = DmaCell(UnsafeCell::new([0; 14]));
static TAKEN: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Timeout,
    Configuration,
    Overflow,
    NoFrame,
    InvalidFrame,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxDecision {
    Pending,
    Complete,
    Failed,
}

pub const fn tx_decision(txb0ctrl: u8, canintf: u8) -> TxDecision {
    if txb0ctrl & 0x70 != 0 {
        TxDecision::Failed
    } else if txb0ctrl & 0x08 == 0 && canintf & 0x04 != 0 {
        TxDecision::Complete
    } else {
        TxDecision::Pending
    }
}

pub struct Mcp2515 {
    stuck: bool,
}

impl Mcp2515 {
    /// Claims the sole non-reentrant SPIM2/static-DMA-buffer instance.
    pub fn take() -> Option<Self> {
        TAKEN
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self { stuck: false })
    }

    pub fn init(&mut self) -> Result<(), Error> {
        unsafe {
            wr(GPIO, 0x518, CS);
            wr(GPIO, 0x508, CS);
            // IRQ P0.11 input with pull-up, CS output, SPI pins assigned directly.
            wr(GPIO, 0x700 + 11 * 4, 3 << 2);
            wr(SPIM, 0x508, 13);
            wr(SPIM, 0x50c, 14);
            wr(SPIM, 0x510, 15);
            wr(SPIM, 0x524, 0x0200_0000); // 2 MHz, safe for init and simulation.
            wr(SPIM, 0x554, 0);
            wr(SPIM, 0x500, 7);
        }
        self.command(&[RESET])?;
        // MCP2515 oscillator/reset recovery, then bounded configuration-mode poll.
        for _ in 0..RESET_RECOVERY {
            unsafe {
                let _ = rd(SPIM, 0x500);
            }
        }
        let mut config_ready = false;
        for _ in 0..CONFIG_POLLS {
            if self.read(0x0e)? & 0xe0 == 0x80 {
                config_ready = true;
                break;
            }
        }
        if !config_ready {
            return Err(Error::Timeout);
        }
        // Configuration mode, 500 kbps at 16 MHz: 16 TQ/bit.
        self.write(0x0f, 0x80)?;
        self.write(0x2a, MCP_500K_16MHZ_CNF[0])?;
        self.write(0x29, MCP_500K_16MHZ_CNF[1])?;
        self.write(0x28, MCP_500K_16MHZ_CNF[2])?;
        // Exact 11-bit mask with filters for the functional ECU response ID.
        self.write_standard_id(0x20, 0x7ff)?; // RXM0
        self.write_standard_id(0x24, 0x7ff)?; // RXM1
        for filter in [0x00, 0x04, 0x08, 0x10, 0x14, 0x18] {
            self.write_standard_id(filter, 0x7e8)?;
        }
        self.write(0x60, 0x04)?; // standard-filtered, rollover enabled
        self.write(0x70, 0x00)?; // standard-filtered
        self.write(0x2b, 0x03)?;
        self.write(0x0f, 0x00)?;
        if self.read(0x0e)? & 0xe0 != 0 {
            return Err(Error::Configuration);
        }
        Ok(())
    }

    pub fn irq_asserted(&self) -> bool {
        unsafe { rd(GPIO, 0x510) & (1 << 11) == 0 }
    }
    pub fn interrupt_flags(&mut self) -> Result<u8, Error> {
        self.read(0x2c)
    }
    pub fn clear_overflow(&mut self) -> Result<(), Error> {
        self.bit_modify(0x1d, 0xc0, 0)
    }

    pub fn send(&mut self, frame: &CanFrame) -> Result<(), Error> {
        validate_frame(frame)?;
        let initial_control = match self.read(0x30) {
            Ok(value) => value,
            Err(error) => return self.abort_txb0(error),
        };
        if initial_control & 0x08 != 0 {
            return self.abort_txb0(Error::Timeout);
        }
        self.bit_modify(0x2c, 0x04, 0)?;
        let id = frame.id;
        let mut packet = [0u8; 14];
        packet[0] = LOAD_TXB0;
        packet[1] = (id >> 3) as u8;
        packet[2] = (id << 5) as u8;
        packet[5] = frame.len.min(8);
        packet[6..14].copy_from_slice(&frame.data);
        self.command(&packet)?;
        if let Err(error) = self.command(&[RTS_TXB0]) {
            return self.abort_txb0(error);
        }
        for _ in 0..TX_POLLS {
            let control = match self.read(0x30) {
                Ok(value) => value,
                Err(error) => return self.abort_txb0(error),
            };
            let interrupts = match self.interrupt_flags() {
                Ok(value) => value,
                Err(error) => return self.abort_txb0(error),
            };
            match tx_decision(control, interrupts) {
                TxDecision::Complete => {
                    self.bit_modify(0x2c, 0x04, 0)?;
                    return Ok(());
                }
                TxDecision::Failed => return self.abort_txb0(Error::Configuration),
                TxDecision::Pending => {}
            }
        }
        self.abort_txb0(Error::Timeout)
    }

    pub fn receive(&mut self) -> Result<CanFrame, Error> {
        let flags = self.interrupt_flags()?;
        let (opcode, clear) = if flags & 1 != 0 {
            (0x90, 1)
        } else if flags & 2 != 0 {
            (0x94, 2)
        } else {
            return Err(Error::NoFrame);
        };
        self.transfer(&[opcode, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])?;
        let mut registers = [0u8; 13];
        for (index, byte) in registers.iter_mut().enumerate() {
            *byte = dma_rx(1 + index);
        }
        // READ RX auto-clears RXnIF on CS release; explicit clear also supports models
        // that implement only register semantics. It happens before DLC validation.
        self.bit_modify(0x2c, clear, 0)?;
        decode_rx_registers(&registers)
    }

    pub fn read(&mut self, address: u8) -> Result<u8, Error> {
        self.transfer(&[READ, address, 0])?;
        Ok(dma_rx(2))
    }
    pub fn write(&mut self, address: u8, value: u8) -> Result<(), Error> {
        self.command(&[WRITE, address, value])
    }
    pub fn bit_modify(&mut self, address: u8, mask: u8, value: u8) -> Result<(), Error> {
        self.command(&[BIT_MODIFY, address, mask, value])
    }
    fn write_standard_id(&mut self, address: u8, id: u16) -> Result<(), Error> {
        self.command(&[WRITE, address, (id >> 3) as u8, (id << 5) as u8, 0, 0])
    }
    fn command(&mut self, tx: &[u8]) -> Result<(), Error> {
        self.transfer(tx)
    }
    fn abort_txb0(&mut self, result: Error) -> Result<(), Error> {
        if self.bit_modify(0x30, 0x08, 0).is_err() {
            self.stuck = true;
            return Err(Error::Configuration);
        }
        for _ in 0..TX_POLLS {
            match self.read(0x30) {
                Ok(control) if control & 0x08 == 0 => return Err(result),
                Ok(_) => {}
                Err(_) => {
                    self.stuck = true;
                    return Err(Error::Configuration);
                }
            }
        }
        self.stuck = true;
        Err(Error::Configuration)
    }
    fn transfer(&mut self, tx: &[u8]) -> Result<(), Error> {
        if self.stuck || tx.len() > 14 {
            return Err(Error::Configuration);
        }
        unsafe {
            for index in 0..14 {
                (*SPIM_TX.0.get())[index] = if index < tx.len() { tx[index] } else { 0 };
                (*SPIM_RX.0.get())[index] = 0;
            }
            wr(GPIO, 0x50c, CS);
            wr(SPIM, SPIM_EVENTS_END, 0);
            wr(SPIM, SPIM_EVENTS_STOPPED, 0);
            wr(SPIM, 0x544, SPIM_TX.0.get() as *mut u8 as u32);
            wr(SPIM, 0x548, tx.len() as u32);
            wr(SPIM, 0x534, SPIM_RX.0.get() as *mut u8 as u32);
            wr(SPIM, 0x538, tx.len() as u32);
            wr(SPIM, 0x010, 1);
            for _ in 0..WAIT_LIMIT {
                if rd(SPIM, SPIM_EVENTS_END) != 0 {
                    wr(GPIO, 0x508, CS);
                    return Ok(());
                }
            }
            wr(SPIM, 0x014, 1);
            for _ in 0..WAIT_LIMIT {
                if rd(SPIM, SPIM_EVENTS_STOPPED) != 0 {
                    wr(GPIO, 0x508, CS);
                    return Err(Error::Timeout);
                }
            }
            // Buffers remain driver-owned; poison future use if STOP never lands.
            self.stuck = true;
            wr(GPIO, 0x508, CS);
            Err(Error::Timeout)
        }
    }
}
pub fn validate_frame(frame: &CanFrame) -> Result<(), Error> {
    if frame.id > 0x7ff || frame.len > 8 {
        Err(Error::InvalidFrame)
    } else {
        Ok(())
    }
}
pub fn decode_rx_registers(registers: &[u8; 13]) -> Result<CanFrame, Error> {
    let len = registers[4] & 0x0f;
    if len > 8 {
        return Err(Error::InvalidFrame);
    }
    let id = ((registers[0] as u16) << 3) | ((registers[1] as u16) >> 5);
    let mut data = [0; 8];
    data.copy_from_slice(&registers[5..13]);
    Ok(CanFrame { id, len, data })
}
fn dma_rx(index: usize) -> u8 {
    unsafe { (*SPIM_RX.0.get())[index] }
}
unsafe fn wr(base: usize, offset: usize, value: u32) {
    write_volatile((base + offset) as *mut u32, value)
}
unsafe fn rd(base: usize, offset: usize) -> u32 {
    read_volatile((base + offset) as *const u32)
}
