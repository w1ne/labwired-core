# CH32V003F4P6 stock noneos-sdk USART1 RX interrupt fixture

Built with the PlatformIO `ch32v` platform 1.1.0 (Community-PIO-CH32V),
`board = ch32v003f4p6_evt_r0`, `framework = noneos-sdk`. The vendor startup
sets `mtvec = _start | 3` and INTSYSCR = 3 (HWSTKEN | INESTEN).
`USART1_IRQHandler` is declared `__attribute__((interrupt("WCH-Interrupt-fast")))`,
so the compiler saves no caller-saved registers: it calls
`USART_GetITStatus` and `USART_ReceiveData` and relies on the QingKe hardware
prologue/epilogue (HPE) to restore `ra`.

`main` configures USART1 (TX PD5, RX PD6, 115200 8N1, TX | RX), enables
`USART_IT_RXNE` and USART1_IRQn (32) in the PFIC, prints a boot line, then
spins until the handler has stored four received bytes and prints them. The
test queues the bytes before reset, so the interrupts land inside the SDK
`printf` of the boot line; the boot line stays intact only if HPE restores
the registers the handler clobbers.

Rebuild:

    cd tests/fixtures/ch32v003f4p6-noneos-sdk-usart-irq
    pio run
    riscv-wch-elf-objcopy --strip-debug .pio/build/sim/firmware.elf \
        ../ch32v003f4p6-noneos-sdk-usart-irq.elf

Test: `crates/core/tests/vendor/ch32v003_compiled_firmware.rs`.
