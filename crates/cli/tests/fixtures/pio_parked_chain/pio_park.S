@ RP2040 PIO0 parked-chain fixture (crates/cli/tests/pio_parked_chain.rs).
@ SM0 parks on `jmp 0`, sees a TX FIFO write while parked, runs a two-word
@ ping-pong (`jmp 1` / `jmp 0`) at divider 1 and 2, and parks again. Each
@ report sends FSTAT[31:24] (TXEMPTY) and SM0_ADDR to UART0 at fixed cycle
@ offsets, so a step chain that stops or restarts on the wrong cycle changes
@ the UART bytes.
    .syntax unified
    .cpu cortex-m0plus
    .thumb

    .section .vectors, "a"
    .word 0x20000400
    .word reset

    .text
    .thumb_func
    .global reset
reset:
    ldr r7, =0x50200000         @ PIO0
    ldr r6, =0x40034000         @ UART0 DR
    ldr r5, =0x502000c8         @ PIO0 SM0_CLKDIV (SM0_ADDR at +12)
    movs r0, #1
    str r0, [r7, #0x00]         @ CTRL: enable SM0 at `jmp 0` (parked)
    bl wait
    bl report
    movs r0, #0x55
    str r0, [r7, #0x10]         @ TXF0 while parked
    ldr r0, [r7, #0x04]         @ FSTAT right after the write
    lsrs r0, r0, #24
    str r0, [r6]
    ldr r0, [r7, #0x04]         @ and one access later
    lsrs r0, r0, #24
    str r0, [r6]
    bl wait
    bl report
    movs r0, #0
    str r0, [r7, #0x4c]         @ INSTR_MEM1 = jmp 0
    movs r0, #1
    str r0, [r7, #0x48]         @ INSTR_MEM0 = jmp 1: ping-pong, not parked
    bl sample
    movs r0, #0
    str r0, [r7, #0x48]         @ INSTR_MEM0 = jmp 0: parks again
    bl wait
    bl report
    ldr r0, =0x00020000
    str r0, [r5, #0x00]         @ SM0_CLKDIV = 2.0: not parked
    movs r0, #1
    str r0, [r7, #0x48]         @ ping-pong at half rate
    bl sample
    ldr r0, =0x00010000
    str r0, [r5, #0x00]         @ SM0_CLKDIV = 1.0
    movs r0, #0
    str r0, [r7, #0x48]         @ park for good
    bl wait
    bl report
park:
    b park

    .thumb_func
wait:                           @ ~3000-cycle register countdown
    ldr r1, =1000
1:  subs r1, r1, #1
    bne 1b
    bx lr

    .thumb_func
report:                         @ FSTAT[31:24], SM0_ADDR
    ldr r0, [r7, #0x04]
    lsrs r0, r0, #24
    str r0, [r6]
    ldr r0, [r5, #0x0c]
    str r0, [r6]
    bx lr

    .thumb_func
sample:                         @ SM0_ADDR at 1..8 extra cycles apart
    movs r2, #8
1:  ldr r0, [r5, #0x0c]
    str r0, [r6]
    mov r1, r2
2:  subs r1, r1, #1
    bne 2b
    subs r2, r2, #1
    bne 1b
    bx lr
