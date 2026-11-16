.section .text._start,"ax"
.globl _start
_start:
    li sp, 0x20005000
    call main
1:
    j 1b
