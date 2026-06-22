/*
 * Copyright (c) 2026 Andrii Shylenko
 * SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
 */

/*
 * Silicon-vs-emulator validation for the STM32 (Cortex-M4/M33) wide-instruction
 * decoder fixes: LDRB.W/LDRH.W signedness, the wide register-extends
 * {S,U}XT{B,H}.W, and the extend-and-add {S,U}XTA{B,H}.W.
 *
 * Runs the exact instructions (with the exact operands used by the labwired
 * cortex_m unit tests) on bare metal, writing each result to a fixed RAM array
 * at 0x20000000. The same ELF runs on the labwired stm32l476 model AND on a
 * real NUCLEO-L476RG over SWD; the results array must be byte-identical and must
 * equal EXPECTED below. Any divergence means the emulator does NOT match silicon.
 *
 * Cortex-M4 (L476) shares these Thumb-2 encodings with the Cortex-M33 (H563)
 * the UDS gate runs on, so this validates the same decoder logic.
 */

#include <stdint.h>

#define RAM_TOP 0x20018000u /* L476: 96 KB SRAM1 at 0x20000000 */

/* Results land at 0x20000000 (see minimal.ld). Index 15 is a done-sentinel. */
volatile uint32_t g_results[16] __attribute__((section(".results"), used));

/* Test inputs live in flash (.rodata) so no .data copy is needed at reset. */
static const uint8_t  g_b85   = 0x85u;
static const uint16_t g_h8042 = 0x8042u;

__attribute__((noreturn)) void reset_handler(void)
{
    uint32_t r;
    const uint8_t  *pb = &g_b85;
    const uint16_t *ph = &g_h8042;

    /* LDRB.W / LDRSB.W of a byte with bit7 set (the F-9 bug: LDRB.W was signed). */
    __asm volatile("ldrb.w  %0, [%1]" : "=r"(r) : "r"(pb)); g_results[0] = r; /* 0x00000085 */
    __asm volatile("ldrsb.w %0, [%1]" : "=r"(r) : "r"(pb)); g_results[1] = r; /* 0xFFFFFF85 */

    /* LDRH.W / LDRSH.W of a halfword with bit15 set. */
    __asm volatile("ldrh.w  %0, [%1]" : "=r"(r) : "r"(ph)); g_results[2] = r; /* 0x00008042 */
    __asm volatile("ldrsh.w %0, [%1]" : "=r"(r) : "r"(ph)); g_results[3] = r; /* 0xFFFF8042 */

    /* Wide register-extends (F-11: UXTH.W/UXTB.W/SXTB.W were not decoded). */
    { uint32_t v = 0x1234FF00u; __asm volatile("uxth.w %0, %1" : "=r"(r) : "r"(v)); }
    g_results[4] = r;                                                          /* 0x0000FF00 */
    { uint32_t v = 0x00000085u; __asm volatile("uxtb.w %0, %1" : "=r"(r) : "r"(v)); }
    g_results[5] = r;                                                          /* 0x00000085 */
    { uint32_t v = 0x00000085u; __asm volatile("sxtb.w %0, %1" : "=r"(r) : "r"(v)); }
    g_results[6] = r;                                                          /* 0xFFFFFF85 */
    { uint32_t v = 0x00850000u; __asm volatile("uxth.w %0, %1, ror #8" : "=r"(r) : "r"(v)); }
    g_results[7] = r;                                                          /* 0x00008500 */

    /* Extend-and-add (F-10: UXTAH was not decoded). r = 4 + uxth(0x12340002). */
    { uint32_t a = 4u, b = 0x12340002u;
      __asm volatile("uxtah %0, %1, %2" : "=r"(r) : "r"(a), "r"(b)); }
    g_results[8] = r;                                                          /* 0x00000006 */

    g_results[15] = 0xDEADBEEFu; /* done sentinel */
    for (;;) { __asm volatile("nop"); }
}

/* Minimal vector table: initial SP + reset (function-pointer typed so both are
 * address constants; the reset symbol already carries the Thumb bit). */
typedef void (*vector_t)(void);
__attribute__((section(".vectors"), used))
const vector_t g_vectors[2] = {
    (vector_t)RAM_TOP,
    reset_handler,
};
