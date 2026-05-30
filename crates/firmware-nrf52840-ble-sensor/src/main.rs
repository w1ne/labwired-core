// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT
//
// Periodic BLE "sensor" transmitter for the playground two-nRF demo AND
// for real-silicon parity. The SAME ELF runs in the LabWired simulator
// and flashes to a physical nRF52840 (e.g. nRF52840-DK over J-Link).
//
// Companion to firmware-nrf52840-ble-collector. Starts the HFXO crystal,
// configures the RADIO for BLE_1Mbit on FREQUENCY=42 with logical
// address 0 = BASE0 | PREFIX0[0], then loops forever: bump a 1-byte
// reading, build a 4-byte packet (S0 + LENGTH + payload) at PACKETPTR,
// run TASKS_TXEN -> READY -> TASKS_START -> END -> TASKS_DISABLE ->
// DISABLED, wait a beat, repeat.
//
// HFXO note: real nRF silicon will not leave TXRU (EVENTS_READY never
// fires) until the 64 MHz crystal oscillator is running, so we start it
// up front. The simulator's CLOCK model fires EVENTS_HFCLKSTARTED on the
// next tick, so this same step is a no-op-cost on the model — making the
// binary behave identically on both. That is the parity guarantee.
//
// The reading is a self-incrementing counter (not the TEMP peripheral)
// so observers can verify monotonic progress on both model and silicon;
// on hardware, read TX_DONE_COUNT / TX_LAST_VALUE over the debug port.

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
const RADIO_TASKS_TXEN: *mut u32 = RADIO_BASE as *mut u32;
const RADIO_TASKS_START: *mut u32 = (RADIO_BASE + 0x008) as *mut u32;
const RADIO_TASKS_DISABLE: *mut u32 = (RADIO_BASE + 0x010) as *mut u32;
const RADIO_EVENTS_READY: *mut u32 = (RADIO_BASE + 0x100) as *mut u32;
const RADIO_EVENTS_END: *mut u32 = (RADIO_BASE + 0x10C) as *mut u32;
const RADIO_EVENTS_DISABLED: *mut u32 = (RADIO_BASE + 0x110) as *mut u32;
const RADIO_PACKETPTR: *mut u32 = (RADIO_BASE + 0x504) as *mut u32;
const RADIO_FREQUENCY: *mut u32 = (RADIO_BASE + 0x508) as *mut u32;
const RADIO_MODE: *mut u32 = (RADIO_BASE + 0x510) as *mut u32;
const RADIO_PCNF0: *mut u32 = (RADIO_BASE + 0x514) as *mut u32;
const RADIO_PCNF1: *mut u32 = (RADIO_BASE + 0x518) as *mut u32;
const RADIO_BASE0: *mut u32 = (RADIO_BASE + 0x51C) as *mut u32;
const RADIO_PREFIX0: *mut u32 = (RADIO_BASE + 0x524) as *mut u32;
const RADIO_TXADDRESS: *mut u32 = (RADIO_BASE + 0x52C) as *mut u32;
const RADIO_CRCINIT: *mut u32 = (RADIO_BASE + 0x53C) as *mut u32;
const RADIO_CRCCNF: *mut u32 = (RADIO_BASE + 0x534) as *mut u32;
const RADIO_CRCPOLY: *mut u32 = (RADIO_BASE + 0x538) as *mut u32;
const RADIO_DATAWHITEIV: *mut u32 = (RADIO_BASE + 0x554) as *mut u32;

const PACKET_RAM_ADDR: u32 = 0x2000_2000;
// Payload byte offset within the PACKETPTR buffer: [S0, LENGTH, payload..].
const PAYLOAD_OFF: u32 = 2;

/// Approx instructions to spin between broadcasts so observers see a
/// steady stream. At ~5000 instr/frame in the playground this is a
/// sub-second cadence; on 64 MHz silicon it is a few ms.
const TX_INTERVAL_SPINS: u32 = 120_000;

/// Test-observable counter incremented after each successful transmit.
#[no_mangle]
static TX_DONE_COUNT: AtomicU32 = AtomicU32::new(0);
/// Last reading value placed on-air (test-observable).
#[no_mangle]
static TX_LAST_VALUE: AtomicU32 = AtomicU32::new(0);

#[entry]
fn main() -> ! {
    unsafe {
        // Start the HFXO — required on real silicon before RADIO TX.
        write_volatile(CLOCK_EVENTS_HFCLKSTARTED, 0);
        write_volatile(CLOCK_TASKS_HFCLKSTART, 1);
        while read_volatile(CLOCK_EVENTS_HFCLKSTARTED) == 0 {}

        // One-time packet header in RAM: S0=0xAB, LENGTH=4.
        (PACKET_RAM_ADDR as *mut u8).write_volatile(0xAB);
        (PACKET_RAM_ADDR as *mut u8).add(1).write_volatile(0x04);
        (PACKET_RAM_ADDR as *mut u8).add(3).write_volatile(0x00);
        (PACKET_RAM_ADDR as *mut u8).add(4).write_volatile(0x00);
        (PACKET_RAM_ADDR as *mut u8).add(5).write_volatile(0x00);

        // Configure RADIO once: BLE_1Mbit at FREQUENCY=42.
        write_volatile(RADIO_MODE, 3);
        write_volatile(RADIO_FREQUENCY, 42);
        // PCNF0: S0LEN=1 byte, LFLEN=8 bits.
        write_volatile(RADIO_PCNF0, 8 | (1 << 8));
        // PCNF1: MAXLEN=255, WHITEEN=1.
        write_volatile(RADIO_PCNF1, 0xFF | (1 << 25));
        write_volatile(RADIO_BASE0, 0xCAFE_BA00);
        write_volatile(RADIO_PREFIX0, 0xBE);
        write_volatile(RADIO_TXADDRESS, 0);
        // CRC: 3 bytes, BLE poly/init (matches the RADIO model + a real
        // BLE receiver). CRCCNF.LEN=3, skip address bytes.
        write_volatile(RADIO_CRCCNF, 3);
        write_volatile(RADIO_CRCPOLY, 0x0000_065B);
        write_volatile(RADIO_CRCINIT, 0x555555);
        write_volatile(RADIO_DATAWHITEIV, 42);
        write_volatile(RADIO_PACKETPTR, PACKET_RAM_ADDR);

        let mut reading: u8 = 0;
        loop {
            // Update the first payload byte with the current reading.
            (PACKET_RAM_ADDR as *mut u8)
                .add(PAYLOAD_OFF as usize)
                .write_volatile(reading);

            // Enable TX, wait for READY.
            write_volatile(RADIO_TASKS_TXEN, 1);
            while read_volatile(RADIO_EVENTS_READY) == 0 {}
            write_volatile(RADIO_EVENTS_READY, 0);

            // Transmit, wait for END.
            write_volatile(RADIO_TASKS_START, 1);
            while read_volatile(RADIO_EVENTS_END) == 0 {}
            write_volatile(RADIO_EVENTS_END, 0);

            // Disable, wait for DISABLED so the next TXEN starts clean.
            write_volatile(RADIO_TASKS_DISABLE, 1);
            while read_volatile(RADIO_EVENTS_DISABLED) == 0 {}
            write_volatile(RADIO_EVENTS_DISABLED, 0);

            TX_LAST_VALUE.store(reading as u32, Ordering::Relaxed);
            TX_DONE_COUNT.fetch_add(1, Ordering::Relaxed);
            reading = reading.wrapping_add(1);

            // Idle between broadcasts.
            for _ in 0..TX_INTERVAL_SPINS {
                cortex_m::asm::nop();
            }
        }
    }
}
