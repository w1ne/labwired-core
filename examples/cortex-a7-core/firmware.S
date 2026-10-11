.syntax unified
.cpu cortex-a7
.arm
.global _start
.type _start, %function
_start:
    mov r0, #7
    add r1, r0, #5
    ldr r2, =0x20000000
    str r1, [r2]
1:  b 1b
.size _start, . - _start
