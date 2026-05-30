// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT
//
// Periodic BLE "collector" receiver for the playground two-nRF demo and
// for real-silicon parity. Companion to firmware-nrf52840-ble-sensor.
//
// Starts the HFXO crystal (required on real nRF silicon before the RADIO
// will receive; a no-op-cost on the simulator's CLOCK model), configures
// the RADIO to receive BLE_1Mbit on FREQUENCY=42 with logical address
// 0 = BASE0 | PREFIX0[0], then loops forever: arm RX, block on
// EVENTS_END until a matching frame arrives, read the reading out of the
// Easy-DMA'd packet buffer, disable, re-arm.
//
// On hardware, read RX_LAST_VALUE / RX_DONE_COUNT over the debug port.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicU32, Ordering};
use cortex_m_rt::entry;
use panic_halt as _;

const CLOCK_BASE: usize = 0x4000_0000;
const CLOCK_TASKS_HFCLKSTART: *mut u32 = CLOCK_BASE as *mut u32;
const CLOCK_EVENTS_HFCLKSTARTED: *mut u32 = (CLOCK_BASE + 0x100) as *mut u32;

const RADIO_BASE: usize = 0x4000_1000;
const RADIO_TASKS_RXEN: *mut u32 = (RADIO_BASE + 0x004) as *mut u32;
const RADIO_TASKS_START: *mut u32 = (RADIO_BASE + 0x008) as *mut u32;
const RADIO_TASKS_DISABLE: *mut u32 = (RADIO_BASE + 0x010) as *mut u32;
const RADIO_EVENTS_READY: *mut u32 = (RADIO_BASE + 0x100) as *mut u32;
const RADIO_EVENTS_END: *mut u32 = (RADIO_BASE + 0x10C) as *mut u32;
const RADIO_EVENTS_DISABLED: *mut u32 = (RADIO_BASE + 0x110) as *mut u32;
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
const RADIO_CRCCNF: *mut u32 = (RADIO_BASE + 0x534) as *mut u32;
const RADIO_CRCPOLY: *mut u32 = (RADIO_BASE + 0x538) as *mut u32;
const RADIO_DATAWHITEIV: *mut u32 = (RADIO_BASE + 0x554) as *mut u32;

const PACKET_RAM_ADDR: u32 = 0x2000_3000;
// Payload byte offset within the PACKETPTR buffer: [S0, LENGTH, payload..].
const PAYLOAD_OFF: u32 = 2;

/// Last reading received (test-observable).
#[no_mangle]
static RX_LAST_VALUE: AtomicU32 = AtomicU32::new(0);
/// LENGTH field of the most recent frame (test-observable).
#[no_mangle]
static RX_LENGTH: AtomicU32 = AtomicU32::new(0);
/// CRCSTATUS of the most recent frame (1 = OK).
#[no_mangle]
static RX_CRC_STATUS: AtomicU32 = AtomicU32::new(0);
/// Count of frames received (test-observable).
#[no_mangle]
static RX_DONE_COUNT: AtomicU32 = AtomicU32::new(0);

#[entry]
fn main() -> ! {
    unsafe {
        // Start the HFXO — required on real silicon before RADIO RX.
        write_volatile(CLOCK_EVENTS_HFCLKSTARTED, 0);
        write_volatile(CLOCK_TASKS_HFCLKSTART, 1);
        while read_volatile(CLOCK_EVENTS_HFCLKSTARTED) == 0 {}

        // Configure RADIO once to match the sensor's on-air parameters.
        write_volatile(RADIO_MODE, 3);
        write_volatile(RADIO_FREQUENCY, 42);
        write_volatile(RADIO_PCNF0, 8 | (1 << 8));
        write_volatile(RADIO_PCNF1, 0xFF | (1 << 25));
        write_volatile(RADIO_BASE0, 0xCAFE_BA00);
        write_volatile(RADIO_PREFIX0, 0xBE);
        write_volatile(RADIO_RXADDRESSES, 0x01);
        write_volatile(RADIO_CRCCNF, 3);
        write_volatile(RADIO_CRCPOLY, 0x0000_065B);
        write_volatile(RADIO_CRCINIT, 0x555555);
        write_volatile(RADIO_DATAWHITEIV, 42);
        write_volatile(RADIO_PACKETPTR, PACKET_RAM_ADDR);

        loop {
            // Arm RX, wait for READY.
            write_volatile(RADIO_TASKS_RXEN, 1);
            while read_volatile(RADIO_EVENTS_READY) == 0 {}
            write_volatile(RADIO_EVENTS_READY, 0);

            // Start receiving; blocks here until a matching frame is
            // dequeued and EVENTS_END fires.
            write_volatile(RADIO_TASKS_START, 1);
            while read_volatile(RADIO_EVENTS_END) == 0 {}
            write_volatile(RADIO_EVENTS_END, 0);

            // Read the bytes the radio Easy-DMA'd into RAM.
            let length = (PACKET_RAM_ADDR as *const u8).add(1).read_volatile();
            let value = (PACKET_RAM_ADDR as *const u8)
                .add(PAYLOAD_OFF as usize)
                .read_volatile();
            let crcok = read_volatile(RADIO_CRCSTATUS);

            RX_LENGTH.store(length as u32, Ordering::Relaxed);
            RX_LAST_VALUE.store(value as u32, Ordering::Relaxed);
            RX_CRC_STATUS.store(crcok, Ordering::Relaxed);
            RX_DONE_COUNT.fetch_add(1, Ordering::Relaxed);

            // Disable, wait for DISABLED, then loop to re-arm RX.
            write_volatile(RADIO_TASKS_DISABLE, 1);
            while read_volatile(RADIO_EVENTS_DISABLED) == 0 {}
            write_volatile(RADIO_EVENTS_DISABLED, 0);
        }
    }
}
