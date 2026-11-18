# NUCLEO-F767ZI stock STM32duino fixture

`../stm32f767-stock-arduino.elf` is built from `src/main.ino` with
PlatformIO Core 6.2.0, `ststm32@19.7.1`, `nucleo_f767zi`, Arduino framework
2.12.0 (`framework-arduinoststm32@4.21200.0`) and GCC ARM 12.3.1
(`toolchain-gccarmnoneeabi@1.120301.0`). SHA-256:
`9391182437769183ef251b2b8fe72f0e8429ae3f98e0f3dba8117bf02ef00fe5`.

Rebuild from this directory with `pio run -e sim`; copy `.pio/build/sim/firmware.elf`
to the sibling `stm32f767-stock-arduino.elf`. This is a simulation fixture,
not evidence of a physical-board run.

The linked ELF contains STM32duino core and ST HAL/CMSIS code. The exact
framework package's `License.md` is preserved as `STM32DUINO-LICENSES.md`
(SHA-256 `36d165776a356822f2613e5357b60bf0d195f2684baab663dfeb6baa45590e6a`).
Its sources are distributed as PlatformIO package
`framework-arduinoststm32@4.21200.0`, with upstream at
https://github.com/stm32duino/Arduino_Core_STM32. The toolchain's newlib 4.3.0
license collection is `NEWLIB-LICENSES.txt` (whitespace normalized;
original package file SHA-256 `422aa40293093fb54fc66e692a0d68fd0b24ed5602e5d1d33ad05ba3909057e9`,
normalized SHA-256 `b7d68270904c15d136d0c4e446b6739eae846785b2159b525cfea5a2c433d3cc`).
