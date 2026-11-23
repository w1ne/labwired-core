# CH32V003F4P6 stock noneos-sdk fixture

Built with the PlatformIO `ch32v` platform 1.1.0 (Community-PIO-CH32V),
`board = ch32v003f4p6_evt_r0`, `framework = noneos-sdk`: the WCH vendor
startup (`startup_ch32v00x.S`), `SystemInit`, `Delay_Ms` (SysTick polling)
and `printf` over USART1 (`USART_Printf_Init`). No source line is changed to
suit the simulator.

Rebuild:

    cd tests/fixtures/ch32v003f4p6-noneos-sdk
    pio run
    riscv-wch-elf-objcopy --strip-debug .pio/build/sim/firmware.elf \
        ../ch32v003f4p6-noneos-sdk.elf

The startup writes the QingKe INTSYSCR CSR (0x804) and the delay loop polls
STK SR.CNTIF at 0xE000F000. Test:
`crates/core/tests/vendor/ch32v003_compiled_firmware.rs`.
