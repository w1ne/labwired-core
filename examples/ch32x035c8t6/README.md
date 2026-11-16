# CH32X035C8T6 (LQFP48) GPIO blink and USART1 hello

The CH32X035C8T6 is a WCH QingKe V4C (RV32IMAC) microcontroller with USB FS
and USB PD: 48 MHz built-in HSI, 62 KB CodeFlash, 20 KB SRAM and 46 GPIOs in
LQFP48 (WCH CH32X035/X033 Datasheet V2.3). Its RCC, GPIO (pins 0..15) and
USART registers use the STM32F1 layout (CH32X035 Reference Manual V1.9), so
the descriptor runs the `stm32f1` RCC and GPIO profiles on the standard RV32
core.

## Copy-paste bare-metal C example

```c
#include <stdint.h>

/* Register addresses: WCH CH32X035 RM V1.9 (RCC Table 3-1, GPIO port B). */
#define RCC_APB2PCENR (*(volatile uint32_t *)0x40021018u)
#define GPIOB_CFGHR   (*(volatile uint32_t *)0x40010C04u)
#define GPIOB_BSHR    (*(volatile uint32_t *)0x40010C10u)
#define GPIOB_BCR     (*(volatile uint32_t *)0x40010C14u)

#define LED_BIT 12u

static void delay(void) {
    for (volatile uint32_t i = 0; i < 500; ++i) {
        __asm__ volatile("nop");
    }
}

void main(void) {
    RCC_APB2PCENR |= 1u << 3; /* enable GPIOB */
    /* PB12 is in CFGHR (pins 8..15): push-pull output (GPIO_Init nibble 0x1). */
    GPIOB_CFGHR = (GPIOB_CFGHR & ~(0xFu << ((LED_BIT - 8u) * 4u)))
                | (0x1u << ((LED_BIT - 8u) * 4u));

    for (;;) {
        GPIOB_BSHR = 1u << LED_BIT; /* set PB12 */
        delay();
        GPIOB_BCR = 1u << LED_BIT;  /* reset PB12 */
        delay();
    }
}
```

Build both fixtures with any RISC-V GCC that supports `-march=rv32imac`:

```sh
./examples/ch32x035c8t6/build.sh
```

The committed `tests/fixtures/ch32x035c8t6-blink.elf` and
`tests/fixtures/ch32x035c8t6-uart-hello.elf` were built with
`riscv32-esp-elf-gcc` 14.2.0 (crosstool-NG `esp-14.2.0_20260121`). Their
sha256 values are
`69f17dd80e6f73b3bb8eb7d4401b6ebf3ec940e67f31424aafe90f4cf4191802` and
`e9c3836cb936895f06e854990c37f3d1f42834329437f3f3e33d0b9c2884f5ca`. The
smokes check behaviour, not the hash.

Run the blink against the local model:

```sh
cargo run -q -p labwired-cli -- test \
  --script examples/ch32x035c8t6/io-smoke.yaml \
  --output-dir out/boards/ch32x035c8t6/io-smoke --no-uart-stdout
```

The smoke requires at least two PB12 output edges. On a physical part, wire
PB12 (pin 35) through a current-limiting resistor to an LED.

## USART1 hello

`uart_hello.c` maps USART1 TX to its default pin, PB10 (pin 30), at 115200
baud from the 8 MHz reset clock (48 MHz HSI, HPRE = /6). Before it prints, it
computes a value with `mul`, `divu` and `amoadd.w` and compares it with the
compile-time result, so the string also shows that the M and A extensions ran
correctly.

```sh
cargo run -q -p labwired-cli -- test \
  --script examples/ch32x035c8t6/uart-smoke.yaml \
  --output-dir out/boards/ch32x035c8t6/uart-smoke --no-uart-stdout
```

The smoke asserts `Hello from CH32X035` on the UART stream.

## Scope

These smokes exercise the local simulator's RV32IMAC core, the F1-layout RCC,
GPIO ports A to C (pins 0..15 only) and polled USART1 TX. Pins 16..23
(CFGXR/BSXR, including the USB and SDI pins) are not modelled. The model has
no PFIC, SysTick, timers, ADC, OPA, CMP, PIOC, I2C, SPI, DMA, USB FS or USB
PD. The RCC model reports an 8 MHz HSI, so firmware that sets HPRE to /1 runs
faster on silicon than the model clock tree shows. No CH32X035 has been
compared with the model over SDI, and CH32X035 is not a hosted LabWired
verifier target. This is local model evidence, not a hosted `model_verified`
result or hardware confirmation.
