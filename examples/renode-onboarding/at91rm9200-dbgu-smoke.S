/*
 * Bounded AT91RM9200 import smoke: ARMv4T A32 code (assembled for
 * -mcpu=arm920t, so no ARMv5 instruction is accepted) executes from the
 * internal-memory window at 0x00000000, resets and enables the Debug Unit
 * transmitter (DBGU_CR RSTTX then TXEN), polls DBGU_SR.TXRDY before each byte
 * and writes a banner to DBGU_THR, then spins. No AIC, system timer, PMC,
 * PIO, SDRAM or MMU is exercised.
 */
.syntax unified
.cpu arm920t
.arm

.equ DBGU_BASE, 0xFFFFF200
.equ DBGU_CR, 0x00
.equ DBGU_SR, 0x14
.equ DBGU_THR, 0x1C
.equ CR_RSTTX, 1 << 3
.equ CR_TXEN, 1 << 6
.equ SR_TXRDY, 1 << 1

.section .text.start, "ax", %progbits
.global _start
.type _start, %function
_start:
    ldr r4, =message
    ldr r6, =DBGU_BASE
    mov r7, #CR_RSTTX
    str r7, [r6, #DBGU_CR]
    mov r7, #CR_TXEN
    str r7, [r6, #DBGU_CR]
1:
    ldrb r5, [r4], #1
    cmp r5, #0
    beq 3f
2:
    ldr r7, [r6, #DBGU_SR]
    tst r7, #SR_TXRDY
    beq 2b
    str r5, [r6, #DBGU_THR]
    b 1b
3:
    b 3b
.size _start, . - _start

.section .rodata, "a", %progbits
message:
    .asciz "AT91RM9200 DBGU SMOKE\n"
