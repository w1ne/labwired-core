/* Integer-only direct-ELF smoke for the imported RZ/G2L Cortex-M33 profile. */
.syntax unified
.cpu cortex-m33
.thumb

.section .text.Reset_Handler, "ax", %progbits
.global Reset_Handler
.type Reset_Handler, %function
.thumb_func
Reset_Handler:
    ldr     r3, =__stack_top
    msr     msp, r3
    movs    r0, #0x23
    movs    r1, #0x32
1:
    push    {r0-r2}
    pop     {r3-r5}
    eors    r2, r0, r1
    adds    r0, r2, #1
    eors    r1, r2, r0
    b       1b

.size Reset_Handler, .-Reset_Handler
