    .section .text.entry, "ax"
    .align 4
    .global _start
_start:
    /* Fibonacci(10) without ENTRY/RETW.
     *
     * ENTRY was removed from this fixture because the fixture is not called
     * from another windowed frame (there is no prior CALL4/8/12 to set
     * PS.CALLINC), so ENTRY would not rotate the register window.  The
     * computation is identical; we simply start executing directly.
     *
     * Result: a2 = fib(10) = 55.
     * Terminator: BREAK 1, 15 halts the simulator (BreakpointHit) and the
     * ESP32-S3 hardware debug unit.
     */
    movi    a2, 10              /* N = 10 (countdown) */
    movi    a3, 0               /* fib(n-2) = 0 */
    movi    a4, 1               /* fib(n-1) = 1 */
    beqz    a2, done
loop:
    add     a5, a3, a4          /* a5 = fib(n-2) + fib(n-1) */
    mov     a3, a4              /* a3 = old fib(n-1) */
    mov     a4, a5              /* a4 = new fib(n-1) */
    addi    a2, a2, -1          /* decrement counter */
    bnez    a2, loop
done:
    mov     a2, a3              /* result → a2 */
    break   1, 15               /* BREAK 1,15: halt sim / HW debug unit */
