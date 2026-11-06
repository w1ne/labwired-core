.syntax unified
.cpu arm7tdmi
.arm

.section .text.start, "ax", %progbits
.global _start
.type _start, %function
_start:
    ldr r4, =message
    ldr r6, =0xE000C000
1:
    ldrb r5, [r4], #1
    cmp r5, #0
    beq 2f
    str r5, [r6]
    b 1b
2:
    b 2b
.size _start, . - _start

.section .rodata, "a", %progbits
message:
    .asciz "LPC2294_UART0_TX_OK"
