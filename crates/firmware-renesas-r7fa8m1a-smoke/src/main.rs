// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
//! R7FA8M1A (RA8M1, Cortex-M85) smoke firmware for the imported Renode
//! platform: SCI0 (SCI_B) polled TX and a P600 heartbeat, in bare registers.
//!
//! The RA8M1 resets onto MOCO (8 MHz). The SCI_B channels run from their own
//! SCICLK, so the firmware switches SCICLK to MOCO /1 with the FSP
//! `bsp_peripheral_clock_set` CKSREQ/CKSRDY handshake under PRCR.PRC0, takes
//! SCI0 out of module stop (MSTPCRB.MSTPB31), programs SCI0 the way FSP
//! `r_sci_b_uart.c` does and waits for CESR.TIST/RIST, prints a banner, then
//! makes P600 an output and toggles it through PCNTR3.
//!
//! Readbacks stored in SRAM: 0x22000000 MUL 17*19, 0x22000004 SCICKCR,
//! 0x22000008 CCR3, 0x2200000C PORT6 PODR with P600 set.
//!
//! The firmware does not route SCI0 pads through PmnPFS (the model needs no
//! pad routing for SCI_B TX). Firmware for real silicon must.
//!
//! Baud: SCICLK = MOCO 8 MHz / 1. Async, CCR2.CKS = 0, ABCS = BGDM = 0:
//! BRR = SCICLK / (32 * 9600) - 1 = 25.04 -> 25 (9615 baud, +0.16 %).

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RESULT: *mut u32 = 0x2200_0000 as *mut u32;

// R_SYSTEM @ 0x4001E000 (R7FA8M1AH.h): SCICKDIVCR @ 0x54, SCICKCR @ 0x55,
// PRCR (secure) @ 0x3FA.
const SYSTEM: u32 = 0x4001_E000;
const SCICKDIVCR: *mut u8 = (SYSTEM + 0x54) as *mut u8;
const SCICKCR: *mut u8 = (SYSTEM + 0x55) as *mut u8;
const SCICKCR_CKSREQ: u8 = 1 << 6;
const SCICKCR_CKSRDY: u8 = 1 << 7;
// FSP BSP_CLOCKS_SOURCE_CLOCK_MOCO.
const SCICKSEL_MOCO: u8 = 1;
const PRCR: *mut u16 = (SYSTEM + 0x3FA) as *mut u16;
const PRCR_KEY: u16 = 0xA500;
const PRCR_PRC0: u16 = 1 << 0;

// R_MSTP @ 0x40203000: MSTPCRB @ 0x04; SCIn is MSTPB(31 - n) (FSP
// bsp_module_stop.h BSP_MSTP_BIT_FSP_IP_SCI). 0 = module running.
const MSTPCRB: *mut u32 = 0x4020_3004 as *mut u32;
const MSTPB31_SCI0: u32 = 1 << 31;

// PORT6 @ 0x40400000 + 0x20*6: PCNTR1 PDR[15:0]/PODR[31:16] @ 0x00, PCNTR3
// POSR[15:0]/PORR[31:16] @ 0x08.
const PORT6: u32 = 0x4040_00C0;
const PCNTR1: *mut u32 = PORT6 as *mut u32;
const PCNTR3: *mut u32 = (PORT6 + 0x08) as *mut u32;
const P600: u32 = 1 << 0;

// SCI0 (SCI_B) @ 0x40358000 (R_SCI_B0_Type): TDR +0x04, CCR0-3 +0x08..+0x14,
// CESR +0x1C, CSR +0x48, CFCLR +0x68.
const SCI0: u32 = 0x4035_8000;
const SCI_TDR: *mut u32 = (SCI0 + 0x04) as *mut u32;
const SCI_CCR0: *mut u32 = (SCI0 + 0x08) as *mut u32;
const SCI_CCR1: *mut u32 = (SCI0 + 0x0C) as *mut u32;
const SCI_CCR2: *mut u32 = (SCI0 + 0x10) as *mut u32;
const SCI_CCR3: *mut u32 = (SCI0 + 0x14) as *mut u32;
const SCI_CESR: *const u32 = (SCI0 + 0x1C) as *const u32;
const SCI_CSR: *const u32 = (SCI0 + 0x48) as *const u32;
const SCI_CFCLR: *mut u32 = (SCI0 + 0x68) as *mut u32;
const CCR0_RE: u32 = 1 << 0;
const CCR0_TE: u32 = 1 << 4;
// CCR1.SPB2DT: TXD idles high while TE is off (FSP default).
const CCR1_SPB2DT: u32 = 1 << 4;
// CCR3: CHR = 0b10 (8 bits, FSP UART_DATA_BITS_8), LSBF, MOD = 0 (async).
const CCR3_CHR_8BIT: u32 = 2 << 8;
const CCR3_LSBF: u32 = 1 << 12;
const CESR_TIST: u32 = 1 << 4;
const CESR_RIST: u32 = 1 << 0;
const CSR_TDRE: u32 = 1 << 29;
const CSR_TEND: u32 = 1 << 30;
// FSP SCI_B_UART_CFCLR_CLEAR_ALL_MASK.
const CFCLR_ALL: u32 = 0x9D07_0010;

const MOCO_HZ: u32 = 8_000_000;
const BAUD: u32 = 9600;
const BRR: u32 = MOCO_HZ / (32 * BAUD) - 1;

fn putc(byte: u8) {
    unsafe {
        while read_volatile(SCI_CSR) & CSR_TDRE == 0 {}
        write_volatile(SCI_TDR, u32::from(byte));
    }
}

fn puts(text: &[u8]) {
    for byte in text {
        putc(*byte);
    }
}

#[inline(never)]
fn spin(iterations: u32) {
    for _ in 0..iterations {
        unsafe {
            core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
        }
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        let product: u32;
        core::arch::asm!(
            "mul {product}, {left}, {right}",
            product = out(reg) product,
            left = in(reg) 17u32,
            right = in(reg) 19u32,
            options(nomem, nostack, pure),
        );
        write_volatile(RESULT, product);

        // SCICLK = MOCO / 1 (SCICKCR/SCICKDIVCR are PRC0-protected).
        write_volatile(PRCR, PRCR_KEY | PRCR_PRC0);
        write_volatile(SCICKCR, read_volatile(SCICKCR) | SCICKCR_CKSREQ);
        while read_volatile(SCICKCR) & SCICKCR_CKSRDY == 0 {}
        write_volatile(SCICKDIVCR, 0);
        write_volatile(SCICKCR, SCICKSEL_MOCO | SCICKCR_CKSREQ | SCICKCR_CKSRDY);
        write_volatile(SCICKCR, read_volatile(SCICKCR) & !SCICKCR_CKSREQ);
        while read_volatile(SCICKCR) & SCICKCR_CKSRDY != 0 {}
        write_volatile(RESULT.add(1), u32::from(read_volatile(SCICKCR)));
        write_volatile(PRCR, PRCR_KEY);

        // Release SCI0 from module stop.
        write_volatile(MSTPCRB, read_volatile(MSTPCRB) & !MSTPB31_SCI0);

        // SCI0 async 8N1, in the r_sci_b_uart.c R_SCI_B_UART_Open order.
        write_volatile(SCI_CCR0, 0);
        while read_volatile(SCI_CESR) & (CESR_TIST | CESR_RIST) != 0 {}
        write_volatile(SCI_CFCLR, CFCLR_ALL);
        write_volatile(SCI_CCR1, CCR1_SPB2DT);
        write_volatile(SCI_CCR2, BRR << 8);
        write_volatile(SCI_CCR3, CCR3_CHR_8BIT | CCR3_LSBF);
        write_volatile(RESULT.add(2), read_volatile(SCI_CCR3));
        write_volatile(SCI_CCR0, CCR0_TE | CCR0_RE);
        while read_volatile(SCI_CESR) & (CESR_TIST | CESR_RIST) != (CESR_TIST | CESR_RIST) {}
    }

    puts(b"RA8M1 SCI0 SMOKE\n");
    unsafe { while read_volatile(SCI_CSR) & CSR_TEND == 0 {} }

    unsafe {
        // P600 output, set through POSR, PODR readback.
        write_volatile(PCNTR1, read_volatile(PCNTR1) | P600);
        write_volatile(PCNTR3, P600);
        write_volatile(RESULT.add(3), read_volatile(PCNTR1) >> 16);
    }

    let mut on = true;
    loop {
        spin(2_000);
        on = !on;
        let bit = if on { P600 } else { P600 << 16 };
        unsafe { write_volatile(PCNTR3, bit) };
    }
}
