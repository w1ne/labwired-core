// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT
//
// nRF52840 HC-SR04 ultrasonic proximity firmware.
//
// Wiring (matches the lab's system.yaml):
//   TRIG  = P0.04  (MCU output -> sensor)
//   ECHO  = P1.05  (sensor -> MCU input)
//   ALARM = P0.06  (MCU output -> buzzer/LED; the "in range" flag)
//
// Loop: pulse TRIG ~10 us, wait for the ECHO rising edge, time how long ECHO
// stays high, convert that to a distance, and raise ALARM when the target is
// within THRESHOLD_MM. ECHO-high time is proportional to distance
// (cm = echo_us / 58), so the measured tick count is proportional to distance;
// THRESHOLD_TICKS is that count at THRESHOLD_MM, calibrated against the
// simulator (deterministic) — see the crate README "Calibration".
//
// Pure nRF register access (no LabWired APIs): the same ELF runs in the
// LabWired simulator and flashes to real nRF52840 silicon.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// nRF52840 GPIO port base addresses (P1 is remapped to 0x50001000 in the
// LabWired memory map; on silicon it is 0x50000300 — firmware never hard-codes
// that, it only ever touches these two bases, which the linker/board map fix).
const GPIO0: usize = 0x5000_0000;
const GPIO1: usize = 0x5000_1000;

// nRF GPIO register offsets (Nordic nRF52840 PS v1.7 §6.10).
const OUT: usize = 0x504;
const OUTSET: usize = 0x508;
const OUTCLR: usize = 0x50C;
const IN: usize = 0x510;
const DIRSET: usize = 0x518;

const TRIG_BIT: u32 = 4; // P0.04
const ECHO_BIT: u32 = 5; // P1.05
const ALARM_BIT: u32 = 6; // P0.06

const TRIG: u32 = 1 << TRIG_BIT;
const ECHO: u32 = 1 << ECHO_BIT;
const ALARM: u32 = 1 << ALARM_BIT;

/// Target distance: raise ALARM when the measured distance is at or below this.
const THRESHOLD_MM: u32 = 150;

/// ECHO-high tick count corresponding to THRESHOLD_MM. Calibrated against the
/// simulator at distance_cm = 15.0 (= 150 mm) with cpu_hz = 64 MHz: the ECHO
/// pulse there is measured as 8950 of this loop's iterations. Deterministic for
/// a given HC-SR04 `cpu_hz` and this measurement loop. See README "Calibration".
const THRESHOLD_TICKS: u32 = 8950;

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

        loop {
            // 1) >=10 us trigger pulse. We drive OUT directly (OUTSET/OUTCLR
            //    also work) so the rising edge is unambiguous.
            wr(GPIO0, OUTSET, TRIG);
            for _ in 0..64 {
                let _ = rd(GPIO0, OUT);
            }
            wr(GPIO0, OUTCLR, TRIG);

            // 2) Wait for the ECHO rising edge (bounded).
            let mut guard: u32 = 0;
            while rd(GPIO1, IN) & ECHO == 0 {
                guard += 1;
                if guard >= MAX_TICKS {
                    break;
                }
            }

            // 3) Time how long ECHO stays high — proportional to distance.
            let mut ticks: u32 = 0;
            while rd(GPIO1, IN) & ECHO != 0 {
                ticks += 1;
                if ticks >= MAX_TICKS {
                    break;
                }
            }

            // 4) Convert to mm (self-consistent with the calibration point) and
            //    threshold -> ALARM.
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

            // Settle before the next ranging (datasheet: >=60 ms between cycles;
            // short here to keep the lab's cycle budget small).
            for _ in 0..256 {
                let _ = rd(GPIO0, OUT);
            }
        }
    }
}
