.syntax unified
.cpu arm7tdmi
.arm

.section .text.start, "ax", %progbits
.global _start
.type _start, %function
_start:
    mov r0, #0
1:
    add r0, r0, #1
    b 1b
.size _start, . - _start
