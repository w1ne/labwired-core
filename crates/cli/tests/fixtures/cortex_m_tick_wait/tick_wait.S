/* STM32L476, register level: SysTick (reload 1999, core clock) increments
 * `ticks` in RAM; main waits for `ticks` to change with a pure loop that only
 * an interrupt can end (the shape of HAL_Delay / Arduino delay()), toggles
 * PA5, 100 times. Then a 20000-iteration register countdown (not a fixed
 * point: each iteration changes r7), one more toggle, and `b .`. The wait
 * loop and `b .` are what `labwired test` skips as fixed-point poll loops;
 * the countdown must run as it is.
 *
 * Built by build.sh (arm-none-eabi-as/ld), linked at flash 0x08000000.
 */
    .syntax unified
    .cpu cortex-m4
    .thumb

    .section .vectors, "a"
    .word 0x20010000            /* initial SP */
    .word reset + 1
    .rept 13
    .word default_handler + 1
    .endr
    .word systick_handler + 1   /* exception 15 */

    .text
    .global reset
    .thumb_func
reset:
    ldr  r0, =0x4002104C        /* RCC_AHB2ENR */
    ldr  r1, [r0]
    orr  r1, r1, #1             /* GPIOAEN */
    str  r1, [r0]
    ldr  r0, =0x48000000        /* GPIOA_MODER: PA5 output */
    ldr  r1, [r0]
    bic  r1, r1, #(3 << 10)
    orr  r1, r1, #(1 << 10)
    str  r1, [r0]
    ldr  r0, =0xE000E014        /* SYST_RVR */
    ldr  r1, =1999
    str  r1, [r0]
    ldr  r0, =0xE000E018        /* SYST_CVR */
    movs r1, #0
    str  r1, [r0]
    ldr  r0, =0xE000E010        /* SYST_CSR: enable, tickint, core clock */
    movs r1, #7
    str  r1, [r0]
    ldr  r4, =ticks
    ldr  r6, =0x48000014        /* GPIOA_ODR */
    movs r5, #100
toggle:
    ldr  r2, [r4]
wait:
    ldr  r3, [r4]               /* the fixed-point loop */
    cmp  r3, r2
    beq  wait
    ldr  r1, [r6]
    eor  r1, r1, #(1 << 5)
    str  r1, [r6]
    subs r5, r5, #1
    bne  toggle
    ldr  r7, =20000             /* a countdown: NOT a fixed point */
count:
    subs r7, r7, #1
    bne  count
    ldr  r1, [r6]
    eor  r1, r1, #(1 << 5)
    str  r1, [r6]
park:
    b    park                   /* a fixed point no interrupt ends */

    .thumb_func
systick_handler:
    ldr  r0, =ticks
    ldr  r1, [r0]
    adds r1, r1, #1
    str  r1, [r0]
    bx   lr

    .thumb_func
default_handler:
    b    default_handler

    .ltorg

    .bss
    .align 2
ticks:
    .word 0
