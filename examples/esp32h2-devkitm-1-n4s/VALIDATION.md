# ESP32-H2-DevKitM-1-N4S validation

Run from the repository root.

1. Install the RISC-V target: `rustup target add riscv32imc-unknown-none-elf`
2. Run the UART0 and onboard RGB smoke: `examples/esp32h2-devkitm-1-n4s/ci/test.sh`

The smoke checks for `H2_N4S_RGB_OK` on UART0, requires at least 16 GPIO8
transitions, and decodes one red WS2812 pixel in the structured simulation
result. The committed logs and result are in `evidence/`. This bare-ELF
fast-boot test does not run the H2 mask ROM, Espressif bootloader,
physical USB-UART bridge, IO_MUX pad route, or RMT.

H2 UART0's register head and clock setup were compared against Espressif's
`esp32h2.svd` snapshot. The model limits the UART window before the H2-specific
`CLK_CONF`/`DATE` reset values and limits PCR to the shared UART0 prefix.
