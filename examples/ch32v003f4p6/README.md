# CH32V003F4P6 GPIO blink and USART1 hello

This is a small bare-metal C example for the WCH CH32V003F4P6. It enables the
GPIOC clock, configures PC0 as push-pull output, then toggles PC0 through the
WCH set/reset registers. The build targets RV32E instructions, a subset of the
chip's RV32EC CPU; it does not depend on compressed instructions.

Build the ELF with a RISC-V GCC that supports `-march=rv32e -mabi=ilp32e`:

```sh
./examples/ch32v003f4p6/build.sh
```

The committed `tests/fixtures/ch32v003f4p6-blink.elf` was built with
`riscv32-esp-elf-gcc` 14.2.0 (crosstool-NG `esp-14.2.0_20260121`, as recorded
in the ELF's `.comment` section). Its sha256 is
`67196d739f228401711ced1702295e3129f6df7136f1c2244cc051e14dfba329`. A
different GCC may produce different bytes; the smoke checks behaviour, not the
hash.

Run the committed firmware against the local CH32V003 model:

```sh
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v003f4p6/io-smoke.yaml \
  --output-dir out/boards/ch32v003f4p6/io-smoke --no-uart-stdout
```

The smoke requires at least two PC0 output edges. The bare chip model has no
external LED attached; on a physical part, wire PC0 through a current-limiting
resistor to an LED and ground.

## USART1 hello

`uart_hello.c` is built for `-march=rv32ec`, the chip's full ISA, so it also
runs compressed instructions on the 16-register file. It follows WCH's own
sources:

- Clock: the RCC step of `SystemInit` (`RCC->CFGR0 &= 0xF8FF0000`) leaves
  SYSCLK on HSI with HPRE = DIV1, so HCLK is 24 MHz. That is WCH's
  `SetSysClockTo_24MHZ_HSI` minus its `FLASH->ACTLR` write. The flash
  controller is not modelled, and that write selects 0 wait states anyway.
- Pins: USART1's default mapping from WCH `Debug/debug.c`. TX is PD5, AF
  push-pull, 30 MHz (CFGLR nibble 0xB).
- Baud: `USART_Init`'s divider for 115200 at 24 MHz gives BRR = 0x00D0
  (115384 baud, +0.16%).

Run it:

```sh
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v003f4p6/uart-smoke.yaml \
  --output-dir out/boards/ch32v003f4p6/uart-smoke --no-uart-stdout
```

The smoke asserts `Hello from CH32V003` on the UART stream. TX is polled. The
PFIC is not modelled, so USART1 interrupts are not delivered. On a physical
part, PD5 goes to the RX pin of a 3.3 V USB-serial adapter at 115200 8N1.

The committed `tests/fixtures/ch32v003f4p6-uart-hello.elf` was built by
`build.sh` with the same `riscv32-esp-elf-gcc` 14.2.0. Its sha256 is
`b2cb6f34eb4c907054d4fc9bdfa1c13ec162579e7285343e182c628b0bea463a`.

These fixtures exercise the local simulator's RV32E/RV32EC, reset/flash
loading, RCC, GPIO and USART1 paths. CH32V003 is not yet a hosted LabWired
verifier target, so this is not a hosted `model_verified` result or hardware confirmation.
