#![no_std]
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
#![no_main]
#![allow(clippy::empty_loop)]

//! UART XMODEM-CRC bootloader for BRD2709A (EFR32MG26).
//!
//! Lives in the first 32 KiB of flash (`0x0800_0000`). After reset it knocks
//! for `LWBL` (or stays if BTN0 is held), receives a raw `.bin` over XMODEM-1K,
//! programs it at `APP_BASE` (`0x0800_8000`) via MSC, then jumps.
//!
//! Register facts: simplicity_sdk sisdk-2025.6 CMSIS headers (same set as
//! `firmware-mg26-demo`).

const APP_BASE: u32 = 0x0800_8000;
const FLASH_END: u32 = 0x0800_0000 + 3200 * 1024;
const PAGE_SIZE: u32 = 0x2000;
const KNOCK: [u8; 4] = *b"LWBL";

const CMU_CLKEN0: *mut u32 = 0x4000_8064 as *mut u32;
const CMU_CLKEN1: *mut u32 = 0x4000_8068 as *mut u32;
const CMU_CLKEN2: *mut u32 = 0x4000_806C as *mut u32;
const CMU_CLKEN0_GPIO: u32 = 1 << 26;
const CMU_CLKEN1_MSC: u32 = 1 << 16;
const CMU_CLKEN2_USART1: u32 = 1 << 7;

const GPIOB_MODEL: *mut u32 = 0x4003_C064 as *mut u32;
const GPIOB_DOUT: *mut u32 = 0x4003_C070 as *mut u32;
const GPIOB_DIN: *const u32 = 0x4003_C074 as *const u32;
const GPIOC_MODEH: *mut u32 = 0x4003_C09C as *mut u32;
const GPIOC_DOUT: *mut u32 = 0x4003_C0A0 as *mut u32;

const USART1_ROUTEEN: *mut u32 = 0x4003_C840 as *mut u32;
const USART1_RXROUTE: *mut u32 = 0x4003_C850 as *mut u32;
const USART1_TXROUTE: *mut u32 = 0x4003_C858 as *mut u32;
const GPIO_USART_ROUTEEN_RXPEN: u32 = 1 << 2;
const GPIO_USART_ROUTEEN_TXPEN: u32 = 1 << 4;
const TXROUTE_PB02: u32 = 1 | (2 << 16);
const RXROUTE_PB03: u32 = 1 | (3 << 16);

const USART1_BASE: usize = 0x400A_4000;
const USART1_EN: *mut u32 = (USART1_BASE + 0x04) as *mut u32;
const USART1_CMD: *mut u32 = (USART1_BASE + 0x14) as *mut u32;
const USART1_STATUS: *const u32 = (USART1_BASE + 0x18) as *const u32;
const USART1_CLKDIV: *mut u32 = (USART1_BASE + 0x1C) as *mut u32;
const USART1_RXDATA: *const u32 = (USART1_BASE + 0x24) as *const u32;
const USART1_TXDATA: *mut u32 = (USART1_BASE + 0x38) as *mut u32;
const USART_CMD_RXEN: u32 = 1 << 0;
const USART_CMD_TXEN: u32 = 1 << 2;
const USART_STATUS_TXBL: u32 = 1 << 6;
const USART_STATUS_RXDATAV: u32 = 1 << 7;
const USART1_CLKDIV_115200: u32 = 2384;

const MODE_INPUT: u32 = 0x1;
const MODE_INPUTPULL: u32 = 0x2;
const MODE_PUSHPULL: u32 = 0x4;

const MSC_BASE: usize = 0x4003_0000;
const MSC_WRITECTRL: *mut u32 = (MSC_BASE + 0x0C) as *mut u32;
const MSC_WRITECMD: *mut u32 = (MSC_BASE + 0x10) as *mut u32;
const MSC_ADDRB: *mut u32 = (MSC_BASE + 0x14) as *mut u32;
const MSC_WDATA: *mut u32 = (MSC_BASE + 0x18) as *mut u32;
const MSC_STATUS: *const u32 = (MSC_BASE + 0x1C) as *const u32;
const MSC_LOCK: *mut u32 = (MSC_BASE + 0x3C) as *mut u32;
const MSC_WRITECTRL_WREN: u32 = 1 << 0;
const MSC_WRITECMD_ERASEPAGE: u32 = 1 << 1;
const MSC_WRITECMD_WRITEEND: u32 = 1 << 2;
const MSC_STATUS_BUSY: u32 = 1 << 0;
const MSC_STATUS_INVADDR: u32 = 1 << 2;
const MSC_STATUS_WDATAREADY: u32 = 1 << 3;
const MSC_STATUS_PENDING: u32 = 1 << 5;
const MSC_LOCKKEY_UNLOCK: u32 = 0x1B71;

const SOH: u8 = 0x01;
const STX: u8 = 0x02;
const EOT: u8 = 0x04;
const ACK: u8 = 0x06;
const NAK: u8 = 0x15;
const CAN: u8 = 0x18;
const CRC_NAK: u8 = b'C';

const SCB_VTOR: *mut u32 = 0xE000_ED08 as *mut u32;

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn read_u32(p: *const u32) -> u32 {
    unsafe { core::ptr::read_volatile(p) }
}

fn write_u32(p: *mut u32, v: u32) {
    unsafe { core::ptr::write_volatile(p, v) }
}

fn putc(c: u8) {
    while read_u32(USART1_STATUS) & USART_STATUS_TXBL == 0 {}
    write_u32(USART1_TXDATA, c as u32);
}

fn puts(s: &str) {
    for &b in s.as_bytes() {
        putc(b);
    }
}

fn getc() -> u8 {
    while read_u32(USART1_STATUS) & USART_STATUS_RXDATAV == 0 {}
    (read_u32(USART1_RXDATA) & 0xff) as u8
}

fn getc_timeout(spins: u32) -> Option<u8> {
    for _ in 0..spins {
        if read_u32(USART1_STATUS) & USART_STATUS_RXDATAV != 0 {
            return Some((read_u32(USART1_RXDATA) & 0xff) as u8);
        }
    }
    None
}

fn crc16_xmodem(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn msc_wait_not_busy() -> bool {
    for _ in 0..2_000_000 {
        let s = read_u32(MSC_STATUS);
        if s & (MSC_STATUS_BUSY | MSC_STATUS_PENDING) == 0 {
            return s & MSC_STATUS_INVADDR == 0;
        }
    }
    false
}

fn msc_erase_page(addr: u32) -> bool {
    write_u32(MSC_LOCK, MSC_LOCKKEY_UNLOCK);
    write_u32(MSC_WRITECTRL, read_u32(MSC_WRITECTRL) | MSC_WRITECTRL_WREN);
    write_u32(MSC_ADDRB, addr);
    write_u32(MSC_WRITECMD, MSC_WRITECMD_ERASEPAGE);
    let ok = msc_wait_not_busy() && msc_wait_not_busy();
    write_u32(MSC_WRITECTRL, read_u32(MSC_WRITECTRL) & !MSC_WRITECTRL_WREN);
    ok
}

fn msc_write_words(addr: u32, words: &[u32]) -> bool {
    if words.is_empty() {
        return true;
    }
    write_u32(MSC_LOCK, MSC_LOCKKEY_UNLOCK);
    write_u32(MSC_WRITECTRL, read_u32(MSC_WRITECTRL) | MSC_WRITECTRL_WREN);
    write_u32(MSC_ADDRB, addr);
    if read_u32(MSC_STATUS) & MSC_STATUS_INVADDR != 0 {
        write_u32(MSC_WRITECTRL, read_u32(MSC_WRITECTRL) & !MSC_WRITECTRL_WREN);
        return false;
    }
    write_u32(MSC_WDATA, words[0]);
    for &w in &words[1..] {
        for _ in 0..100_000 {
            if read_u32(MSC_STATUS) & MSC_STATUS_WDATAREADY != 0 {
                break;
            }
        }
        write_u32(MSC_WDATA, w);
    }
    write_u32(MSC_WRITECMD, MSC_WRITECMD_WRITEEND);
    let ok = msc_wait_not_busy() && msc_wait_not_busy();
    write_u32(MSC_WRITECTRL, read_u32(MSC_WRITECTRL) & !MSC_WRITECTRL_WREN);
    ok
}

fn bring_up() {
    write_u32(CMU_CLKEN0, CMU_CLKEN0_GPIO);
    write_u32(CMU_CLKEN1, read_u32(CMU_CLKEN1) | CMU_CLKEN1_MSC);
    write_u32(CMU_CLKEN2, CMU_CLKEN2_USART1);
    // PB00 INPUTPULL (BTN0), PB02 PUSHPULL (TX), PB03 INPUT (RX)
    write_u32(
        GPIOB_MODEL,
        MODE_INPUTPULL | (MODE_PUSHPULL << 8) | (MODE_INPUT << 12),
    );
    write_u32(GPIOB_DOUT, 1); // pull-up on BTN0
    write_u32(USART1_TXROUTE, TXROUTE_PB02);
    write_u32(USART1_RXROUTE, RXROUTE_PB03);
    write_u32(
        USART1_ROUTEEN,
        GPIO_USART_ROUTEEN_TXPEN | GPIO_USART_ROUTEEN_RXPEN,
    );
    write_u32(USART1_EN, 1);
    write_u32(USART1_CLKDIV, USART1_CLKDIV_115200);
    write_u32(USART1_CMD, USART_CMD_TXEN | USART_CMD_RXEN);
    write_u32(GPIOC_MODEH, MODE_PUSHPULL); // LED0 PC08
    write_u32(GPIOC_DOUT, 1 << 8);
}

fn btn0_held() -> bool {
    read_u32(GPIOB_DIN) & 1 == 0
}

fn app_valid() -> bool {
    let sp = unsafe { core::ptr::read_volatile(APP_BASE as *const u32) };
    let pc = unsafe { core::ptr::read_volatile((APP_BASE + 4) as *const u32) };
    let ram_ok = (0x2000_0000..0x2008_0001).contains(&sp);
    let thumb = pc & 1 == 1;
    let in_app = (APP_BASE..FLASH_END).contains(&(pc & !1));
    ram_ok && thumb && in_app
}

fn jump_to_app() -> ! {
    unsafe {
        core::ptr::write_volatile(SCB_VTOR, APP_BASE);
        let sp = core::ptr::read_volatile(APP_BASE as *const u32);
        let pc = core::ptr::read_volatile((APP_BASE + 4) as *const u32);
        core::arch::asm!(
            "cpsid i",
            "msr msp, {sp}",
            "bx {pc}",
            sp = in(reg) sp,
            pc = in(reg) pc,
            options(noreturn)
        );
    }
}

fn wait_knock() -> bool {
    if btn0_held() {
        return true;
    }
    let mut idx = 0usize;
    // ~2 s of polling at 19 MHz
    for _ in 0..4_000_000u32 {
        if btn0_held() {
            return true;
        }
        if let Some(b) = getc_timeout(8) {
            if b == KNOCK[idx] {
                idx += 1;
                if idx == KNOCK.len() {
                    return true;
                }
            } else {
                idx = if b == KNOCK[0] { 1 } else { 0 };
            }
        }
    }
    false
}

fn receive_and_program() -> bool {
    puts("LWBL ready\n");
    putc(CRC_NAK);

    let mut expected: u8 = 1;
    let mut dst = APP_BASE;
    let mut page = [0u8; PAGE_SIZE as usize];
    let mut page_fill: usize = 0;
    let mut got_any = false;

    let flush = |page: &mut [u8; PAGE_SIZE as usize], fill: &mut usize, dst: &mut u32| -> bool {
        if *fill == 0 {
            return true;
        }
        for b in page[*fill..].iter_mut() {
            *b = 0xff;
        }
        if *dst + PAGE_SIZE > FLASH_END {
            return false;
        }
        if !msc_erase_page(*dst) {
            return false;
        }
        let mut words = [0u32; (PAGE_SIZE / 4) as usize];
        for (i, w) in words.iter_mut().enumerate() {
            let o = i * 4;
            *w = u32::from_le_bytes([page[o], page[o + 1], page[o + 2], page[o + 3]]);
        }
        if !msc_write_words(*dst, &words) {
            return false;
        }
        *dst += PAGE_SIZE;
        *fill = 0;
        true
    };

    loop {
        let mark = match getc_timeout(20_000_000) {
            Some(b) => b,
            None => {
                putc(CRC_NAK);
                continue;
            }
        };
        if mark == EOT {
            if !got_any {
                putc(NAK);
                continue;
            }
            if !flush(&mut page, &mut page_fill, &mut dst) {
                putc(CAN);
                return false;
            }
            putc(ACK);
            return true;
        }
        if mark == CAN {
            return false;
        }
        let block_size: usize = match mark {
            STX => 1024,
            SOH => 128,
            _ => continue,
        };
        let blk = getc();
        let nblk = getc();
        let mut buf = [0u8; 1024];
        for b in buf.iter_mut().take(block_size) {
            *b = getc();
        }
        let crc_hi = getc();
        let crc_lo = getc();
        let crc = ((crc_hi as u16) << 8) | crc_lo as u16;
        if ((blk.wrapping_add(nblk)) != 0xff) || crc != crc16_xmodem(&buf[..block_size]) {
            putc(NAK);
            continue;
        }
        if blk == expected.wrapping_sub(1) {
            // duplicate — ACK and ignore
            putc(ACK);
            continue;
        }
        if blk != expected {
            putc(NAK);
            continue;
        }
        for &b in &buf[..block_size] {
            if page_fill == page.len() && !flush(&mut page, &mut page_fill, &mut dst) {
                putc(CAN);
                return false;
            }
            page[page_fill] = b;
            page_fill += 1;
        }
        expected = expected.wrapping_add(1);
        got_any = true;
        putc(ACK);
    }
}

fn main() -> ! {
    bring_up();
    if wait_knock() {
        write_u32(GPIOC_DOUT, 1 << 8);
        if receive_and_program() {
            puts("OK\n");
        } else {
            puts("ERR\n");
            loop {}
        }
    }
    if app_valid() {
        jump_to_app();
    }
    puts("NOAPP\n");
    loop {
        write_u32(GPIOC_DOUT, 1 << 8);
        for _ in 0..200_000 {
            core::hint::spin_loop();
        }
        write_u32(GPIOC_DOUT, 0);
        for _ in 0..200_000 {
            core::hint::spin_loop();
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
