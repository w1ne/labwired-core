.section .text._start,"ax"
.globl _start
_start:
    li sp, 0x20002000
    call main
1:
    j 1b
