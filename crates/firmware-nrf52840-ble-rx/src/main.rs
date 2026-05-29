// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT
//
// Raw-RADIO BLE receiver companion to firmware-nrf52840-ble-tx.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicU32, Ordering};
use cortex_m_rt::entry;
use panic_halt as _;

const RADIO_BASE: usize = 0x4000_1000;
const RADIO_TASKS_RXEN: *mut u32 = (RADIO_BASE + 0x004) as *mut u32;
const RADIO_TASKS_START: *mut u32 = (RADIO_BASE + 0x008) as *mut u32;
const RADIO_TASKS_DISABLE: *mut u32 = (RADIO_BASE + 0x010) as *mut u32;
const RADIO_EVENTS_READY: *mut u32 = (RADIO_BASE + 0x100) as *mut u32;
const RADIO_EVENTS_END: *mut u32 = (RADIO_BASE + 0x10C) as *mut u32;
const RADIO_CRCSTATUS: *mut u32 = (RADIO_BASE + 0x400) as *mut u32;
const RADIO_PACKETPTR: *mut u32 = (RADIO_BASE + 0x504) as *mut u32;
const RADIO_FREQUENCY: *mut u32 = (RADIO_BASE + 0x508) as *mut u32;
const RADIO_MODE: *mut u32 = (RADIO_BASE + 0x510) as *mut u32;
const RADIO_PCNF0: *mut u32 = (RADIO_BASE + 0x514) as *mut u32;
const RADIO_PCNF1: *mut u32 = (RADIO_BASE + 0x518) as *mut u32;
const RADIO_BASE0: *mut u32 = (RADIO_BASE + 0x51C) as *mut u32;
const RADIO_PREFIX0: *mut u32 = (RADIO_BASE + 0x524) as *mut u32;
const RADIO_RXADDRESSES: *mut u32 = (RADIO_BASE + 0x530) as *mut u32;
const RADIO_CRCINIT: *mut u32 = (RADIO_BASE + 0x53C) as *mut u32;
const RADIO_DATAWHITEIV: *mut u32 = (RADIO_BASE + 0x554) as *mut u32;

const PACKET_RAM_ADDR: u32 = 0x2000_3000;

#[no_mangle]
static RX_FIRST_PAYLOAD_BYTE: AtomicU32 = AtomicU32::new(0);
#[no_mangle]
static RX_LENGTH: AtomicU32 = AtomicU32::new(0);
#[no_mangle]
static RX_CRC_STATUS: AtomicU32 = AtomicU32::new(0);
#[no_mangle]
static RX_DONE_COUNT: AtomicU32 = AtomicU32::new(0);

#[entry]
fn main() -> ! {
    unsafe {
        write_volatile(RADIO_MODE, 3);
        write_volatile(RADIO_FREQUENCY, 42);
        write_volatile(RADIO_PCNF0, 8 | (1 << 8));
        write_volatile(RADIO_PCNF1, 0xFF | (1 << 25));
        write_volatile(RADIO_BASE0, 0xCAFE_BA00);
        write_volatile(RADIO_PREFIX0, 0xBE);
        write_volatile(RADIO_RXADDRESSES, 0x01);
        write_volatile(RADIO_CRCINIT, 0x555555);
        write_volatile(RADIO_DATAWHITEIV, 42);
        write_volatile(RADIO_PACKETPTR, PACKET_RAM_ADDR);

        write_volatile(RADIO_TASKS_RXEN, 1);
        while read_volatile(RADIO_EVENTS_READY) == 0 {}
        write_volatile(RADIO_EVENTS_READY, 0);

        write_volatile(RADIO_TASKS_START, 1);
        while read_volatile(RADIO_EVENTS_END) == 0 {}
        write_volatile(RADIO_EVENTS_END, 0);

        // Read out the bytes the radio Easy-DMA'd into RAM.
        let s0 = (PACKET_RAM_ADDR as *const u8).read_volatile();
        let length = (PACKET_RAM_ADDR as *const u8).add(1).read_volatile();
        let first = (PACKET_RAM_ADDR as *const u8).add(2).read_volatile();
        let crcok = read_volatile(RADIO_CRCSTATUS);

        let _ = s0; // unused but read to keep the compiler honest
        RX_LENGTH.store(length as u32, Ordering::Relaxed);
        RX_FIRST_PAYLOAD_BYTE.store(first as u32, Ordering::Relaxed);
        RX_CRC_STATUS.store(crcok, Ordering::Relaxed);

        write_volatile(RADIO_TASKS_DISABLE, 1);
    }

    RX_DONE_COUNT.fetch_add(1, Ordering::Relaxed);

    loop {
        cortex_m::asm::wfi();
    }
}
