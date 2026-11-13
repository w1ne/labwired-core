/* STM32L476, register level: enable GPIOA, drive PA5 high once, then park in
 * `wfi; b` with no interrupt source enabled. Nothing can wake the core, so
 * `labwired test` must end at `max_steps` with idle fast-forward on, the same
 * way it does with it off (where every WFI cycle is a step).
 *
 * Built by build.sh (arm-none-eabi-as/ld), linked at flash 0x08000000.
 */
    .syntax unified
    .cpu cortex-m4
    .thumb

    .section .vectors, "a"
    .word 0x20010000            /* initial SP */
    .word reset + 1
    .rept 14
    .word default_handler + 1
    .endr

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
    ldr  r0, =0x48000018        /* GPIOA_BSRR: PA5 high */
    movs r1, #(1 << 5)
    str  r1, [r0]
sleep:
    wfi
    b    sleep

    .thumb_func
default_handler:
    b    default_handler
