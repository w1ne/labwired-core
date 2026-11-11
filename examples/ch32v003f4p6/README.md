# CH32V003F4P6 GPIO blink and USART1 hello

Demand target: **Tier C WCH vendor bet** from the demand-ordered list; it has no
numeric percentage rank in the survey table.

This is a small bare-metal C example for the WCH CH32V003F4P6. It enables the
GPIOC clock, configures PC0 as push-pull output, then toggles PC0 through the
WCH set/reset registers. The build targets RV32E instructions, a subset of the
chip's RV32EC CPU; it does not depend on compressed instructions.

Build the ELF with a RISC-V GCC that supports `-march=rv32e -mabi=ilp32e`:

```sh
./examples/ch32v003f4p6/build.sh
```

The committed `tests/fixtures/ch32v003f4p6-blink.elf` was built with
`riscv64-unknown-elf-gcc` 13.2.0-11ubuntu1+12, as recorded in the ELF's
`.comment` section. Its sha256 is
`ffe97d1a4de4493b58bd959d5fb0e23f0dfb31cedd43e82644a0d0e76687bbc4`. A
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

## WCH CH32V003EVT board

The exact WCH evaluation board is modelled in
[`configs/systems/ch32v003evt.yaml`](../../configs/systems/ch32v003evt.yaml).
It maps the board's active-low LED1 header to PC0. The board leaves LED1
unconnected by default, so add a jumper from PC0 to LED1 on header P4 before
running this smoke on a physical board. The [WCH board reference](https://www.olimex.com/Products/RISC-V/WCH-CH32V003EVT/resources/CH32V00x-Evaluation-Board-Reference-EN.pdf)
documents P4; the [board wiring guide](https://ch405labs.net/ch32v003_intro/)
shows the LED cathode at LED1, so a low PC0 level lights it. The board's USB
connector supplies power only.

Run the board smoke against the committed firmware:

```sh
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v003f4p6/evt-smoke.yaml \
  --output-dir out/boards/ch32v003evt/led1 --no-uart-stdout
```

The smoke asserts PC0 edges through the board system's LED1 wiring. The model
does not claim the evaluation board's USB data path, PFIC interrupts, or
unverified peripherals. Current WCH and distributor availability notes are in
[`vendor-status.md`](vendor-status.md).

The separate USART1 smoke exercises the board's exposed PD5 transmit pin:

```sh
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v003f4p6/evt-uart-smoke.yaml \
  --output-dir out/boards/ch32v003evt/usart1 --no-uart-stdout
```

On the physical board, connect PD5 to the RX input of a 3.3 V USB-serial
adapter. The board has no onboard USB-to-UART bridge.

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
`build.sh` with the same `riscv64-unknown-elf-gcc` 13.2.0-11ubuntu1+12. Its
sha256 is
`f77a83f95286c1f08ffc4e545645bdba435d84e8543f3988679e2452fa4570fd`.

The board smoke receipts are summarized in
[`evidence/onboarding-scoreboard.md`](evidence/onboarding-scoreboard.md).

The committed fixtures exercise the local simulator's RV32E/RV32EC,
reset/flash loading, RCC, GPIO, USART1, and the polled I2C1 master path.
CH32V003 is not yet a hosted LabWired verifier target, so this is not a hosted
`model_verified` result or hardware confirmation.

## BME280 over hardware I2C1

The `bme280-smoke.yaml` system connects an Adafruit BME280 STEMMA QT breakout
(product 2652) to the CH32V003EVT's exposed default I2C1 pins, PC1/SDA and
PC2/SCL. It enables the legacy-layout I2C1 master, reads the BME280 chip ID
register (`0xD0`), and prints a pass marker only for the expected ID `0x60` on
USART1/PD5. The board configuration selects the CH32V003 pin map explicitly;
the STM32F103 PB6/PB7 mapping is not reused for this WCH part. The smoke also
asserts SDA and SCL transitions on PC1 and PC2, so a missing pad route fails.

On hardware, power the sensor at 3.3 V and connect common ground, PC1 to SDA,
and PC2 to SCL. The Adafruit STEMMA QT breakout supplies the I2C pull-ups; for
other breakouts, check whether pull-ups are fitted or add 4.7 kΩ pull-ups to
3.3 V. Connect PD5 to a 3.3 V USB-serial adapter RX at 115200 8N1 to observe
the result. The smoke covers a polled single-byte I2C register read; it does
not claim I2C interrupts, DMA, AFIO remapping, or other WCH peripherals.

The committed smoke evidence lives under
[`evidence/evt-r0-bme280`](evidence/evt-r0-bme280/). The hosted LabWired
catalog does not yet list CH32V003, so this receipt is for the local engine run,
not hosted `labwired_verify` model-verification status.
