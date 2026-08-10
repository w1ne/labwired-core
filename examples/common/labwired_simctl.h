/* LabWired - Firmware Simulation Platform
 * Copyright (C) 2026 Andrii Shylenko
 *
 * This software is released under the MIT License.
 * See the LICENSE file in the project root for full license information.
 *
 * GENERATED FILE — DO NOT EDIT.
 * Regenerate with:  python3 tools/gen_simctl_header.py
 * Source of truth:  crates/core/src/peripherals/simctl.rs
 *
 * labwired_simctl.h — let firmware end its own simulation run with a verdict.
 *
 * Without this, a test asserts on what appeared on a serial line: it proves the
 * expected characters were printed, not that the firmware passed. Writing to
 * this device ends the run carrying a structured exit code the harness reads.
 *
 *   #define LABWIRED_SIM
 *   #include "labwired_simctl.h"
 *
 *   int main(void) {
 *       if (!self_test()) LABWIRED_FAIL();
 *       LABWIRED_PASS();
 *   }
 *
 * THE DEVICE DOES NOT EXIST ON SILICON. Its window is mapped outside the chip's
 * real peripheral space, so a write reaches nothing (and on many parts faults)
 * on a physical board. Every macro below compiles to nothing unless
 * LABWIRED_SIM is defined, so the same source builds for hardware untouched.
 *
 * Requires a board that declares the device — see
 * `configs/systems/pico-selftest.yaml`. Override LABWIRED_SIMCTL_BASE to match
 * the base address your board declares.
 */

#ifndef LABWIRED_SIMCTL_H
#define LABWIRED_SIMCTL_H

#include <stdint.h>

#ifdef LABWIRED_SIM

/* Base address of the simctl window. Must match the board manifest. */
#ifndef LABWIRED_SIMCTL_BASE
#define LABWIRED_SIMCTL_BASE 0x60000000u
#endif

/* Size of the register window, in bytes. */
#define LABWIRED_SIMCTL_WINDOW 0x20u

#define LABWIRED_SIMCTL_REG(off) \
    (*(volatile uint32_t *)((uintptr_t)LABWIRED_SIMCTL_BASE + (off)))

#define LABWIRED_SIMCTL_EXIT LABWIRED_SIMCTL_REG(0x00)
#define LABWIRED_SIMCTL_SCLK LABWIRED_SIMCTL_REG(0x08)
#define LABWIRED_SIMCTL_SOUT LABWIRED_SIMCTL_REG(0x10)
#define LABWIRED_SIMCTL_SERR LABWIRED_SIMCTL_REG(0x18)

/* End the run with `code`. 0 is a pass, by the same convention a process exit
 * status uses. Never returns. */
#define LABWIRED_EXIT(code)                            \
    do {                                               \
        LABWIRED_SIMCTL_EXIT = (uint32_t)(code);       \
        for (;;) { /* the run ends at the store */ }   \
    } while (0)

/* The two cases worth naming. */
#define LABWIRED_PASS() LABWIRED_EXIT(0)
#define LABWIRED_FAIL() LABWIRED_EXIT(1)

/* Assert from inside firmware: fail the run with `code` if `cond` is false. */
#define LABWIRED_ASSERT(cond, code)  \
    do {                             \
        if (!(cond)) {               \
            LABWIRED_EXIT(code);     \
        }                            \
    } while (0)

/* Host stdout / stderr, distinct from any UART the board has. */
#define LABWIRED_PUTC(c) (LABWIRED_SIMCTL_SOUT = (uint32_t)(uint8_t)(c))
#define LABWIRED_PUTC_ERR(c) (LABWIRED_SIMCTL_SERR = (uint32_t)(uint8_t)(c))

static inline void labwired_puts(const char *s)
{
    while (*s) {
        LABWIRED_PUTC(*s++);
    }
}

/* SCLK is a 64-bit register read as two 32-bit words. A read of the low word is
 * itself an MMIO access that advances simulated time, so the halves can
 * straddle a carry: read high, low, high again, and retry if it moved. */
static inline uint64_t labwired_simctl_read64(volatile uint32_t *lo)
{
    uint32_t hi, low;
    do {
        hi = lo[1];
        low = lo[0];
    } while (hi != lo[1]);
    return ((uint64_t)hi << 32) | low;
}

/* Simulated time, in CPU cycles.
 *
 * The 32-bit form wraps — at 125 MHz, every ~34 s of simulated time — so use it
 * only for short intervals and prefer the 64-bit form across a run. */
static inline uint32_t labwired_sim_cycles(void)
{
    return LABWIRED_SIMCTL_SCLK;
}

static inline uint64_t labwired_sim_cycles64(void)
{
    return labwired_simctl_read64(&LABWIRED_SIMCTL_SCLK);
}

#else /* !LABWIRED_SIM — building for real hardware. */

#define LABWIRED_EXIT(code) ((void)0)
#define LABWIRED_PASS() ((void)0)
#define LABWIRED_FAIL() ((void)0)
#define LABWIRED_ASSERT(cond, code) ((void)0)
#define LABWIRED_PUTC(c) ((void)0)
#define LABWIRED_PUTC_ERR(c) ((void)0)

static inline void labwired_puts(const char *s) { (void)s; }
static inline uint32_t labwired_sim_cycles(void) { return 0; }
static inline uint64_t labwired_sim_cycles64(void) { return 0; }

#endif /* LABWIRED_SIM */

#endif /* LABWIRED_SIMCTL_H */
