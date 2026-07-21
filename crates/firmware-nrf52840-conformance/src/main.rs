// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT
//
//! nRF52840 conformance firmware.
//!
//! Drives each peripheral through a realistic register sequence and writes an
//! observable-state **digest** to a fixed RAM block. The same ELF runs on the
//! simulator (full-chip `Machine`) and on real silicon; the harness
//! (`crates/hw-oracle/tests/nrf52840_conformance.rs`) diffs the two digests.
//!
//! Layout: `VERDICT[0]` = DONE sentinel (written last); the harness polls it.
//! `VERDICT[1..]` are per-test digest words — see `IDX_*`.
#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// ── Digest block ─────────────────────────────────────────────────────────────

/// Fixed RAM block for the observable-state digest (well below the stack).
const VERDICT: u32 = 0x2000_3000;
/// Written to `VERDICT[0]` last, after every test completes.
const DONE_MAGIC: u32 = 0x5284_0D0E;

const IDX_DONE: usize = 0;
const IDX_GPIO_OUT: usize = 1; // GPIO0 OUTSET/OUTCLR → OUT
const IDX_TIMER_COUNT: usize = 2; // TIMER0 counter-mode capture (expect 7)
const IDX_ECB_CT0: usize = 3; // ECB AES-128 ciphertext word 0 (LE u32)
const IDX_GPIOTE_OUT: usize = 4; // GPIOTE task drives GPIO0 OUT
const IDX_TEMP_INRANGE: usize = 5; // TEMP plausibility flag (1/0)
const IDX_RNG_LIVE: usize = 6; // RNG VALRDY fired flag (1/0)

const VERDICT_WORDS: usize = 16;

// ── nRF52840 peripheral base addresses (PS v1.7) ─────────────────────────────

const GPIO0_BASE: u32 = 0x5000_0000;
const TIMER0_BASE: u32 = 0x4000_8000;
const ECB_BASE: u32 = 0x4000_e000;
const GPIOTE_BASE: u32 = 0x4000_6000;
const TEMP_BASE: u32 = 0x4000_c000;
const RNG_BASE: u32 = 0x4000_d000;

// ── MMIO helpers ─────────────────────────────────────────────────────────────

#[inline(always)]
unsafe fn wr(addr: u32, val: u32) {
    write_volatile(addr as *mut u32, val);
}

#[inline(always)]
unsafe fn rd(addr: u32) -> u32 {
    read_volatile(addr as *const u32)
}

#[inline(always)]
unsafe fn digest(idx: usize, val: u32) {
    wr(VERDICT + (idx as u32) * 4, val);
}

/// Short busy-wait so task→event has a few cycles to propagate on silicon.
#[inline(never)]
fn settle() {
    for _ in 0..8u32 {
        unsafe { core::arch::asm!("nop") };
    }
}

// ── ECB data struct (static mut so linker gives it a real RAM address) ────────

#[repr(C)]
struct EcbData {
    key: [u8; 16],
    cleartext: [u8; 16],
    ciphertext: [u8; 16],
}

static mut ECB_BUF: EcbData = EcbData {
    // NIST FIPS-197 Appendix B key: 00 01 02 ... 0f
    key: [
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f,
    ],
    // Plaintext: 00 11 22 33 44 55 66 77 88 99 aa bb cc dd ee ff
    cleartext: [
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
        0xff,
    ],
    // Expected ciphertext (big-endian bytes): 69 c4 e0 d8  6a 7b 04 30  d8 cd b7 80  70 b4 c5 5a
    // Pre-fill so the struct has concrete size; overwritten by ECB hw.
    ciphertext: [0u8; 16],
};

// ── Entry point ──────────────────────────────────────────────────────────────

#[entry]
fn main() -> ! {
    unsafe {
        // Zero the digest block first.
        for i in 0..VERDICT_WORDS {
            digest(i, 0);
        }

        test_gpio();
        test_timer0();
        test_ecb();
        test_gpiote();
        test_temp();
        test_rng();

        // Sentinel written LAST so the harness knows all tests ran.
        digest(IDX_DONE, DONE_MAGIC);
    }
    halt_forever()
}

#[inline(never)]
fn halt_forever() -> ! {
    loop {
        core::hint::spin_loop();
    }
}

// ── Test: GPIO0 ───────────────────────────────────────────────────────────────
//
// nRF52840 PS §6.8 GPIO register offsets:
//   OUT     0x504   read current output
//   OUTSET  0x508   write 1 → set pin
//   OUTCLR  0x50C   write 1 → clear pin
//   DIRSET  0x518   write 1 → configure as output
//
// Sequence: configure pins 0..7 as outputs, set all 8, then clear pins 4..7.
// Expected OUT = 0x0000_000F (pins 0..3 remain set).
unsafe fn test_gpio() {
    wr(GPIO0_BASE + 0x518, 0x0000_00FF); // DIRSET: pins 0..7 = output
    wr(GPIO0_BASE + 0x50C, 0x0000_00FF); // OUTCLR: clear all 8 first
    settle();
    wr(GPIO0_BASE + 0x508, 0x0000_00FF); // OUTSET: set pins 0..7
    wr(GPIO0_BASE + 0x50C, 0x0000_00F0); // OUTCLR: clear pins 4..7
    settle();
    digest(IDX_GPIO_OUT, rd(GPIO0_BASE + 0x504));
}

// ── Test: TIMER0 (counter mode) ───────────────────────────────────────────────
//
// nRF52840 PS §6.28 TIMER register offsets:
//   TASKS_START    0x000   start timer
//   TASKS_STOP     0x004   stop timer
//   TASKS_COUNT    0x008   external count (counter mode)
//   TASKS_CLEAR    0x00C   clear counter
//   TASKS_CAPTURE[0] 0x040 capture CC[0]
//   MODE           0x504   0=Timer, 2=Counter
//   BITMODE        0x508   0=16-bit, 3=32-bit
//   CC[0]          0x540   capture/compare register 0
//
// Counter mode is fully deterministic: no real-time dependency.
// We pulse TASKS_COUNT exactly 7 times, then capture. CC[0] should equal 7.
unsafe fn test_timer0() {
    wr(TIMER0_BASE + 0x504, 2); // MODE = Counter
    wr(TIMER0_BASE + 0x508, 3); // BITMODE = 32-bit
    wr(TIMER0_BASE + 0x00C, 1); // TASKS_CLEAR
    settle();
    wr(TIMER0_BASE, 1); // TASKS_START (required even in counter mode)
    settle();
    // Pulse TASKS_COUNT 7 times.
    for _ in 0..7u32 {
        wr(TIMER0_BASE + 0x008, 1);
        settle();
    }
    wr(TIMER0_BASE + 0x040, 1); // TASKS_CAPTURE[0]
    settle();
    digest(IDX_TIMER_COUNT, rd(TIMER0_BASE + 0x540));
}

// ── Test: ECB AES-128 ─────────────────────────────────────────────────────────
//
// nRF52840 PS §6.18 ECB register offsets:
//   TASKS_STARTECB  0x000   start ECB encrypt
//   EVENTS_ENDECB   0x100   set when done
//   ECBDATAPTR      0x504   pointer to { key[16], cleartext[16], ciphertext[16] }
//
// NIST FIPS-197 Appendix B test vector:
//   key       = 000102030405060708090a0b0c0d0e0f
//   plaintext = 00112233445566778899aabbccddeeff
//   expected  = 69c4e0d86a7b0430d8cdb78070b4c55a
//
// IDX_ECB_CT0 = first 4 bytes of ciphertext as LE u32:
//   bytes [0x69, 0xc4, 0xe0, 0xd8] → u32::from_le_bytes = 0xd8e0c469
unsafe fn test_ecb() {
    let ptr = &raw const ECB_BUF as u32;
    wr(ECB_BASE + 0x504, ptr); // ECBDATAPTR
    wr(ECB_BASE + 0x100, 0); // clear EVENTS_ENDECB
    wr(ECB_BASE, 1); // TASKS_STARTECB
                     // Bounded wait on EVENTS_ENDECB.
    let mut guard: u32 = 0;
    while rd(ECB_BASE + 0x100) == 0 && guard < 2_000_000 {
        guard += 1;
    }
    // Read first 4 bytes of ciphertext as a LE u32.
    let ct = ECB_BUF.ciphertext;
    let word0 = u32::from_le_bytes([ct[0], ct[1], ct[2], ct[3]]);
    digest(IDX_ECB_CT0, word0);
}

// ── Test: GPIOTE task → GPIO pin ──────────────────────────────────────────────
//
// nRF52840 PS §6.9 GPIOTE register offsets:
//   TASKS_OUT[0]    0x000   toggle (when MODE=Task, POLARITY=Toggle)
//   TASKS_SET[0]    0x030   set (when MODE=Task, POLARITY=HiToLo or LoToHi)
//   TASKS_CLR[0]    0x060   clear
//   CONFIG[0]       0x510   channel config
//
// CONFIG[0] bit layout (nRF52840 PS v1.7 §6.9.4.10):
//   [1:0]   MODE    0=Disabled, 1=Event, 3=Task
//   [12:8]  PSEL    pin number within port (0..31)
//   [13]    PORT    0=GPIO0, 1=GPIO1
//   [17:16] POLARITY  0=None, 1=LoToHi(SET task→HIGH), 2=HiToLo(CLR task→LOW), 3=Toggle
//   [20]    OUTINIT   initial output level when task mode enabled
//
// We use GPIO0 pin 8, Task mode, SET polarity (LoToHi → TASKS_SET[0] drives HIGH).
// OUTINIT=0 (start low). After TASKS_SET[0]=1 the pin should go high.
// Expected: GPIO0 OUT bit 8 = 1 → IDX_GPIOTE_OUT = 1.
unsafe fn test_gpiote() {
    // Configure GPIO0 pin 8 as output first.
    wr(GPIO0_BASE + 0x518, 1 << 8); // DIRSET pin 8
    wr(GPIO0_BASE + 0x50C, 1 << 8); // OUTCLR pin 8 (start low)
    settle();

    // GPIOTE CONFIG[0]: MODE=Task(3), PSEL=8, PORT=0(GPIO0), POLARITY=LoToHi(1), OUTINIT=0
    // bit pattern: bits[1:0]=3, bits[12:8]=8, bit[13]=0, bits[17:16]=1, bit[20]=0
    // = 0x0001_0803
    // MODE=Task, PSEL=8, PORT=GPIO0, POLARITY=LoToHi, OUTINIT=low.
    let config: u32 = 3 | (8 << 8) | (1 << 16);
    wr(GPIOTE_BASE + 0x510, config); // CONFIG[0]
    settle();

    wr(GPIOTE_BASE + 0x030, 1); // TASKS_SET[0]
    settle();

    // Read GPIO0 OUT and digest just the bit for pin 8.
    let out = rd(GPIO0_BASE + 0x504);
    digest(IDX_GPIOTE_OUT, (out >> 8) & 1);
}

// ── Test: TEMP ────────────────────────────────────────────────────────────────
//
// nRF52840 PS §6.29 TEMP register offsets:
//   TASKS_START     0x000   start measurement
//   EVENTS_DATARDY  0x100   set when result ready
//   TEMP            0x508   result (signed, 0.25°C units, i.e. 100°C = 400)
//
// We don't digest the raw value (it's analog/varies). We check plausibility:
// -50°C..100°C → raw [-200, 400]. Liveness + sanity only.
unsafe fn test_temp() {
    wr(TEMP_BASE + 0x100, 0); // clear EVENTS_DATARDY
    wr(TEMP_BASE, 1); // TASKS_START
    let mut guard: u32 = 0;
    while rd(TEMP_BASE + 0x100) == 0 && guard < 2_000_000 {
        guard += 1;
    }
    let in_range: u32 = if rd(TEMP_BASE + 0x100) != 0 {
        let raw = rd(TEMP_BASE + 0x508) as i32;
        if (-200..=400).contains(&raw) {
            1
        } else {
            0
        }
    } else {
        0 // timed out — don't flag as in-range
    };
    digest(IDX_TEMP_INRANGE, in_range);
}

// ── Test: RNG ─────────────────────────────────────────────────────────────────
//
// nRF52840 PS §6.19 RNG register offsets:
//   TASKS_START     0x000   start RNG
//   TASKS_STOP      0x004   stop RNG
//   EVENTS_VALRDY   0x100   set when VALUE is ready
//   VALUE           0x508   one random byte
//
// We only check that VALRDY fires (liveness). The VALUE itself is not digested
// (non-deterministic). IDX_RNG_LIVE = 1 if VALRDY fired, else 0.
unsafe fn test_rng() {
    wr(RNG_BASE + 0x100, 0); // clear EVENTS_VALRDY
    wr(RNG_BASE, 1); // TASKS_START
    let mut guard: u32 = 0;
    while rd(RNG_BASE + 0x100) == 0 && guard < 2_000_000 {
        guard += 1;
    }
    wr(RNG_BASE + 0x004, 1); // TASKS_STOP
    digest(IDX_RNG_LIVE, if rd(RNG_BASE + 0x100) != 0 { 1 } else { 0 });
}
