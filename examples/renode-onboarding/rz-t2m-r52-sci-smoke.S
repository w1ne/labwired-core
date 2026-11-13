/*
 * Bounded Renesas RZ/T2M import smoke: one Cortex-R52 PE enters at its
 * Hyp-mode reset state, executing A32 from System RAM at 0x10000000. It reads
 * MIDR and spins without output unless the part number is Cortex-R52
 * (0xD13), then sets CCR0.TE on SCI0 (0x80001000, the 32-bit SCI register
 * map Renode's Renesas_SCI models), polls CSR.TDRE before each byte and
 * writes it to TDR. Module-stop release, clock and pin setup are skipped:
 * the model does not gate SCI on MSTPCR or the clock tree. No GIC, generic
 * timer, TCM, xSPI flash, GPIO or other SCI channel is exercised.
 */
.syntax unified
.cpu cortex-r52
.arm

.equ SCI_TDR, 0x04
.equ SCI_CCR0, 0x08
.equ SCI_CSR, 0x48
.equ CCR0_TE, 1 << 4
.equ CSR_TDRE, 1 << 29

.section .text.start, "ax", %progbits
.global _start
.type _start, %function
_start:
    mrc p15, 0, r0, c0, c0, 0      @ MIDR
    ubfx r0, r0, #4, #12           @ PartNum
    movw r1, #0xd13
    cmp r0, r1
    bne 4f
    ldr r4, =message
    ldr r6, =0x80001000
    mov r7, #CCR0_TE
    str r7, [r6, #SCI_CCR0]
1:
    ldrb r5, [r4], #1
    cmp r5, #0
    beq 3f
2:
    ldr r7, [r6, #SCI_CSR]
    tst r7, #CSR_TDRE
    beq 2b
    str r5, [r6, #SCI_TDR]
    b 1b
3:
    b 3b
4:
    b 4b
.size _start, . - _start

.section .rodata, "a", %progbits
message:
    .asciz "RZ/T2M R52 SCI0 SMOKE\n"
