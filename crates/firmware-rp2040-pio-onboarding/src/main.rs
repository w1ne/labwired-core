#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

// RP2040 Peripheral Base Addresses (from SVD/JSON)
const UART0_BASE: u32 = 0x40034000;
const PIO0_BASE: u32 = 0x50200000;
const IO_BANK0_BASE: u32 = 0x40014000;

// UART data register (PL011 UARTDR at offset 0x00 — the RP2040's real UART,
// matching firmware-rp2040-demo and configs/chips/rp2040.yaml profile: pl011).
const UART0_TDR: *mut u32 = UART0_BASE as *mut u32;

// PIO Registers
const PIO0_CTRL: *mut u32 = PIO0_BASE as *mut u32;
const PIO0_FSTAT: *const u32 = (PIO0_BASE + 0x04) as *const u32;
const PIO0_INSTR_MEM0: *mut u32 = (PIO0_BASE + 0x48) as *mut u32;
const PIO0_INSTR_MEM1: *mut u32 = (PIO0_BASE + 0x4C) as *mut u32;
const PIO0_INSTR_MEM2: *mut u32 = (PIO0_BASE + 0x50) as *mut u32;
const PIO0_INSTR_MEM3: *mut u32 = (PIO0_BASE + 0x54) as *mut u32;
const PIO0_INSTR_MEM4: *mut u32 = (PIO0_BASE + 0x58) as *mut u32;
const PIO0_INSTR_MEM5: *mut u32 = (PIO0_BASE + 0x5C) as *mut u32;
const PIO0_TXF1: *mut u32 = (PIO0_BASE + 0x14) as *mut u32;
// SM0 registers: exec_ctrl=0x0cc, pin_ctrl=0x0dc.
const PIO0_SM0_EXEC_CTRL: *mut u32 = (PIO0_BASE + 0xCC) as *mut u32;
const PIO0_SM0_PIN_CTRL: *mut u32 = (PIO0_BASE + 0xDC) as *mut u32;
// SM1 registers: exec_ctrl=0x0e4, addr=0x0ec.
const PIO0_SM1_EXEC_CTRL: *mut u32 = (PIO0_BASE + 0xE4) as *mut u32;
const PIO0_SM1_ADDR: *mut u32 = (PIO0_BASE + 0xEC) as *mut u32;
const GPIO25_CTRL: *mut u32 = (IO_BANK0_BASE + 25 * 8 + 4) as *mut u32;

#[entry]
fn main() -> ! {
    unsafe {
        // Route the Pico's onboard LED (GP25) to PIO0. FUNCSEL=6 is PIO0.
        core::ptr::write_volatile(GPIO25_CTRL, 6);

        // Map SET PINS/PINDIRS to GP25 (SET_BASE=25, SET_COUNT=1).
        core::ptr::write_volatile(PIO0_SM0_PIN_CTRL, (1 << 26) | (25 << 5));

        // Shared PIO program:
        //   0: SET PINDIRS, 1 -> 0xE081
        //   1: SET PINS, 1    -> 0xE001
        //   2: SET PINS, 0    -> 0xE000
        //   3: JMP 1          -> 0x0001 (SM0 keeps blinking GP25)
        //   4: SET X, 10      -> 0xE02A
        //   5: PULL block     -> 0x80A0 (SM1 waits for TXF1)
        core::ptr::write_volatile(PIO0_INSTR_MEM0, 0xE081);
        core::ptr::write_volatile(PIO0_INSTR_MEM1, 0xE001);
        core::ptr::write_volatile(PIO0_INSTR_MEM2, 0xE000);
        core::ptr::write_volatile(PIO0_INSTR_MEM3, 0x0001);
        core::ptr::write_volatile(PIO0_INSTR_MEM4, 0xE02A);
        core::ptr::write_volatile(PIO0_INSTR_MEM5, 0x80A0);

        // SM0 loops over the LED program; SM1 starts at the FIFO self-check.
        core::ptr::write_volatile(PIO0_SM0_EXEC_CTRL, 3u32 << 12);
        core::ptr::write_volatile(PIO0_SM1_EXEC_CTRL, (5u32 << 12) | (4u32 << 7));
        core::ptr::write_volatile(PIO0_SM1_ADDR, 4);

        // Enable both state machines.
        core::ptr::write_volatile(PIO0_CTRL, 3);

        // Let SM1 reach its blocking PULL, while SM0 continues toggling GP25.
        for _ in 0..10u32 {
            cortex_m::asm::nop();
        }

        // Push a value into SM1's TX FIFO to unblock PULL.
        core::ptr::write_volatile(PIO0_TXF1, 0xDEAD_BEEF);

        // Wait a bit for PIO to consume
        for _ in 0..10u32 {
            cortex_m::asm::nop();
        }

        // Read FSTAT to check SM1's TX FIFO is empty (bit 25 = TXEMPTY for SM1).
        let fstat = core::ptr::read_volatile(PIO0_FSTAT);
        let txempty_sm1 = (fstat >> 25) & 1;

        if txempty_sm1 == 1 {
            // PIO consumed the data from TX FIFO successfully
            print_uart("PIO_OK\n");
        } else {
            print_uart("PIO_FAIL\n");
        }
    }

    loop {
        cortex_m::asm::nop();
    }
}

fn print_uart(s: &str) {
    for b in s.bytes() {
        unsafe {
            core::ptr::write_volatile(UART0_TDR, b as u32);
        }
    }
}
