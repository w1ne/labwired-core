// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT
//
// nRF52840 HC-SR04 ultrasonic proximity firmware, with BLE telemetry.
//
// Wiring (matches the lab's system.yaml):
//   TRIG  = P0.04  (MCU output -> sensor)
//   ECHO  = P0.05  (sensor -> MCU input)
//   ALARM = P0.06  (MCU output -> buzzer/LED; the "in range" flag)
//
// Loop: pulse TRIG ~10 us, wait for the ECHO rising edge, time how long ECHO
// stays high, convert that to a distance, raise ALARM when within THRESHOLD_MM,
// and broadcast {distance_mm, in_range, counter} over the RADIO as a BLE-shaped
// packet so a phone / BLE analyzer can read it live.
//
// Pure nRF register access (no LabWired APIs): the same ELF runs in the
// LabWired simulator and flashes to real nRF52840 silicon.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// nRF52840 GPIO port 0 base address. TRIG/ECHO/ALARM all live on P0.
const GPIO0: usize = 0x5000_0000;

// nRF GPIO register offsets (Nordic nRF52840 PS v1.7 §6.10).
const OUT: usize = 0x504;
const OUTSET: usize = 0x508;
const OUTCLR: usize = 0x50C;
const IN: usize = 0x510;
const DIRSET: usize = 0x518;

const TRIG_BIT: u32 = 4; // P0.04
const ECHO_BIT: u32 = 5; // P0.05
const ALARM_BIT: u32 = 6; // P0.06

const TRIG: u32 = 1 << TRIG_BIT;
const ECHO: u32 = 1 << ECHO_BIT;
const ALARM: u32 = 1 << ALARM_BIT;

// ── BLE / RADIO (raw, BLE_1Mbit, mirrors firmware-nrf52840-ble-sensor) ───────
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
const RADIO_CRCCNF: *mut u32 = (RADIO_BASE + 0x534) as *mut u32;
const RADIO_CRCPOLY: *mut u32 = (RADIO_BASE + 0x538) as *mut u32;
const RADIO_CRCINIT: *mut u32 = (RADIO_BASE + 0x53C) as *mut u32;
const RADIO_DATAWHITEIV: *mut u32 = (RADIO_BASE + 0x554) as *mut u32;

// BLE packet buffer in RAM: [S0, LENGTH, payload...]. Payload is 4 bytes:
// distance_mm (u16 LE), in_range (u8), sample counter (u8).
const PACKET_RAM_ADDR: u32 = 0x2000_2000;
const PAYLOAD_OFF: usize = 2;
const BLE_PAYLOAD_LEN: u8 = 4;

/// Target distance: raise ALARM when the measured distance is at or below this.
/// 500 mm (50 cm) sits in the middle of the HC-SR04's useful range, so dragging
/// the host "hand distance" down past it visibly toggles the ALARM.
const THRESHOLD_MM: u32 = 500;

/// ECHO-high tick count corresponding to THRESHOLD_MM. The ECHO pulse the
/// simulator drives is strictly proportional to distance (it holds ECHO high for
/// `distance_cm × 58 µs`), so this loop's iteration count is linear in distance:
/// the original calibration measured 8950 ticks at 150 mm (cpu_hz = 64 MHz), i.e.
/// 59.67 ticks/mm. 500 mm × 59.67 = 29833 ticks. Keeping ticks/mm identical leaves
/// the reported `distance_mm` calibrated; only the in-range boundary moves to
/// 50 cm. Deterministic for a given HC-SR04 `cpu_hz`. See README "Calibration".
const THRESHOLD_TICKS: u32 = 29833;

/// Safety bound so a missing/!wired ECHO never hangs the firmware.
const MAX_TICKS: u32 = 4_000_000;

// Observable results (read by the lab via these ELF symbols). `#[no_mangle]`
// keeps the names stable so `--watch-mem`/memory assertions can target them.
#[no_mangle]
pub static mut LAST_TICKS: u32 = 0;
#[no_mangle]
pub static mut DISTANCE_MM: u32 = 0;
#[no_mangle]
pub static mut IN_RANGE: u32 = 0;
#[no_mangle]
pub static mut SAMPLE_COUNT: u32 = 0;
/// Successful BLE transmits (test-observable: proves the RADIO actually ran).
#[no_mangle]
pub static mut TX_DONE_COUNT: u32 = 0;

#[inline(always)]
unsafe fn wr(base: usize, off: usize, val: u32) {
    write_volatile((base + off) as *mut u32, val);
}

#[inline(always)]
unsafe fn rd(base: usize, off: usize) -> u32 {
    read_volatile((base + off) as *const u32)
}

#[entry]
fn main() -> ! {
    unsafe {
        // TRIG + ALARM are P0 outputs; ECHO stays an input (DIR=0 by reset).
        wr(GPIO0, DIRSET, TRIG | ALARM);
        wr(GPIO0, OUTCLR, TRIG | ALARM);

        // ── Bring up the RADIO once (BLE_1Mbit, FREQUENCY=42) ────────────────
        write_volatile(CLOCK_EVENTS_HFCLKSTARTED, 0);
        write_volatile(CLOCK_TASKS_HFCLKSTART, 1);
        while read_volatile(CLOCK_EVENTS_HFCLKSTARTED) == 0 {}

        (PACKET_RAM_ADDR as *mut u8).write_volatile(0xAB); // S0
        (PACKET_RAM_ADDR as *mut u8)
            .add(1)
            .write_volatile(BLE_PAYLOAD_LEN); // LENGTH

        write_volatile(RADIO_MODE, 3); // BLE_1Mbit
        write_volatile(RADIO_FREQUENCY, 42);
        write_volatile(RADIO_PCNF0, 8 | (1 << 8)); // LFLEN=8, S0LEN=1
        write_volatile(RADIO_PCNF1, 0xFF | (1 << 25)); // MAXLEN=255, WHITEEN=1
        write_volatile(RADIO_BASE0, 0xCAFE_BA00);
        write_volatile(RADIO_PREFIX0, 0xBE);
        write_volatile(RADIO_TXADDRESS, 0);
        write_volatile(RADIO_CRCCNF, 3);
        write_volatile(RADIO_CRCPOLY, 0x0000_065B);
        write_volatile(RADIO_CRCINIT, 0x555555);
        write_volatile(RADIO_DATAWHITEIV, 42);
        write_volatile(RADIO_PACKETPTR, PACKET_RAM_ADDR);

        loop {
            // 1) >=10 us trigger pulse.
            wr(GPIO0, OUTSET, TRIG);
            for _ in 0..64 {
                let _ = rd(GPIO0, OUT);
            }
            wr(GPIO0, OUTCLR, TRIG);

            // 2) Wait for the ECHO rising edge (bounded).
            let mut guard: u32 = 0;
            while rd(GPIO0, IN) & ECHO == 0 {
                guard += 1;
                if guard >= MAX_TICKS {
                    break;
                }
            }

            // 3) Time how long ECHO stays high — proportional to distance.
            let mut ticks: u32 = 0;
            while rd(GPIO0, IN) & ECHO != 0 {
                ticks += 1;
                if ticks >= MAX_TICKS {
                    break;
                }
            }

            // 4) Convert to mm and threshold -> ALARM.
            let distance_mm = if THRESHOLD_TICKS > 0 {
                ticks.saturating_mul(THRESHOLD_MM) / THRESHOLD_TICKS
            } else {
                0
            };
            let in_range = ticks > 0 && ticks <= THRESHOLD_TICKS;

            if in_range {
                wr(GPIO0, OUTSET, ALARM);
            } else {
                wr(GPIO0, OUTCLR, ALARM);
            }

            LAST_TICKS = ticks;
            DISTANCE_MM = distance_mm;
            IN_RANGE = in_range as u32;
            SAMPLE_COUNT = SAMPLE_COUNT.wrapping_add(1);

            // 5) Broadcast {distance_mm LE, in_range, counter} over BLE.
            let p = (PACKET_RAM_ADDR as *mut u8).add(PAYLOAD_OFF);
            p.write_volatile((distance_mm & 0xFF) as u8);
            p.add(1).write_volatile(((distance_mm >> 8) & 0xFF) as u8);
            p.add(2).write_volatile(in_range as u8);
            p.add(3).write_volatile(SAMPLE_COUNT as u8);

            write_volatile(RADIO_TASKS_TXEN, 1);
            while read_volatile(RADIO_EVENTS_READY) == 0 {}
            write_volatile(RADIO_EVENTS_READY, 0);
            write_volatile(RADIO_TASKS_START, 1);
            while read_volatile(RADIO_EVENTS_END) == 0 {}
            write_volatile(RADIO_EVENTS_END, 0);
            write_volatile(RADIO_TASKS_DISABLE, 1);
            while read_volatile(RADIO_EVENTS_DISABLED) == 0 {}
            write_volatile(RADIO_EVENTS_DISABLED, 0);
            TX_DONE_COUNT = TX_DONE_COUNT.wrapping_add(1);

            // Settle before the next ranging.
            for _ in 0..256 {
                let _ = rd(GPIO0, OUT);
            }
        }
    }
}
