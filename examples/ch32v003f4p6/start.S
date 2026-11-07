.section .text._start,"ax"
.globl _start
_start:
    li sp, 0x20000800
    call main
1:
    j 1b
