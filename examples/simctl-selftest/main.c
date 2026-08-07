/* LabWired - Firmware Simulation Platform
 * Copyright (C) 2026 Andrii Shylenko
 *
 * This software is released under the MIT License.
 * See the LICENSE file in the project root for full license information.
 *
 * The `simctl` self-test: firmware that decides its own verdict.
 *
 * This is the dogfood for `examples/common/labwired_simctl.h`. It runs a couple
 * of checks and reports the result through the device rather than printing a
 * string for the harness to grep, so `simctl-selftest.yaml` asserts
 * `firmware_exit: 0` instead of `uart_contains: "PASS"`.
 *
 * Build (produces the checked-in fixture):
 *   ./examples/simctl-selftest/build.sh
 */

#define LABWIRED_SIM
#include "labwired_simctl.h"

/* Exit codes. Distinct per check, so a failing run names WHICH check failed —
 * the thing a grep for "FAIL" on a serial line cannot tell you. */
#define EXIT_OK              0
#define EXIT_ARITHMETIC      2
#define EXIT_MEMORY          3
#define EXIT_CLOCK_STOPPED   4

static volatile unsigned scratch[8];

static int arithmetic_works(void)
{
    unsigned acc = 0;
    for (unsigned i = 1; i <= 10; i++) {
        acc += i;
    }
    return acc == 55u;
}

static int memory_round_trips(void)
{
    for (unsigned i = 0; i < 8; i++) {
        scratch[i] = i * 0x11111111u;
    }
    for (unsigned i = 0; i < 8; i++) {
        if (scratch[i] != i * 0x11111111u) {
            return 0;
        }
    }
    return 1;
}

/* Simulated time must advance while the firmware runs. This is the check that
 * could not be written at all without the device. */
static int the_simulated_clock_advances(void)
{
    uint64_t before = labwired_sim_cycles64();
    for (volatile unsigned i = 0; i < 1000; i++) {
        /* burn cycles */
    }
    return labwired_sim_cycles64() > before;
}

int main(void)
{
    labwired_puts("simctl selftest\n");

    LABWIRED_ASSERT(arithmetic_works(), EXIT_ARITHMETIC);
    LABWIRED_ASSERT(memory_round_trips(), EXIT_MEMORY);
    LABWIRED_ASSERT(the_simulated_clock_advances(), EXIT_CLOCK_STOPPED);

    LABWIRED_EXIT(EXIT_OK);
}

/* ---- minimal Cortex-M startup: vector table + reset handler ---- */

extern unsigned _stack_top;

void Reset_Handler(void)
{
    main();
    for (;;) {
    }
}

__attribute__((section(".isr_vector"), used))
void (*const vector_table[2])(void) = {
    (void (*)(void)) & _stack_top,
    Reset_Handler,
};
