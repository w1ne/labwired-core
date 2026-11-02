# NuttX for the Curiosity PIC32MZ EF 2.0

`board/` is an out-of-tree Apache NuttX board for the Microchip Curiosity
PIC32MZ EF 2.0 (DM320209, PIC32MZ2048EFM144), derived from NuttX's
`flipnclick-pic32mz` board (Apache-2.0, headers kept): the 8 MHz FRC-fed PLL
at 200 MHz, UART2 on the PKoB4 virtual COM port (U2TX RG9, U2RX RE9), the
three user LEDs, MIPS32 boot ISA. NuttX and nuttx-apps themselves are
unmodified upstream commits (see `build.sh`).

```sh
./build.sh nsh      # tests/fixtures/curiosity-pic32mz-ef-nuttx-nsh.elf
./build.sh ostest   # tests/fixtures/curiosity-pic32mz-ef-nuttx-ostest.elf
```

Needs a bare-metal little-endian MIPS32r2 GCC (the Zephyr SDK 0.17.0
`mips-zephyr-elf` toolchain is what the fixtures were built with) and
`kconfiglib`. `kconfig-tweak` is shimmed by the script beside this file when
kconfig-frontends is not installed.
