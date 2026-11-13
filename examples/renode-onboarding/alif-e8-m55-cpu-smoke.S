/*
 * Cortex-M55 UART smoke for the bounded Alif Ensemble E8 profile.
 * It runs from ITCM, uses the local DTCM stack, and avoids Helium, the other
 * CPU cores, and boot firmware. Polled only: no interrupt is enabled.
 *
 * UART0 (0x49018000) and UART7 (0x4901F000) are DW_apb_uart 16550s with a
 * 4-byte stride: THR/DLL@0x00, IER/DLM@0x04, LCR@0x0C, LSR@0x14.
 *   bit0: LCR=0x83 (DLAB|8N1) reads back
 *   bit1: DLL=0x1B / DLM=0x02 read back behind DLAB
 *   bit2: LCR=0x03 (DLAB clear) reads back
 *   bit3: LSR idle has THRE|TEMT (0x60)
 *   bit4: UART7 holds its own divisor (0x05), separate from UART0
 * The results are printed on UART0 (each byte waits for LSR.THRE), and
 * 0xA1E80000|bits is stored at 0x200FFFF0 (all five = 0xA1E8001F).
 */
.syntax unified
.cpu cortex-m33
.thumb

.equ UART0,     0x49018000
.equ UART7,     0x4901F000
.equ U_THR,     0x00
.equ U_DLL,     0x00
.equ U_DLM,     0x04
.equ U_LCR,     0x0C
.equ U_LSR,     0x14
.equ LSR_THRE,  0x20
.equ RESULT,    0x200FFFF0

.section .isr_vector, "a", %progbits
.word __stack_top - 0x100
.word Reset_Handler

.section .text.Reset_Handler, "ax", %progbits
.global Reset_Handler
.type Reset_Handler, %function
.thumb_func
Reset_Handler:
    movs    r7, #0                  /* pass bits */
    ldr     r4, =UART0
    ldr     r5, =UART7

    /* UART0: DLAB on, program and read back the divisor latches. */
    movs    r0, #0x83
    str     r0, [r4, #U_LCR]
    ldr     r0, [r4, #U_LCR]
    and     r0, r0, #0xFF
    cmp     r0, #0x83
    bne     1f
    orr     r7, r7, #0x01
1:  movs    r0, #0x1B
    str     r0, [r4, #U_DLL]
    movs    r0, #0x02
    str     r0, [r4, #U_DLM]
    /* UART7 gets a different divisor while UART0 still has DLAB set. */
    movs    r0, #0x83
    str     r0, [r5, #U_LCR]
    movs    r0, #0x05
    str     r0, [r5, #U_DLL]
    movs    r0, #0x00
    str     r0, [r5, #U_DLM]
    ldr     r0, [r4, #U_DLL]
    and     r0, r0, #0xFF
    ldr     r1, [r4, #U_DLM]
    and     r1, r1, #0xFF
    cmp     r0, #0x1B
    bne     1f
    cmp     r1, #0x02
    bne     1f
    orr     r7, r7, #0x02
1:  ldr     r0, [r5, #U_DLL]
    and     r6, r0, #0xFF
    cmp     r6, #0x05
    bne     1f
    orr     r7, r7, #0x10
1:  movs    r0, #0x03
    str     r0, [r5, #U_LCR]
    str     r0, [r4, #U_LCR]
    ldr     r0, [r4, #U_LCR]
    and     r0, r0, #0xFF
    cmp     r0, #0x03
    bne     1f
    orr     r7, r7, #0x04
1:  ldr     r0, [r4, #U_LSR]
    and     r0, r0, #0x60
    cmp     r0, #0x60
    bne     1f
    orr     r7, r7, #0x08
1:
    ldr     r0, =banner
    bl      puts
    ldr     r0, =msg_dl
    bl      puts
    movs    r0, #0x83
    str     r0, [r4, #U_LCR]
    ldr     r0, [r4, #U_DLM]
    and     r0, r0, #0xFF
    ldr     r1, [r4, #U_DLL]
    and     r1, r1, #0xFF
    orr     r0, r1, r0, lsl #8
    movs    r1, #0x03
    str     r1, [r4, #U_LCR]
    lsl     r0, r0, #16
    bl      puthex4
    ldr     r0, =msg_u7
    bl      puts
    mov     r0, r6
    lsl     r0, r0, #24
    bl      puthex2
    ldr     r0, =msg_pass
    bl      puts
    mov     r0, r7
    lsl     r0, r0, #24
    bl      puthex2
    movs    r0, #'\n'
    bl      putc

    ldr     r0, =0xA1E80000
    orr     r0, r0, r7
    ldr     r1, =RESULT
    str     r0, [r1]
2:  b       2b

/* putc(r0): wait for LSR.THRE, write THR. */
.thumb_func
putc:
1:  ldr     r1, [r4, #U_LSR]
    tst     r1, #LSR_THRE
    beq     1b
    str     r0, [r4, #U_THR]
    bx      lr

/* puts(r0 = NUL-terminated string). */
.thumb_func
puts:
    push    {r2, lr}
    mov     r2, r0
1:  ldrb    r0, [r2], #1
    cbz     r0, 2f
    bl      putc
    b       1b
2:  pop     {r2, pc}

/* puthexN(r0 = value left-aligned in bits 31..): print N nibbles. */
.thumb_func
puthex2:
    push    {r2, r3, lr}
    movs    r3, #2
    b       hexstart
.thumb_func
puthex4:
    push    {r2, r3, lr}
    movs    r3, #4
hexstart:
    mov     r2, r0
hexloop:
    lsr     r0, r2, #28
    cmp     r0, #10
    ite     lt
    addlt   r0, r0, #'0'
    addge   r0, r0, #('a' - 10)
    bl      putc
    lsl     r2, r2, #4
    subs    r3, r3, #1
    bne     hexloop
    pop     {r2, r3, pc}

.section .rodata
banner:   .asciz "ALIF E8 M55 SMOKE\n"
msg_dl:   .asciz "uart0 dl="
msg_u7:   .asciz "\nuart7 dll="
msg_pass: .asciz "\npass="
