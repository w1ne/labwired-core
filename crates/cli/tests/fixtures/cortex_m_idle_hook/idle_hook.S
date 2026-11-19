/* STM32L476, register level: SysTick (reload 1999, core clock) increments
 * `ticks` in RAM. main waits for `ticks` to change in an RTOS-style idle
 * loop that calls an idle hook on every iteration (the shape of mbed's
 * rtos_idle_loop -> default_idle_hook): the hook pushes its return address,
 * enters a critical section (PRIMASK set, a RAM nesting counter 0 -> 1),
 * leaves it (1 -> 0, PRIMASK clear) and returns. Every iteration stores,
 * but rewrites exactly the bytes memory already holds. After each change
 * main toggles PA5, 100 times; then it bumps an idle counter through the
 * same hook 2000 times (each iteration changes memory: not a fixed point),
 * toggles once more and parks in `b .`.
 *
 * Built by build.sh (clang + rust-lld), linked at flash 0x08000000.
 */
    .syntax unified
    .cpu cortex-m4
    .thumb

    .section .vectors, "a"
    .word 0x20010000            /* initial SP */
    .word reset               /* .thumb_func symbols carry the Thumb bit */
    .rept 13
    .word default_handler
    .endr
    .word systick_handler       /* exception 15 */

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
idle:
    bl   idle_hook              /* the fixed point: stores rewrite memory */
    ldr  r3, [r4]
    cmp  r3, r2
    beq  idle
    ldr  r1, [r6]
    eor  r1, r1, #(1 << 5)
    str  r1, [r6]
    subs r5, r5, #1
    bne  toggle
    ldr  r7, =2000              /* an idle counter: NOT a fixed point */
    ldr  r5, =idle_count
count:
    bl   idle_hook
    ldr  r1, [r5]
    adds r1, r1, #1
    str  r1, [r5]
    subs r7, r7, #1
    bne  count
    ldr  r1, [r6]
    eor  r1, r1, #(1 << 5)
    str  r1, [r6]
park:
    b    park

    .thumb_func
idle_hook:
    push {r4, lr}
    cpsid i
    ldr  r0, =nesting
    ldr  r1, [r0]
    adds r1, r1, #1
    str  r1, [r0]
    subs r1, r1, #1
    str  r1, [r0]
    cpsie i
    pop  {r4, pc}

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
nesting:
    .word 0
idle_count:
    .word 0
