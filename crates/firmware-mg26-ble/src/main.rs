#![no_std]
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
#![no_main]
#![allow(clippy::empty_loop)]

//! BRD2709A BLE beacon — **twin only**.
//!
//! ⚠️ Unlike `firmware-mg26-demo`, this image does NOT run on the physical
//! board. It drives the LabWired virtual BLE controller at `0x4F00_0000`, and
//! there is no peripheral at that address on real EFR32MG26 silicon: an access
//! there bus-faults. That is deliberate and it is why this is a separate crate
//! rather than another step in the demo — the dual-target image stays
//! dual-target.
//!
//! The reason the controller is a LabWired device at all is that Silicon Labs
//! documents no radio register anywhere for this part: the xG26 Reference
//! Manual's radio chapter carries no register map, the CMSIS headers ship no
//! `rac`/`frc`/`modem`/`protimer` header, and there is no SVD. See
//! `crates/core/src/peripherals/virtual_ble.rs`.
//!
//! What it does: brings up the VCOM console exactly as the demo does, stages a
//! legacy non-connectable advertisement carrying manufacturer data, starts
//! advertising, then scans for a peer and prints whatever it hears. Two of
//! these in one lab find each other; so does one of these and an ESP32-C3,
//! because they share one virtual air.

// ── Console (identical to firmware-mg26-demo; see its header for sources) ──
const CMU_CLKEN0: *mut u32 = 0x4000_8064 as *mut u32;
const CMU_CLKEN2: *mut u32 = 0x4000_806C as *mut u32;
const CMU_CLKEN0_GPIO: u32 = 1 << 26;
const CMU_CLKEN2_USART1: u32 = 1 << 7;

const GPIOB_MODEL: *mut u32 = 0x4003_C064 as *mut u32;
const USART1_ROUTEEN: *mut u32 = 0x4003_C840 as *mut u32;
const USART1_TXROUTE: *mut u32 = 0x4003_C858 as *mut u32;
const GPIO_USART_ROUTEEN_TXPEN: u32 = 1 << 4;
const TXROUTE_PB02: u32 = 1 | (2 << 16);

const USART1_BASE: usize = 0x400A_4000;
const USART1_EN: *mut u32 = (USART1_BASE + 0x04) as *mut u32;
const USART1_CMD: *mut u32 = (USART1_BASE + 0x14) as *mut u32;
const USART1_STATUS: *const u32 = (USART1_BASE + 0x18) as *const u32;
const USART1_CLKDIV: *mut u32 = (USART1_BASE + 0x1C) as *mut u32;
const USART1_TXDATA: *mut u32 = (USART1_BASE + 0x38) as *mut u32;
const USART_CMD_TXEN: u32 = 1 << 2;
const USART_STATUS_TXBL: u32 = 1 << 6;
const USART1_CLKDIV_115200: u32 = 2384;
const MODE_PUSHPULL: u32 = 0x4;

// ── LabWired virtual BLE controller ───────────────────────────────────────
const BLE_BASE: usize = 0x4F00_0000;
const BLE_ID: *const u32 = BLE_BASE as *const u32;
const BLE_CTRL: *mut u32 = (BLE_BASE + 0x04) as *mut u32;
const BLE_STATUS: *const u32 = (BLE_BASE + 0x08) as *const u32;
const BLE_CHANNEL: *mut u32 = (BLE_BASE + 0x0C) as *mut u32;
const BLE_ADVINTERVAL: *mut u32 = (BLE_BASE + 0x18) as *mut u32;
const BLE_TXLEN: *mut u32 = (BLE_BASE + 0x24) as *mut u32;
const BLE_RXCMD: *mut u32 = (BLE_BASE + 0x2C) as *mut u32;
const BLE_RXLEN: *const u32 = (BLE_BASE + 0x30) as *const u32;
const BLE_TXBUF: usize = BLE_BASE + 0x100;
const BLE_RXBUF: usize = BLE_BASE + 0x200;

const BLE_CTRL_ADV_EN: u32 = 1 << 0;
const BLE_CTRL_SCAN_EN: u32 = 1 << 1;
const BLE_STATUS_RX_AVAIL: u32 = 1 << 0;
/// `"LWBL"` — the controller identifies itself as a LabWired device.
const LWBL_MAGIC: u32 = 0x4C42_574C;

/// 160 × 625 µs = 100 ms, the interval most beacon examples use.
const ADV_INTERVAL_100MS: u32 = 160;

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
    unsafe {
        while core::ptr::read_volatile(USART1_STATUS) & USART_STATUS_TXBL == 0 {}
        core::ptr::write_volatile(USART1_TXDATA, c as u32);
    }
}

fn puts(s: &str) {
    for &b in s.as_bytes() {
        putc(b);
    }
}

fn puthex(v: u8) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    putc(HEX[(v >> 4) as usize]);
    putc(HEX[(v & 0xF) as usize]);
}

fn console_init() {
    write_u32(CMU_CLKEN0, CMU_CLKEN0_GPIO);
    write_u32(CMU_CLKEN2, CMU_CLKEN2_USART1);
    write_u32(GPIOB_MODEL, MODE_PUSHPULL << 8);
    write_u32(USART1_TXROUTE, TXROUTE_PB02);
    write_u32(USART1_ROUTEEN, GPIO_USART_ROUTEEN_TXPEN);
    write_u32(USART1_EN, 1);
    write_u32(USART1_CLKDIV, USART1_CLKDIV_115200);
    write_u32(USART1_CMD, USART_CMD_TXEN);
}

/// Stage `pdu` in TXBUF and set TXLEN. Word writes, because the controller's
/// buffer is a word-addressed window like every other register block here.
fn stage_pdu(pdu: &[u8]) {
    let mut i = 0;
    while i < pdu.len() {
        let mut w = 0u32;
        let mut j = 0;
        while j < 4 && i + j < pdu.len() {
            w |= (pdu[i + j] as u32) << (j * 8);
            j += 1;
        }
        write_u32((BLE_TXBUF + i) as *mut u32, w);
        i += 4;
    }
    write_u32(BLE_TXLEN, pdu.len() as u32);
}

fn main() -> ! {
    console_init();
    puts("brd2709a: MG26 BLE\n");

    // The controller announces itself. A board where this reads anything else
    // has no virtual BLE device mapped, and the rest of this image is
    // meaningless — say so rather than advertising into nowhere.
    let id = read_u32(BLE_ID);
    if id != LWBL_MAGIC {
        puts("BLE NO CONTROLLER\n");
        loop {}
    }
    puts("BLE ID LWBL\n");

    // A legacy ADV_NONCONN_IND: PDU type 0x02, then a 12-byte body — six
    // address bytes and one manufacturer-specific AD structure.
    //   AdvA          : 02:09:26:00:27:09  (locally administered)
    //   AD 05 FF ...  : company 0x02E5, then a two-byte payload
    let adv: [u8; 14] = [
        0x02, 0x0C, // header, length
        0x02, 0x09, 0x26, 0x00, 0x27, 0x09, // AdvA, little-endian
        0x05, 0xFF, 0xE5, 0x02, 0x26, 0x01, // len, type=manufacturer, 02E5, payload
    ];
    stage_pdu(&adv);
    write_u32(BLE_ADVINTERVAL, ADV_INTERVAL_100MS);
    write_u32(BLE_CTRL, BLE_CTRL_ADV_EN);
    puts("BLE ADV\n");

    // Listen on channel 37 for a peer's advertisement while we advertise. The
    // controller never hands us back our own transmission.
    write_u32(BLE_CHANNEL, 37);
    write_u32(BLE_CTRL, BLE_CTRL_ADV_EN | BLE_CTRL_SCAN_EN);
    puts("BLE SCAN\n");

    loop {
        if read_u32(BLE_STATUS) & BLE_STATUS_RX_AVAIL != 0 {
            write_u32(BLE_RXCMD, 1);
            let len = read_u32(BLE_RXLEN) as usize;
            puts("BLE RX ");
            let mut i = 0;
            while i < len {
                let word = read_u32((BLE_RXBUF + (i & !3)) as *const u32);
                puthex(((word >> ((i & 3) * 8)) & 0xFF) as u8);
                i += 1;
            }
            putc(b'\n');
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
