/* Cortex-M4 UART0 + GPIO1 smoke for the bounded Vybrid / Colibri VF61 profile.
 *
 * Polled only: Vybrid routes peripheral interrupts to the M4 NVIC through the
 * MSCM IRSPRC router, which the profile does not model, so no IRQ is used.
 *
 * UART0 (0x40027000, VF6xx RM UART chapter: C2@0x03, S1@0x04, D@0x07):
 *   C2 = TE|RE read back, S1 idle = TDRE|TC, then a banner is transmitted
 *   byte by byte behind S1.TDRE.
 * GPIO1 (0x400FF040: PDOR@0x0, PSOR@0x4, PCOR@0x8, PTOR@0xC):
 *   PDOR=0, PSOR 0xF0 -> 0xF0, PCOR 0x30 -> 0xC0, PTOR 0x81 -> 0x41.
 * The pass bits are stored as 0x5F610000|bits at 0x3F07FFF0 (all five = 0x1F).
 */
.syntax unified
.cpu cortex-m4
.thumb

.equ UART0,      0x40027000
.equ UART_C2,    0x03
.equ UART_S1,    0x04
.equ UART_D,     0x07
.equ S1_TDRE,    0x80
.equ GPIO1,      0x400FF040
.equ GPIO_PDOR,  0x00
.equ GPIO_PSOR,  0x04
.equ GPIO_PCOR,  0x08
.equ GPIO_PTOR,  0x0C
.equ RESULT,     0x3F07FFF0
.equ STACK_TOP,  0x3F07FF00

.section .text.start, "ax", %progbits
.global _start
.type _start, %function
.thumb_func
_start:
    ldr     r0, =STACK_TOP
    mov     sp, r0
    movs    r7, #0                  /* pass bits */
    ldr     r4, =UART0
    ldr     r5, =GPIO1

    /* UART0: enable TX/RX, read C2 back. */
    movs    r0, #0x0C
    strb    r0, [r4, #UART_C2]
    ldrb    r0, [r4, #UART_C2]
    cmp     r0, #0x0C
    bne     1f
    orr     r7, r7, #0x01
1:  ldrb    r0, [r4, #UART_S1]
    and     r0, r0, #0xC0
    cmp     r0, #0xC0
    bne     1f
    orr     r7, r7, #0x02
1:
    /* GPIO1 data registers. */
    movs    r0, #0
    str     r0, [r5, #GPIO_PDOR]
    movs    r0, #0xF0
    str     r0, [r5, #GPIO_PSOR]
    ldr     r0, [r5, #GPIO_PDOR]
    cmp     r0, #0xF0
    bne     1f
    orr     r7, r7, #0x04
1:  movs    r0, #0x30
    str     r0, [r5, #GPIO_PCOR]
    ldr     r0, [r5, #GPIO_PDOR]
    cmp     r0, #0xC0
    bne     1f
    orr     r7, r7, #0x08
1:  movs    r0, #0x81
    str     r0, [r5, #GPIO_PTOR]
    ldr     r6, [r5, #GPIO_PDOR]
    cmp     r6, #0x41
    bne     1f
    orr     r7, r7, #0x10
1:
    ldr     r0, =banner
    bl      puts
    ldr     r0, =msg_c2
    bl      puts
    ldrb    r0, [r4, #UART_C2]
    bl      puthex2
    ldr     r0, =msg_pdor
    bl      puts
    mov     r0, r6
    bl      puthex8
    ldr     r0, =msg_pass
    bl      puts
    mov     r0, r7
    bl      puthex2
    movs    r0, #'\n'
    bl      putc

    ldr     r0, =0x5F610000
    orr     r0, r0, r7
    ldr     r1, =RESULT
    str     r0, [r1]
2:  b       2b

/* putc(r0): wait for S1.TDRE, write D. */
.thumb_func
putc:
1:  ldrb    r1, [r4, #UART_S1]
    tst     r1, #S1_TDRE
    beq     1b
    strb    r0, [r4, #UART_D]
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

/* puthexN(r0): print the low N nibbles of r0, most significant first. */
.thumb_func
puthex2:
    push    {r2, r3, lr}
    movs    r3, #2
    lsl     r2, r0, #24
    b       hexloop
.thumb_func
puthex8:
    push    {r2, r3, lr}
    movs    r3, #8
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
banner:   .asciz "VF61 M4 SMOKE\n"
msg_c2:   .asciz "uart0 c2="
msg_pdor: .asciz "\ngpio1 pdor="
msg_pass: .asciz "\npass="
