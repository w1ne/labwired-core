# CH32V003F4P6 openwch Arduino `attachInterrupt` + `Serial.read()` fixture

Built exactly as `../ch32v003f4p6-arduino-openwch/` (PlatformIO `ch32v`
platform 1.1.0, `board = ch32v003f4p6_evt_r0`, `framework = arduino`,
`board_build.core = openwch`, `framework-arduino-openwch-ch32` at
`3820604346d0a9f0ee9a72cab8b7583e488e6674`). The sketch is not modified for
the simulator.

What the sketch exercises:

- `attachInterrupt(PD3, GPIO_Mode_IPU, onFall, EXTI_Mode_Interrupt,
  EXTI_Trigger_Falling)` (the openwch signature). The core enables the AFIO
  clock (`pinV32_DisconnectDebug`), writes `AFIO->EXTICR` through
  `GPIO_EXTILineConfig` (2 bits per line, GPIOD = 3), sets EXTI INTENR and
  FTENR for line 3 and enables `EXTI7_0_IRQn` (20) in the PFIC;
- the core's `EXTI7_0_IRQHandler` (`WCH-Interrupt-fast`) clears INTFR and
  calls the sketch callback, which counts falling edges;
- `Serial.read()`: the openwch `HardwareSerial` polls USART1 RXNE (its
  `available()` always returns -1).

The sketch waits up to 20 ms for three falling edges on PD3, prints
`falls=<n>`, reads one line from USART1 (up to 20 ms), prints `rx=<line>`,
then `CH32V003_EXTI_PASS` only when it saw 3 edges and the line `wch`.

Rebuild:

    cd tests/fixtures/ch32v003f4p6-arduino-openwch-exti
    pio run
    riscv-wch-elf-objcopy --strip-debug .pio/build/sim/firmware.elf \
        ../ch32v003f4p6-arduino-openwch-exti.elf

Test: `crates/core/tests/vendor/ch32v003_compiled_firmware.rs`
(`ch32v003_openwch_attach_interrupt_and_serial_read_run_unmodified` and the
negative control `ch32v003_attach_interrupt_misses_pd3_with_the_stm32f1_afio_mux`).
