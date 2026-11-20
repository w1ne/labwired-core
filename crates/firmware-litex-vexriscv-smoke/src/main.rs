// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! LiteX VexRiscv SoC smoke firmware.
//!
//! Boots from the SoC's integrated SRAM (fast boot: the ELF is loaded
//! directly, the LiteX BIOS and the SPI-flash load are skipped) and prints
//! over the LiteX UART CSR model:
//!
//! * a banner and the modelled-surface summary,
//! * a bitbanged exchange with the SPI flash through the LiteX SPI-flash
//!   controller CSRs: READ ID (`spi id=20bb19`), then READ STATUS before and
//!   after WRITE ENABLE (`sr=00 wel=02`),
//! * a one-shot run of the modelled LiteX timer: `timer=running` while the
//!   countdown is live, `timer=zero` when the zero event latches,
//! * a heartbeat whose `txfull` field is READ BACK from the UART CSR after
//!   the TX-ready poll, so the assertion sees the register the firmware
//!   actually observed.
//!
//! Built for `riscv32im-unknown-none-elf`: the tested CPU subset is RV32IM
//! (no compressed or atomic instructions). The `verilated-liteuart-profile`
//! feature relocates the UART and timer to the addresses in Renode's imported
//! Verilated LiteUART REPL. That profile uses LabWired's LiteX UART CSR model;
//! it does not run Renode's external CoSimulatedUART implementation.

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use panic_halt as _;
use riscv_rt::entry;

/// LiteX UART CSR window (rxtx @ +0x00, txfull @ +0x04).
#[cfg(feature = "verilated-cfu-profile")]
const UART_RXTX: *mut u32 = 0xf000_3000 as *mut u32;
#[cfg(all(
    not(feature = "verilated-cfu-profile"),
    feature = "verilated-liteuart-profile"
))]
const UART_RXTX: *mut u32 = 0xe000_1000 as *mut u32;
#[cfg(all(
    not(feature = "verilated-cfu-profile"),
    not(feature = "verilated-liteuart-profile")
))]
const UART_RXTX: *mut u32 = 0x6000_1800 as *mut u32;
#[cfg(feature = "verilated-cfu-profile")]
const UART_TXFULL: *const u32 = 0xf000_3004 as *const u32;
#[cfg(all(
    not(feature = "verilated-cfu-profile"),
    feature = "verilated-liteuart-profile"
))]
const UART_TXFULL: *const u32 = 0xe000_1004 as *const u32;
#[cfg(all(
    not(feature = "verilated-cfu-profile"),
    not(feature = "verilated-liteuart-profile")
))]
const UART_TXFULL: *const u32 = 0x6000_1804 as *const u32;

/// LiteX timer CSR window (`litex/soc/cores/timer.py`): load @ +0x00,
/// reload @ +0x04, en @ +0x08, update_value @ +0x0C, value @ +0x10,
/// ev_status @ +0x14, ev_pending @ +0x18, ev_enable @ +0x1C.
#[cfg(feature = "verilated-cfu-profile")]
const TIMER_LOAD: *mut u32 = 0xf000_2800 as *mut u32;
#[cfg(all(
    not(feature = "verilated-cfu-profile"),
    feature = "verilated-liteuart-profile"
))]
const TIMER_LOAD: *mut u32 = 0xe000_2800 as *mut u32;
#[cfg(all(
    not(feature = "verilated-cfu-profile"),
    not(feature = "verilated-liteuart-profile")
))]
const TIMER_LOAD: *mut u32 = 0x6000_2800 as *mut u32;
#[cfg(feature = "verilated-cfu-profile")]
const TIMER_RELOAD: *mut u32 = 0xf000_2804 as *mut u32;
#[cfg(all(
    not(feature = "verilated-cfu-profile"),
    feature = "verilated-liteuart-profile"
))]
const TIMER_RELOAD: *mut u32 = 0xe000_2804 as *mut u32;
#[cfg(all(
    not(feature = "verilated-cfu-profile"),
    not(feature = "verilated-liteuart-profile")
))]
const TIMER_RELOAD: *mut u32 = 0x6000_2804 as *mut u32;
#[cfg(feature = "verilated-cfu-profile")]
const TIMER_EN: *mut u32 = 0xf000_2808 as *mut u32;
#[cfg(all(
    not(feature = "verilated-cfu-profile"),
    feature = "verilated-liteuart-profile"
))]
const TIMER_EN: *mut u32 = 0xe000_2808 as *mut u32;
#[cfg(all(
    not(feature = "verilated-cfu-profile"),
    not(feature = "verilated-liteuart-profile")
))]
const TIMER_EN: *mut u32 = 0x6000_2808 as *mut u32;
#[cfg(feature = "verilated-cfu-profile")]
const TIMER_EV_STATUS: *const u32 = 0xf000_2814 as *const u32;
#[cfg(all(
    not(feature = "verilated-cfu-profile"),
    feature = "verilated-liteuart-profile"
))]
const TIMER_EV_STATUS: *const u32 = 0xe000_2814 as *const u32;
#[cfg(all(
    not(feature = "verilated-cfu-profile"),
    not(feature = "verilated-liteuart-profile")
))]
const TIMER_EV_STATUS: *const u32 = 0x6000_2814 as *const u32;
#[cfg(feature = "verilated-cfu-profile")]
const TIMER_EV_PENDING: *mut u32 = 0xf000_2818 as *mut u32;
#[cfg(all(
    not(feature = "verilated-cfu-profile"),
    feature = "verilated-liteuart-profile"
))]
const TIMER_EV_PENDING: *mut u32 = 0xe000_2818 as *mut u32;
#[cfg(all(
    not(feature = "verilated-cfu-profile"),
    not(feature = "verilated-liteuart-profile")
))]
const TIMER_EV_PENDING: *mut u32 = 0x6000_2818 as *mut u32;

/// The LiteX SPI-flash bitbang CSRs at 0x60005000: bitbang (+0x0), miso
/// (+0x4), bitbang_en (+0x8). The imported Zephyr and MicroPython profiles
/// build with `no-unmodelled-spi-probe` and do not map or access the window.
#[cfg(not(feature = "no-unmodelled-spi-probe"))]
mod spiflash {
    const BITBANG: *mut u32 = 0x6000_5000 as *mut u32;
    const MISO: *const u32 = 0x6000_5004 as *const u32;
    const BITBANG_EN: *mut u32 = 0x6000_5008 as *mut u32;
    const CLK: u32 = 1 << 1;
    const CS_N: u32 = 1 << 2;
    const DIR: u32 = 1 << 3;

    fn bb(value: u32) {
        unsafe { core::ptr::write_volatile(BITBANG, value) }
    }

    fn send(byte: u8) {
        for bit in (0..8).rev() {
            let mosi = ((byte >> bit) & 1) as u32;
            bb(mosi);
            bb(mosi | CLK);
        }
        bb(0);
    }

    /// LiteX BIOS order: a dummy byte fetches the flash's answer, then eight
    /// input-phase (DIR) clocks shift it out, MISO sampled while CLK is high.
    fn receive() -> u8 {
        send(0);
        let mut value = 0u8;
        for _ in 0..8 {
            bb(DIR);
            bb(DIR | CLK);
            value = (value << 1) | (unsafe { core::ptr::read_volatile(MISO) } & 1) as u8;
        }
        bb(0);
        value
    }

    /// One CS-framed command; returns up to `N` answer bytes.
    pub fn command<const N: usize>(cmd: u8) -> [u8; N] {
        unsafe { core::ptr::write_volatile(BITBANG_EN, 1) };
        bb(CS_N);
        bb(0);
        send(cmd);
        let mut answer = [0u8; N];
        for byte in answer.iter_mut() {
            *byte = receive();
        }
        bb(CS_N);
        unsafe { core::ptr::write_volatile(BITBANG_EN, 0) };
        answer
    }
}

/// TX one byte through the LiteX UART: poll `txfull` until clear (idle 0 =
/// ready), then write `rxtx`.
fn putc(byte: u8) {
    unsafe {
        while core::ptr::read_volatile(UART_TXFULL) != 0 {}
        core::ptr::write_volatile(UART_RXTX, byte as u32);
    }
}

fn puts(text: &str) {
    for byte in text.bytes() {
        putc(byte);
    }
}

/// Fixed-width lowercase hex, MSB first.
fn put_hex32(value: u32) {
    for shift in (0..8).rev() {
        let nibble = ((value >> (shift * 4)) & 0xF) as u8;
        putc(match nibble {
            0..=9 => b'0' + nibble,
            _ => b'a' + nibble - 10,
        });
    }
}

#[cfg(not(feature = "no-unmodelled-spi-probe"))]
fn put_hex8(value: u8) {
    for nibble in [value >> 4, value & 0xF] {
        putc(match nibble {
            0..=9 => b'0' + nibble,
            _ => b'a' + nibble - 10,
        });
    }
}

/// Keep the M-extension check explicit so constant folding cannot replace it.
#[cfg(any(
    feature = "verilated-liteuart-profile",
    feature = "verilated-cfu-profile"
))]
#[inline(never)]
fn multiply(left: u32, right: u32) -> u32 {
    let result: u32;
    unsafe {
        core::arch::asm!(
            "mul {result}, {left}, {right}",
            result = lateout(reg) result,
            left = in(reg) left,
            right = in(reg) right,
            options(nomem, nostack, preserves_flags),
        );
    }
    result
}

fn put_dec32(value: u32) {
    let mut digits = [0u8; 10];
    let mut remaining = value;
    let mut start = digits.len();
    loop {
        start -= 1;
        digits[start] = b'0' + (remaining % 10) as u8;
        remaining /= 10;
        if remaining == 0 {
            break;
        }
    }
    for &digit in &digits[start..] {
        putc(digit);
    }
}

/// A busy-wait the optimizer cannot delete: each iteration is an opaque
/// `nop`, and the function is never inlined.
#[inline(never)]
fn spin(iterations: u32) {
    for _ in 0..iterations {
        unsafe {
            core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
        }
    }
}

/// Run the LiteX timer as a one-shot and wait for its zero event. The event
/// only latches if the countdown actually ran, so `timer=zero` is evidence
/// for the model rather than decoration.
fn timer_one_shot(cycles: u32) {
    unsafe {
        // Clear any stale event, then stage and arm a one-shot.
        core::ptr::write_volatile(TIMER_EV_PENDING, 1);
        core::ptr::write_volatile(TIMER_LOAD, cycles);
        core::ptr::write_volatile(TIMER_RELOAD, 0);
        core::ptr::write_volatile(TIMER_EN, 1);

        // ev_status is the event level (`counter == 0`): clear means the
        // countdown is live.
        if core::ptr::read_volatile(TIMER_EV_STATUS) & 1 == 0 {
            puts("timer=running\n");
        } else {
            puts("timer=stuck\n");
        }

        while core::ptr::read_volatile(TIMER_EV_PENDING) & 1 == 0 {}
        puts("timer=zero\n");
    }
}

#[entry]
fn main() -> ! {
    puts("LITEX VEXRISCV SMOKE\n");
    #[cfg(any(
        feature = "verilated-liteuart-profile",
        feature = "verilated-cfu-profile"
    ))]
    {
        puts("cpu=rv32im uart=litex-csr-model\n");
        puts("mul=");
        put_hex32(multiply(17, 19));
        putc(b'\n');
    }
    #[cfg(not(feature = "verilated-liteuart-profile"))]
    puts("cpu=rv32im uart=litex\n");

    // SPI flash over the controller's bitbang CSRs (generic LiteX profile).
    #[cfg(not(feature = "no-unmodelled-spi-probe"))]
    {
        let id: [u8; 3] = spiflash::command(0x9F);
        puts("spi id=");
        for byte in id {
            put_hex8(byte);
        }
        let [before]: [u8; 1] = spiflash::command(0x05);
        let _: [u8; 0] = spiflash::command(0x06);
        let [after]: [u8; 1] = spiflash::command(0x05);
        let _: [u8; 0] = spiflash::command(0x04);
        puts(" sr=");
        put_hex8(before);
        puts(" wel=");
        put_hex8(after);
        putc(b'\n');
    }

    timer_one_shot(100_000);

    let mut heartbeat: u32 = 0;
    loop {
        puts("hb=");
        put_dec32(heartbeat);
        puts(" txfull=");
        put_hex32(unsafe { core::ptr::read_volatile(UART_TXFULL) });
        putc(b'\n');
        spin(400_000);
        heartbeat = heartbeat.wrapping_add(1);
    }
}
