// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT
//
// Raw-RADIO BLE transmitter. Configures RADIO for BLE_1Mbit on
// FREQUENCY=42 with logical address 0 = BASE0 | PREFIX0[0], builds a
// 6-byte packet (S0 + LENGTH + 4-byte payload) at PACKETPTR, then
// drives the TASKS_TXEN → ready → TASKS_START → EVENTS_END cycle.
// After the packet is transmitted, sets STATUS_FLAG so the test can
// observe completion.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicU32, Ordering};
use cortex_m_rt::entry;
use panic_halt as _;

const RADIO_BASE: usize = 0x4000_1000;
const RADIO_TASKS_TXEN: *mut u32 = RADIO_BASE as *mut u32;
const RADIO_TASKS_START: *mut u32 = (RADIO_BASE + 0x008) as *mut u32;
const RADIO_TASKS_DISABLE: *mut u32 = (RADIO_BASE + 0x010) as *mut u32;
const RADIO_EVENTS_READY: *mut u32 = (RADIO_BASE + 0x100) as *mut u32;
const RADIO_EVENTS_END: *mut u32 = (RADIO_BASE + 0x10C) as *mut u32;
const RADIO_PACKETPTR: *mut u32 = (RADIO_BASE + 0x504) as *mut u32;
const RADIO_FREQUENCY: *mut u32 = (RADIO_BASE + 0x508) as *mut u32;
const RADIO_MODE: *mut u32 = (RADIO_BASE + 0x510) as *mut u32;
const RADIO_PCNF0: *mut u32 = (RADIO_BASE + 0x514) as *mut u32;
const RADIO_PCNF1: *mut u32 = (RADIO_BASE + 0x518) as *mut u32;
const RADIO_BASE0: *mut u32 = (RADIO_BASE + 0x51C) as *mut u32;
const RADIO_PREFIX0: *mut u32 = (RADIO_BASE + 0x524) as *mut u32;
const RADIO_TXADDRESS: *mut u32 = (RADIO_BASE + 0x52C) as *mut u32;
const RADIO_CRCINIT: *mut u32 = (RADIO_BASE + 0x53C) as *mut u32;
const RADIO_DATAWHITEIV: *mut u32 = (RADIO_BASE + 0x554) as *mut u32;

const PACKET_RAM_ADDR: u32 = 0x2000_2000;
const PACKET_BYTES: [u8; 6] = [0xAB, 0x04, 0xC0, 0xDE, 0xCA, 0xFE];

/// Test-observable counter incremented after each successful transmit.
#[no_mangle]
static TX_DONE_COUNT: AtomicU32 = AtomicU32::new(0);

#[entry]
fn main() -> ! {
    unsafe {
        // Write the packet to RAM at PACKET_RAM_ADDR.
        for (i, b) in PACKET_BYTES.iter().enumerate() {
            (PACKET_RAM_ADDR as *mut u8).add(i).write_volatile(*b);
        }

        // Configure RADIO: BLE_1Mbit at FREQUENCY=42.
        write_volatile(RADIO_MODE, 3);
        write_volatile(RADIO_FREQUENCY, 42);
        // PCNF0: S0LEN=1 byte, LFLEN=8 bits.
        write_volatile(RADIO_PCNF0, 8 | (1 << 8));
        // PCNF1: MAXLEN=255, WHITEEN=1.
        write_volatile(RADIO_PCNF1, 0xFF | (1 << 25));
        write_volatile(RADIO_BASE0, 0xCAFE_BA00);
        write_volatile(RADIO_PREFIX0, 0xBE);
        write_volatile(RADIO_TXADDRESS, 0);
        write_volatile(RADIO_CRCINIT, 0x555555);
        write_volatile(RADIO_DATAWHITEIV, 42);
        write_volatile(RADIO_PACKETPTR, PACKET_RAM_ADDR);

        // Enable TX, wait for READY.
        write_volatile(RADIO_TASKS_TXEN, 1);
        while read_volatile(RADIO_EVENTS_READY) == 0 {}
        write_volatile(RADIO_EVENTS_READY, 0);

        // Transmit, wait for END.
        write_volatile(RADIO_TASKS_START, 1);
        while read_volatile(RADIO_EVENTS_END) == 0 {}
        write_volatile(RADIO_EVENTS_END, 0);

        // Disable.
        write_volatile(RADIO_TASKS_DISABLE, 1);
    }

    TX_DONE_COUNT.fetch_add(1, Ordering::Relaxed);

    // Idle.
    loop {
        cortex_m::asm::wfi();
    }
}
