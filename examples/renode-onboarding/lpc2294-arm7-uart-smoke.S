@ LPC2294 (ARM7TDMI-S) onboarding smoke: the declared peripheral surface,
@ driven the way the RTEMS rtl22xx BSP drives it (bspstart.c PLL sequence,
@ console/uart.c 16550 init and LSR.THRE polling).
@ Register addresses: RTEMS bsps/arm/rtl22xx/include/lpc22xx.h.
@
@ Each check prints one letter on UART0, P (pass) or F (fail), after the
@ banner "LPC2294 checks ". Checks, in order:
@   1 SCB: PLLCFG/PLLCON/feed, PLLSTAT.PLOCK set, VPBDIV + MAMTIM read back
@   2 PINSEL0 read back (UART0 TXD0/RXD0 on P0.0/P0.1, UART1 on P0.8/P0.9)
@   3 UART0 LCR.DLAB: DLL/DLM read back 0x61/0x00, LCR reads 0x03 after
@   4 UART0 LSR reads THRE|TEMT (0x60) while idle
@   5 port1 IOSET/IOCLR/IODIR: P1.16-P1.23 output, latch and IOPIN agree
@   6 port0..port3 keep their latches apart (one bit per port)
@   7 IOCLR reads 0 and an input pin (IODIR 0) does not show the latch
@   8 UART1 DLAB divisor read back and LSR THRE
@   9 Timer0 MR0 and Timer1 MR1 interrupts reach ARM7 through VIC sources 4/5;
@     match flags, prescale, Timer0 reset-on-match and W1C clear are checked
@  10 PL190 software IRQ reaches the ARM7 IRQ vector and clears on EOI
@ The run then prints " DONE" on UART0, a line on UART1, and parks.
.syntax unified
.cpu arm7tdmi
.arm

.equ UART0,   0xE000C000
.equ UART1,   0xE0010000
.equ RBR,     0x00
.equ DLL,     0x00
.equ DLM,     0x04
.equ LCR,     0x0C
.equ LSR,     0x14
.equ PORT0,   0xE0028000
.equ IOPIN,   0x0
.equ IOSET,   0x4
.equ IODIR,   0x8
.equ IOCLR,   0xC
.equ PINSEL0, 0xE002C000
.equ SCB,     0xE01FC000
.equ MAMCR,   0x000
.equ MAMTIM,  0x004
.equ PLLCON,  0x080
.equ PLLCFG,  0x084
.equ PLLSTAT, 0x088
.equ PLLFEED, 0x08C
.equ VPBDIV,  0x100
.equ VIC,     0xFFFFF000
.equ VIC_IRQSTATUS, 0x000
.equ VIC_INTENABLE, 0x010
.equ VIC_INTENABLECLEAR, 0x014
.equ VIC_SOFTINT, 0x018
.equ VIC_SOFTINTCLEAR, 0x01C
.equ VIC_VECTADDR, 0x030
.equ VIC_VECTADDR0, 0x100
.equ VIC_VECTADDR1, 0x104
.equ VIC_VECTCNTL0, 0x200
.equ VIC_VECTCNTL1, 0x204
.equ IRQ_SOURCE, 5
.equ TIMER0,   0xE0004000
.equ TIMER1,   0xE0008000
.equ TIR,      0x00
.equ TTCR,     0x04
.equ TTC,      0x08
.equ TPR,      0x0C
.equ TPC,      0x10
.equ TMCR,     0x14
.equ TMR0,     0x18
.equ TMR1,     0x1C
.equ TIMER0_IRQ_SOURCE, 4
.equ TIMER1_IRQ_SOURCE, 5
.equ TIMER0_RESULT, 0x40003FF0
.equ TIMER1_RESULT, 0x40003FF4
.equ SOFTIRQ_RESULT, 0x40003FF8

@ Low vectors: the IRQ entry asks the VIC for its selected handler address.
.section .vectors, "ax", %progbits
    b _start
    b .
    b .
    b .
    b .
    b .
    b irq_vector
    b .

.section .text.start, "ax", %progbits
.global _start
.type _start, %function
_start:
    @ Give the IRQ mode its own stack before any peripheral interrupt is
    @ unmasked. The vector saves the shared registers here before dispatch.
    mrs r0, cpsr
    bic r1, r0, #0x1F
    orr r1, r1, #0x12             @ IRQ mode
    msr cpsr_c, r1
    ldr sp, =0x40003F00
    bic r1, r1, #0x1F
    orr r1, r1, #0x13             @ return to SVC mode
    msr cpsr_c, r1
    ldr sp, =0x40004000            @ top of the 16 KiB SRAM

    @ UART0 first, so every later check can report.
    ldr r0, =PINSEL0
    ldr r1, =0x00050005
    str r1, [r0]
    ldr r6, =UART0
    bl uart_init

    ldr r0, =banner
    bl puts0

    @ 1. SCB / PLL (bspstart.c: PLLCFG, feed, wait PLOCK, connect, feed).
    ldr r4, =SCB
    mov r1, #0x24                  @ MSEL 4 (x5), PSEL 1
    str r1, [r4, #PLLCFG]
    mov r1, #1
    str r1, [r4, #PLLCON]
    mov r1, #0xAA
    str r1, [r4, #PLLFEED]
    mov r1, #0x55
    str r1, [r4, #PLLFEED]
    ldr r2, =100000
1:  ldr r1, [r4, #PLLSTAT]
    tst r1, #0x400
    bne 2f
    subs r2, r2, #1
    bne 1b
2:  mov r0, #0
    cmp r2, #0
    movne r0, #1
    mov r1, #3
    str r1, [r4, #PLLCON]
    mov r1, #0xAA
    str r1, [r4, #PLLFEED]
    mov r1, #0x55
    str r1, [r4, #PLLFEED]
    mov r1, #1
    str r1, [r4, #VPBDIV]
    mov r1, #3
    str r1, [r4, #MAMTIM]
    mov r1, #2
    str r1, [r4, #MAMCR]
    ldr r1, [r4, #VPBDIV]
    cmp r1, #1
    movne r0, #0
    ldr r1, [r4, #MAMTIM]
    cmp r1, #3
    movne r0, #0
    ldr r1, [r4, #PLLCFG]
    cmp r1, #0x24
    movne r0, #0
    bl verdict

    @ 2. PINSEL0 read back.
    ldr r4, =PINSEL0
    ldr r1, [r4]
    ldr r2, =0x00050005
    cmp r1, r2
    moveq r0, #1
    movne r0, #0
    bl verdict

    @ 3. UART0 divisor latches behind LCR.DLAB.
    mov r0, #1
    mov r1, #0x83
    str r1, [r6, #LCR]
    ldr r1, [r6, #DLL]
    and r1, r1, #0xFF
    cmp r1, #0x61
    movne r0, #0
    ldr r1, [r6, #DLM]
    ands r1, r1, #0xFF
    movne r0, #0
    mov r1, #0x03
    str r1, [r6, #LCR]
    ldr r1, [r6, #LCR]
    and r1, r1, #0xFF
    cmp r1, #0x03
    movne r0, #0
    bl verdict

    @ 4. UART0 LSR idle: THRE (bit 5) and TEMT (bit 6).
    ldr r1, [r6, #LSR]
    and r1, r1, #0x60
    cmp r1, #0x60
    moveq r0, #1
    movne r0, #0
    bl verdict

    @ 5. port1: P1.16-P1.23 as outputs (the usual LED byte on LPC22xx boards).
    ldr r4, =PORT0 + 0x10
    mov r0, #1
    ldr r1, =0x00FF0000
    str r1, [r4, #IODIR]
    str r1, [r4, #IOCLR]
    ldr r1, =0x00A50000
    str r1, [r4, #IOSET]
    ldr r1, =0x00050000
    str r1, [r4, #IOCLR]
    ldr r2, =0x00A00000
    ldr r1, [r4, #IOSET]
    cmp r1, r2
    movne r0, #0
    ldr r1, [r4, #IOPIN]
    ldr r3, =0x00FF0000
    and r1, r1, r3
    cmp r1, r2
    movne r0, #0
    ldr r1, [r4, #IODIR]
    cmp r1, r3
    movne r0, #0
    bl verdict

    @ 6. port0..port3: bit n+4 set on port n, read every latch back.
    @ Port 0 keeps P0.0/P0.1 and P0.8/P0.9 for the UARTs; bit 4 is free.
    ldr r4, =PORT0
    mov r5, #0
3:  mov r1, #0x10
    mov r1, r1, lsl r5
    add r2, r4, r5, lsl #4
    str r1, [r2, #IOSET]
    add r5, r5, #1
    cmp r5, #4
    bne 3b
    mov r0, #1
    mov r5, #0
4:  add r2, r4, r5, lsl #4
    ldr r1, [r2, #IOSET]
    cmp r5, #1
    ldreq r3, =0x00A00020      @ port1 also keeps check 5's latch
    movne r3, #0x10
    movne r3, r3, lsl r5
    cmp r1, r3
    movne r0, #0
    add r5, r5, #1
    cmp r5, #4
    bne 4b
    bl verdict

    @ 7. IOCLR is write-only (reads 0); port2 bit 6 is an input (IODIR 0),
    @    so IOPIN does not show its latch.
    ldr r4, =PORT0 + 0x20
    mov r0, #1
    ldr r1, [r4, #IOCLR]
    cmp r1, #0
    movne r0, #0
    ldr r1, [r4, #IOPIN]
    tst r1, #0x40
    movne r0, #0
    ldr r1, [r4, #IOSET]
    tst r1, #0x40
    moveq r0, #0
    bl verdict

    @ 8. UART1: init, divisor read back, LSR idle, then a line on UART1.
    mov r7, r6
    ldr r6, =UART1
    bl uart_init
    mov r0, #1
    mov r1, #0x83
    str r1, [r6, #LCR]
    ldr r1, [r6, #DLL]
    and r1, r1, #0xFF
    cmp r1, #0x61
    movne r0, #0
    mov r1, #0x03
    str r1, [r6, #LCR]
    ldr r1, [r6, #LSR]
    and r1, r1, #0x60
    cmp r1, #0x60
    movne r0, #0
    mov r6, r7
    bl verdict

    @ 9. Timer0 MR0 and Timer1 MR1 drive the corresponding level inputs on
    @    the VIC. The ARM7 takes both IRQs, handlers check the timer/VIC state,
    @    clear the W1C flags, stop the timers, and write vectored EOI.
    ldr r4, =VIC
    ldr r0, =TIMER0_RESULT
    mov r1, #0
    str r1, [r0]
    ldr r0, =TIMER1_RESULT
    str r1, [r0]
    ldr r1, =timer0_irq_handler
    str r1, [r4, #VIC_VECTADDR0]
    mov r1, #(0x20 | TIMER0_IRQ_SOURCE)
    str r1, [r4, #VIC_VECTCNTL0]
    ldr r1, =timer1_irq_handler
    str r1, [r4, #VIC_VECTADDR1]
    mov r1, #(0x20 | TIMER1_IRQ_SOURCE)
    str r1, [r4, #VIC_VECTCNTL1]
    mov r1, #1
    mov r2, r1, lsl #TIMER0_IRQ_SOURCE
    mov r3, r1, lsl #TIMER1_IRQ_SOURCE
    orr r2, r2, r3
    str r2, [r4, #VIC_INTENABLE]

    ldr r4, =TIMER0
    mov r1, #2                    @ hold reset while configuring
    str r1, [r4, #TTCR]
    mov r1, #0
    str r1, [r4, #TPR]
    mov r1, #10
    str r1, [r4, #TMR0]
    mov r1, #7                    @ MR0 interrupt + reset + stop-on-match
    str r1, [r4, #TMCR]
    mov r1, #1
    str r1, [r4, #TTCR]

    ldr r4, =TIMER1
    mov r1, #2
    str r1, [r4, #TTCR]
    mov r1, #1                    @ divide PCLK by two
    str r1, [r4, #TPR]
    mov r1, #12
    str r1, [r4, #TMR1]
    mov r1, #40                   @ MR1 interrupt + stop-on-match
    str r1, [r4, #TMCR]
    mov r1, #1
    str r1, [r4, #TTCR]

    @ IRQs are masked at reset. Unmask after vectors and handlers are ready.
    mrs r2, cpsr
    bic r2, r2, #0x80
    msr cpsr_c, r2
    ldr r0, =TIMER0_RESULT
    ldr r1, =TIMER1_RESULT
    ldr r5, =20000
6:  ldr r2, [r0]
    ldr r3, [r1]
    cmp r2, #1
    cmpeq r3, #1
    beq 7f
    subs r5, r5, #1
    bne 6b
7:  mov r0, #1
    cmp r2, #1
    movne r0, #0
    cmp r3, #1
    movne r0, #0
    ldr r4, =TIMER0
    ldr r1, [r4, #TIR]
    cmp r1, #0
    movne r0, #0
    ldr r4, =TIMER1
    ldr r1, [r4, #TIR]
    cmp r1, #0
    movne r0, #0
    ldr r4, =VIC
    ldr r1, [r4, #VIC_IRQSTATUS]
    cmp r1, #0
    movne r0, #0
    bl verdict

    @ 10. PL190 software IRQ: reprogram slot 0 for source 5, enable it,
    @     clear the source in the exception handler and write vectored EOI.
    ldr r4, =VIC
    mov r1, #0
    mov r2, #1
    mov r2, r2, lsl #TIMER0_IRQ_SOURCE
    mov r3, #1
    mov r3, r3, lsl #TIMER1_IRQ_SOURCE
    orr r2, r2, r3
    str r2, [r4, #VIC_INTENABLECLEAR]
    str r1, [r4, #VIC_VECTCNTL1]
    ldr r0, =SOFTIRQ_RESULT
    mov r1, #0
    str r1, [r0]
    ldr r1, =irq_handler
    str r1, [r4, #VIC_VECTADDR0]
    mov r1, #(0x20 | IRQ_SOURCE)
    str r1, [r4, #VIC_VECTCNTL0]
    mov r1, #1
    mov r1, r1, lsl #IRQ_SOURCE
    str r1, [r4, #VIC_INTENABLE]
    str r1, [r4, #VIC_SOFTINT]
    ldr r0, =SOFTIRQ_RESULT
    ldr r1, [r0]
    cmp r1, #2
    moveq r0, #1
    movne r0, #0
    ldr r1, [r4, #VIC_IRQSTATUS]
    cmp r1, #0
    movne r0, #0
    bl verdict

    ldr r0, =done
    bl puts0
    @ The UART1 line goes out last: the capture merges both UARTs.
    ldr r6, =UART1
    ldr r0, =uart1_line
    bl puts0
5:  b 5b
.size _start, . - _start

@ 16550 init as console/uart.c: IER 0, DLAB, divisor 0x0061, 8N1.
uart_init:
    mov r1, #0
    str r1, [r6, #0x04]            @ IER
    mov r1, #0x83
    str r1, [r6, #LCR]
    mov r1, #0x61
    str r1, [r6, #DLL]
    mov r1, #0
    str r1, [r6, #DLM]
    mov r1, #0x03
    str r1, [r6, #LCR]
    mov r1, #0x07
    str r1, [r6, #0x08]            @ FCR: enable + reset FIFOs
    bx lr

@ Print the NUL-terminated string at r0 on the UART at r6, waiting for
@ LSR.THRE before every byte.
puts0:
    ldrb r1, [r0], #1
    cmp r1, #0
    bxeq lr
1:  ldr r2, [r6, #LSR]
    tst r2, #0x20
    beq 1b
    str r1, [r6, #RBR]
    b puts0

@ ARM7 IRQ vector: preserve the interrupted shared registers, then let the VIC
@ return the programmed handler address directly into PC.
irq_vector:
    stmfd sp!, {r0-r3, r12}
    ldr r0, =VIC + VIC_VECTADDR
    ldr pc, [r0]

irq_unexpected:
    b .

irq_handler:
    ldr r0, =VIC + VIC_SOFTINTCLEAR
    mov r1, #1
    mov r1, r1, lsl #IRQ_SOURCE
    str r1, [r0]
    ldr r0, =SOFTIRQ_RESULT
    mov r1, #2
    str r1, [r0]
    ldr r0, =VIC + VIC_VECTADDR
    mov r1, #0
    str r1, [r0]
    ldmfd sp!, {r0-r3, r12}
    subs pc, lr, #4

timer0_irq_handler:
    mov r2, #1
    ldr r0, =VIC + VIC_IRQSTATUS
    ldr r1, [r0]
    mov r3, #1
    mov r3, r3, lsl #TIMER0_IRQ_SOURCE
    tst r1, r3
    movne r2, #1
    moveq r2, #0
    ldr r0, =TIMER0
    ldr r1, [r0, #TIR]
    tst r1, #1
    moveq r2, #0
    ldr r1, [r0, #TTC]
    cmp r1, #0                    @ MR0 reset-on-match resets TC
    movne r2, #0
    mov r1, #1
    str r1, [r0, #TIR]            @ write-one-to-clear MR0
    mov r1, #0
    str r1, [r0, #TTCR]           @ stop before the next MR0 match
    ldr r0, =TIMER0_RESULT
    str r2, [r0]
    ldr r0, =VIC + VIC_VECTADDR
    mov r1, #0
    str r1, [r0]
    ldmfd sp!, {r0-r3, r12}
    subs pc, lr, #4

timer1_irq_handler:
    mov r2, #1
    ldr r0, =VIC + VIC_IRQSTATUS
    ldr r1, [r0]
    mov r3, #1
    mov r3, r3, lsl #TIMER1_IRQ_SOURCE
    tst r1, r3
    movne r2, #1
    moveq r2, #0
    ldr r0, =TIMER1
    ldr r1, [r0, #TIR]
    tst r1, #2
    moveq r2, #0
    ldr r1, [r0, #TTC]
    cmp r1, #12                   @ MR1 fires at this count
    movne r2, #0
    mov r1, #2
    str r1, [r0, #TIR]            @ write-one-to-clear MR1
    mov r1, #0
    str r1, [r0, #TTCR]
    ldr r0, =TIMER1_RESULT
    str r2, [r0]
    ldr r0, =VIC + VIC_VECTADDR
    mov r1, #0
    str r1, [r0]
    ldmfd sp!, {r0-r3, r12}
    subs pc, lr, #4

@ r0 = 1 → 'P', else 'F', on the UART at r6.
verdict:
    cmp r0, #1
    moveq r1, #'P'
    movne r1, #'F'
1:  ldr r2, [r6, #LSR]
    tst r2, #0x20
    beq 1b
    str r1, [r6, #RBR]
    bx lr

.section .rodata, "a", %progbits
banner:
    .asciz "LPC2294 checks "
done:
    .asciz " DONE\n"
uart1_line:
    .asciz "LPC2294_UART1_OK\n"
