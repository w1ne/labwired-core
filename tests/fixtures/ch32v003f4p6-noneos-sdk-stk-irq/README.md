# CH32V003F4P6 stock noneos-sdk SysTick interrupt fixture

Built with the PlatformIO `ch32v` platform 1.1.0 (Community-PIO-CH32V),
`board = ch32v003f4p6_evt_r0`, `framework = noneos-sdk`. The vendor startup
sets `mtvec = _start | 3` (vector table of absolute handler addresses) and
INTSYSCR = 3 (HWSTKEN | INESTEN). `SysTick_Handler` is declared
`__attribute__((interrupt("WCH-Interrupt-fast")))`, so the compiler emits no
register save: the handler clobbers `a5` and relies on the QingKe hardware
prologue/epilogue (HPE) to restore it.

`main` arms the STK for a 1 kHz match with STIE, enables SysTick_IRQn (12) in
the PFIC, waits for five ticks, then disables the line and checks that a
3 ms `Delay_Ms` adds no tick.

Rebuild:

    cd tests/fixtures/ch32v003f4p6-noneos-sdk-stk-irq
    pio run
    riscv-wch-elf-objcopy --strip-debug .pio/build/sim/firmware.elf \
        ../ch32v003f4p6-noneos-sdk-stk-irq.elf

Test: `crates/core/tests/vendor/ch32v003_compiled_firmware.rs`.
