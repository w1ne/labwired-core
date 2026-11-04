    .section .text.start, "ax"
    .global _start
    .type _start, @function
_start:
    li sp, 0x40880000
    call main
1:
    j 1b
