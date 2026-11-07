/*
 * Cortex-M33 SCIFA smoke for the bounded RZ/G2L profile.
 * Runs from the M33 code SRAM with the stack in data SRAM. Polled only: no
 * interrupt is enabled, and neither the A55 cluster nor the boot flow is used.
 *
 * SCIF2 (0x4004C000, the Zephyr rzg2l_smarc cm33 console) and SCIF0
 * (0x4004B800) are RZ/G SCIFA blocks with 16-bit registers: SMR@0x00,
 * BRR@0x02, SCR@0x04, FTDR@0x06, FSR@0x08, FCR@0x0C, FDR@0x0E, FTCR@0x16.
 *   bit0: FSR reset is TDFE|TEND (0x60)
 *   bit1: BRR=0x1A reads back while SCR.TE/RE are clear
 *   bit2: SCR=TE|RE (0x30) reads back
 *   bit3: BRR ignores a write (0x55) while TE|RE are set
 *   bit4: FCR RFRST|TFRST (write-1) read back 0, and FTCR resets to 0x1F1F
 *   bit5: SCIF0 holds its own BRR (0x07), separate from SCIF2
 * A '#' is written to FTDR before SCR.TE is set; the smoke's anchored regex
 * fails if that byte reaches the console. The results are printed on SCIF2
 * (each byte waits for FSR.TDFE), and 0x26200000|bits is stored at
 * 0x2001FFF0 (all six = 0x2620003F).
 */
.syntax unified
.cpu cortex-m33
.thumb

.equ SCIF0,     0x4004B800
.equ SCIF2,     0x4004C000
.equ S_SMR,     0x00
.equ S_BRR,     0x02
.equ S_SCR,     0x04
.equ S_FTDR,    0x06
.equ S_FSR,     0x08
.equ S_FCR,     0x0C
.equ S_FTCR,    0x16
.equ FSR_TDFE,  0x20
.equ RESULT,    0x2001FFF0

.section .text.Reset_Handler, "ax", %progbits
.global Reset_Handler
.type Reset_Handler, %function
.thumb_func
Reset_Handler:
    ldr     r3, =__stack_top - 0x100
    msr     msp, r3
    movs    r7, #0                  /* pass bits */
    ldr     r4, =SCIF2
    ldr     r5, =SCIF0

    ldrh    r0, [r4, #S_FSR]
    cmp     r0, #0x60
    bne     1f
    orr     r7, r7, #0x01
1:  /* TE is clear: this byte must be dropped. */
    movs    r0, #'#'
    strb    r0, [r4, #S_FTDR]
    movs    r0, #0
    strh    r0, [r4, #S_SMR]
    movs    r0, #0x1A
    strb    r0, [r4, #S_BRR]
    movs    r0, #0x07
    strb    r0, [r5, #S_BRR]
    ldrb    r0, [r4, #S_BRR]
    cmp     r0, #0x1A
    bne     1f
    orr     r7, r7, #0x02
1:  movs    r0, #0x30
    strh    r0, [r4, #S_SCR]
    ldrh    r0, [r4, #S_SCR]
    cmp     r0, #0x30
    bne     1f
    orr     r7, r7, #0x04
1:  movs    r0, #0x55
    strb    r0, [r4, #S_BRR]
    ldrb    r6, [r4, #S_BRR]
    cmp     r6, #0x1A
    bne     1f
    orr     r7, r7, #0x08
1:  movs    r0, #0x06
    strh    r0, [r4, #S_FCR]
    ldrh    r0, [r4, #S_FCR]
    ldrh    r1, [r4, #S_FTCR]
    cbnz    r0, 1f
    movw    r2, #0x1F1F
    cmp     r1, r2
    bne     1f
    orr     r7, r7, #0x10
1:  ldrb    r0, [r5, #S_BRR]
    cmp     r0, #0x07
    bne     1f
    orr     r7, r7, #0x20
1:
    ldr     r0, =banner
    bl      puts
    ldr     r0, =msg_brr
    bl      puts
    mov     r0, r6
    lsl     r0, r0, #24
    bl      puthex2
    ldr     r0, =msg_scr
    bl      puts
    ldrh    r0, [r4, #S_SCR]
    lsl     r0, r0, #16
    bl      puthex4
    ldr     r0, =msg_pass
    bl      puts
    mov     r0, r7
    lsl     r0, r0, #24
    bl      puthex2
    movs    r0, #'\n'
    bl      putc

    ldr     r0, =0x26200000
    orr     r0, r0, r7
    ldr     r1, =RESULT
    str     r0, [r1]
2:  b       2b

/* putc(r0): wait for FSR.TDFE, write FTDR. */
.thumb_func
putc:
1:  ldrh    r1, [r4, #S_FSR]
    tst     r1, #FSR_TDFE
    beq     1b
    strb    r0, [r4, #S_FTDR]
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
banner:   .asciz "RZ/G2L M33 SCIFA SMOKE\n"
msg_brr:  .asciz "scif2 brr="
msg_scr:  .asciz "\nscif2 scr="
msg_pass: .asciz "\npass="
