    .section .init, "ax"
    .global _start
_start:
    li sp, 0x3FCDFFF0
    call main
1:  j 1b
