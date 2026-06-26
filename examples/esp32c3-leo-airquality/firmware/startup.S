/* Minimal ESP32-C3 (RISC-V rv32imc) startup.
 *
 * The LabWired simulator loads this bare ELF and jumps to its entry point
 * (`_start`) — there is no boot ROM in the C3 sim path. We set up the global
 * and stack pointers, clear .bss, copy .data from flash (LMA) to RAM (VMA),
 * then call main(). On return we spin.
 */
    .section .init, "ax"
    .global _start
    .type _start, @function
_start:
    /* Set up the global pointer (linker-relaxation anchor). */
    .option push
    .option norelax
    la gp, __global_pointer$
    .option pop

    /* Stack pointer = top of RAM (set by the linker as _stack_top). */
    la sp, _stack_top

    /* Copy .data from flash (LMA) to RAM (VMA). */
    la a0, _sdata
    la a1, _edata
    la a2, _sidata
1:
    bgeu a0, a1, 2f
    lw   a3, 0(a2)
    sw   a3, 0(a0)
    addi a0, a0, 4
    addi a2, a2, 4
    j    1b
2:

    /* Zero .bss. */
    la a0, _sbss
    la a1, _ebss
3:
    bgeu a0, a1, 4f
    sw   zero, 0(a0)
    addi a0, a0, 4
    j    3b
4:

    /* main(). */
    call main

    /* If main returns, spin forever. */
5:
    j 5b
    .size _start, . - _start
