/* Classic ESP32, register level: drive GPIO2 high and low 50 times on a
 * 10 000-cycle CCOUNT busy-wait (the shape of an Arduino sketch timing a
 * servo pulse with ESP.getCycleCount()), then sit in WAITI with nothing to
 * wake it. Both waits are what `labwired test` skips: the busy-wait is a
 * proven CCOUNT poll loop, the WAITI an idle core.
 *
 * Built by build.sh with the esp32 GNU toolchain (xtensa-esp32-elf-as/ld),
 * linked at IRAM 0x40080000 where the bare-ELF boot enters.
 */
    .text
    .global _start
    .align 4
_start:
    movi    a6, 0x3FF
    slli    a6, a6, 20
    movi    a7, 0x440
    slli    a7, a7, 8
    add     a6, a6, a7          /* a6 = GPIO base 0x3FF44000 */
    addmi   a9, a6, 0x500
    movi    a8, 256
    s32i    a8, a9, 0x38        /* GPIO_FUNC2_OUT_SEL_CFG = 256: plain GPIO */
    movi    a8, 4               /* GPIO2 */
    s32i    a8, a6, 0x24        /* GPIO_ENABLE_W1TS */
    addi    a10, a6, 8          /* GPIO_OUT_W1TS; ^4 gives GPIO_OUT_W1TC */
    movi    a11, 4
    movi    a4, 625
    slli    a4, a4, 4           /* period 10 000 cycles */
    movi    a5, 50
    rsr.ccount a2               /* t0 */
1:  rsr.ccount a3               /* busy-wait until ccount - t0 >= period */
    sub     a3, a3, a2
    bltu    a3, a4, 1b
    add     a2, a2, a4          /* t0 += period: no drift */
    s32i    a8, a10, 0          /* set or clear GPIO2 */
    xor     a10, a10, a11
    addi    a5, a5, -1
    bnez    a5, 1b
2:  waiti   0
    j       2b
