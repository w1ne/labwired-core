#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

// RP2040 Peripheral Base Addresses (from SVD/JSON)
const UART0_BASE: u32 = 0x40034000;
const PIO0_BASE: u32 = 0x50200000;

// UART Registers (stm32v2 layout: TDR at offset 0x28)
const UART0_TDR: *mut u32 = (UART0_BASE + 0x28) as *mut u32;

// PIO Registers
const PIO0_CTRL: *mut u32 = PIO0_BASE as *mut u32;
const PIO0_FSTAT: *const u32 = (PIO0_BASE + 0x04) as *const u32;
const PIO0_INSTR_MEM0: *mut u32 = (PIO0_BASE + 0x48) as *mut u32;
const PIO0_INSTR_MEM1: *mut u32 = (PIO0_BASE + 0x4C) as *mut u32;
const PIO0_TXF0: *mut u32 = (PIO0_BASE + 0x10) as *mut u32;
// SM0 registers: clkdiv=0x0c8, exec_ctrl=0x0cc, shift_ctrl=0x0d0, pc=0x0d4
const PIO0_SM0_EXEC_CTRL: *mut u32 = (PIO0_BASE + 0xCC) as *mut u32;

#[entry]
fn main() -> ! {
    unsafe {
        // PIO Program (2 instructions):
        //   addr 0: SET X, 10    -> opcode 0xE02A
        //   addr 1: PULL block   -> opcode 0x80A0 (stalls until data in TX FIFO)
        core::ptr::write_volatile(PIO0_INSTR_MEM0, 0xE02A);
        core::ptr::write_volatile(PIO0_INSTR_MEM1, 0x80A0);

        // Set wrap: top=1, bottom=0 (so program wraps within [0,1])
        core::ptr::write_volatile(PIO0_SM0_EXEC_CTRL, 1u32 << 12);

        // Enable SM0
        core::ptr::write_volatile(PIO0_CTRL, 1);

        // After a few ticks, SM0 should have executed SET X,10 and then
        // stalled on PULL (PC=1). Feed TX FIFO to unblock.
        // First, wait a bit for the PIO to reach the PULL stall.
        for _ in 0..10u32 {
            cortex_m::asm::nop();
        }

        // Now push a value into TX FIFO to unblock PULL
        core::ptr::write_volatile(PIO0_TXF0, 0xDEAD_BEEF);

        // Wait a bit for PIO to consume
        for _ in 0..10u32 {
            cortex_m::asm::nop();
        }

        // Read FSTAT to check if TX FIFO for SM0 is now empty (bit 24 = TXEMPTY for SM0)
        let fstat = core::ptr::read_volatile(PIO0_FSTAT);
        let txempty_sm0 = (fstat >> 24) & 1;

        if txempty_sm0 == 1 {
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
