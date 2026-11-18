# NUCLEO-F767ZI stock STM32duino fixture

`../stm32f767-stock-arduino.elf` is built from `src/main.ino` with
PlatformIO Core 6.2.0, `ststm32@19.7.1`, `nucleo_f767zi`, Arduino framework
2.12.0 (`framework-arduinoststm32@4.21200.0`) and GCC ARM 12.3.1
(`toolchain-gccarmnoneeabi@1.120301.0`). SHA-256:
`9391182437769183ef251b2b8fe72f0e8429ae3f98e0f3dba8117bf02ef00fe5`.

Rebuild from this directory with `pio run -e sim`; copy `.pio/build/sim/firmware.elf`
to the sibling `stm32f767-stock-arduino.elf`. This is a simulation fixture,
not evidence of a physical-board run.
