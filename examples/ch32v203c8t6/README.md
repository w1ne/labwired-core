# CH32V203C8T6 (LQFP48) GPIO blink, USART1, SPI2 DMA and timer interrupts

Demand position: WCH is the Tier C vendor bet in the onboarding target list;
CH32V203 has no measured JetBrains survey rank. This round adds TIM1..TIM4
coverage to the existing part, so it is capability follow-up work, not a top-ranked
demand pick.

The CH32V203C8T6 is a WCH QingKe V4B (RV32IMAC) microcontroller: 144 MHz
maximum, 64 KB zero-wait flash, 20 KB SRAM and 37 GPIOs in LQFP48
(WCH CH32V203DS0 V3.0, Table 2-1). Its RCC, GPIO and USART registers use the
STM32F1 layout (WCH CH32FV2x_V3xRM), so the descriptor runs the `stm32f1`
RCC and GPIO profiles. The core is `qingke-v4b`: RV32IMAC plus the QingKe
PFIC, CORECFGR and the 64-bit SysTick.

## Copy-paste bare-metal C example

```c
#include <stdint.h>

#define RCC_APB2PCENR (*(volatile uint32_t *)0x40021018u)
#define GPIOC_CFGHR   (*(volatile uint32_t *)0x40011004u)
#define GPIOC_BSHR    (*(volatile uint32_t *)0x40011010u)
#define GPIOC_BCR     (*(volatile uint32_t *)0x40011014u)

#define LED_BIT 13u

static void delay(void) {
    for (volatile uint32_t i = 0; i < 500; ++i) {
        __asm__ volatile("nop");
    }
}

void main(void) {
    RCC_APB2PCENR |= 1u << 4; /* enable GPIOC */
    /* PC13 is in CFGHR (pins 8..15): push-pull output, 2 MHz. */
    GPIOC_CFGHR = (GPIOC_CFGHR & ~(0xFu << ((LED_BIT - 8u) * 4u)))
                | (0x2u << ((LED_BIT - 8u) * 4u));

    for (;;) {
        GPIOC_BSHR = 1u << LED_BIT; /* set PC13 */
        delay();
        GPIOC_BCR = 1u << LED_BIT;  /* reset PC13 */
        delay();
    }
}
```

Build both fixtures with any RISC-V GCC that supports `-march=rv32imac`:

```sh
./examples/ch32v203c8t6/build.sh
```

The committed `tests/fixtures/ch32v203c8t6-blink.elf` and
`tests/fixtures/ch32v203c8t6-uart-hello.elf` were built with
`riscv32-esp-elf-gcc` 14.2.0 (crosstool-NG `esp-14.2.0_20260121`). Their
sha256 values are
`07be2450677fc694805bddefa16050ec5a66f7c4ee75fb55c7c5a92e2c50c584` and
`5d0a1fcec9b6423ba0d51746c1f720e5a878bc416c402c78428cbcd4da63c45f`. The
smokes check behaviour, not the hash.

Run the blink against the local model:

```sh
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v203c8t6/io-smoke.yaml \
  --output-dir out/boards/ch32v203c8t6/io-smoke --no-uart-stdout
```

The smoke requires at least two PC13 output edges. On a physical part, wire
PC13 (pin 2) through a current-limiting resistor to an LED.

## USART1 hello

`uart_hello.c` maps USART1 TX to its default pin, PA9, at 115200 baud from the
8 MHz HSI reset clock. Before it prints, it computes a value with `mul`,
`divu` and `amoadd.w` and compares it with the compile-time result, so the
string also shows that the M and A extensions ran correctly.

```sh
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v203c8t6/uart-smoke.yaml \
  --output-dir out/boards/ch32v203c8t6/uart-smoke --no-uart-stdout
```

The smoke asserts `Hello from CH32V203` on the UART stream.

## ch32fun examples and local USART fixture

`build-ch32fun-examples.sh` builds four unmodified ch32fun (cnlohr/ch32fun,
MIT) examples for `TARGET_MCU=CH32V203` and local USART1 RX IRQ and SPI2 DMA fixtures into
`ch32fun/`. ch32fun prints through
the WCH-LinkE debug channel at 0xE0000380 by default, which the descriptor
models as an attached, polling probe (`debug_printf`, log `terminal`).

| Smoke | What it proves |
| --- | --- |
| `ch32fun-blink-smoke.yaml` | `SystemInit` PLL handshake (x18, `EXTEN_CTR.PLL_HSI_PRE`); PA0/PA2/PA4/PA7 toggle every `Delay_Ms(250)` on the 64-bit SysTick, 36,000,014 cycles apart at 144 MHz |
| `ch32fun-debugprintf-smoke.yaml` | `+N` / `-N[c]` lines on the debug channel; DMDATA0 bits 7:6 read 0, so no print timed out |
| `ch32fun-systick-irq-smoke.yaml` | `SysTick_Handler` through PFIC interrupt 12, once per millisecond: the loops count 499 and 500 interrupts per 500 ms (see below) |
| `ch32fun-exti-blink-smoke.yaml` | `EXTI1_IRQHandler` through PFIC interrupt 23 (`exti` `profile: wch_v20x`): a PA1 press, release, press toggles PA10 exactly twice (edges at cycles 92, 2,000,031, 6,000,031); `exti-system.yaml` adds the PA1 button and PA10 LED |
| `usart-rx-irq-smoke.yaml` | Injected `wch!` reaches USART1 RXNE; PFIC interrupt 53 runs the handler, which reads DATAR before USART1 TX prints `USART1_IRQ_RX=wch!` |
| `spi-dma-smoke.yaml` | SPI2 TX requests pace DMA1 channel 5 (circular, 8 half-words); PFIC interrupt 31 logs HT then TC for three laps; USART1 prints `SPI2_DMA events=HTHTHT laps=3 cntr=8` |

```sh
for s in blink debugprintf systick-irq exti-blink; do
  cargo run -q -p labwired-cli -- test \
    --script examples/ch32v203c8t6/ch32fun-$s-smoke.yaml \
    --output-dir out/boards/ch32v203c8t6/ch32fun-$s --no-uart-stdout
done
for s in usart-rx-irq spi-dma; do
  cargo run -q -p labwired-cli -- test \
    --script examples/ch32v203c8t6/$s-smoke.yaml \
    --output-dir out/boards/ch32v203c8t6/$s --no-uart-stdout
done
```

The committed runs are under `evidence/`. In `systick_irq`, `CNTIF` is still
set from the boot-time `Delay_Ms` when `systick_init` sets `STIE`, so the model
pends one interrupt at once. This moves CMP one millisecond ahead, so the first
loop counts 499. That enable-time interrupt is a model assumption, not a
silicon capture. The example's "Current Microseconds" and "SysTick->CNT" lines
pass a 64-bit value to `%lu` (gcc warns), so their text is not asserted.

## USART RX IRQ

The [complete `ch32fun` source](./usart_rx_irq.c) configures PA9/PA10 for
USART1, enables RXNEIE, and receives the four injected bytes in
`USART1_IRQHandler`. The CLI smoke asserts that the handler's `wch!` bytes are
transmitted back over USART1 TX. Rebuild the fixture and run it with:

```sh
RISCV_PREFIX=~/tools/riscv64-deb/root/usr \
NEWLIB=~/tools/riscv64-deb/root/usr/lib/picolibc/riscv64-unknown-elf/include \
  ./examples/ch32v203c8t6/build-ch32fun-examples.sh
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v203c8t6/usart-rx-irq-smoke.yaml \
  --output-dir out/boards/ch32v203c8t6/usart-rx-irq --no-uart-stdout
```

## SPI2 TX DMA

The [complete `ch32fun` source](./spi_dma_tx.c) runs SPI2 as a 16-bit master
on PB13/PB15 and feeds it from DMA1 channel 5, the RM's hard-wired SPI2_TX
channel, in circular half-word mode. As in ch32fun's `spidmatest`, it sets
CTLR2.TXDMAEN before CFGR.EN, so the SPI request is already pending when the
channel starts. `DMA1_Channel5_IRQHandler` logs `H` on half transfer and `T`
on transfer complete, and disables the channel after the third lap.

ch32fun's own `spidmatest` is not used as the smoke: it loads CNTR while the
channel is enabled, which the RM states is read-only once EN is set, so its
behaviour on silicon is not defined by the documentation.

## Timer interrupts

The [complete `ch32fun` source](./tim_irq.c) runs TIM2 and TIM1 at PSC 143 and
ATRLR 999. SystemInit leaves both timer kernels at 144 MHz, so each timer
counts at 1 MHz and updates at 1 kHz. TIM2 enables only the update interrupt
(PFIC IRQ 44). TIM1 enables only the CH1 compare interrupt at CH1CVR 250, so
the event must reach TIM1_CC_IRQn 43 and not TIM1_UP_IRQn 41. The TIM2
handler reads the 64-bit SysTick (HCLK/8) at update 1 and at update 11, then
stops both timers. USART1 prints:

```text
TIM2_UP=11 ms=10 TIM1_CC1=11
```

```sh
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v203c8t6/tim-irq-smoke.yaml \
  --output-dir out/boards/ch32v203c8t6/tim-irq-smoke --no-uart-stdout
```

`ch32_compiled_firmware::ch32v203_tim2_update_stays_unhandled_without_irq_44_wired`
is the negative control: with the TIM2 `irq` removed from the descriptor, the
firmware prints `TIM_TIMEOUT up=0`.

## Scope

These smokes exercise the local simulator's QingKe V4B core, the PFIC, the
64-bit SysTick, the debug printf channel, the F1-layout RCC, GPIO ports A to D
and USART1/USART2 TX. USART1 RXNE is exercised through PFIC IRQ 53; USART2
IRQ 54 is wired in the descriptor but not firmware-tested. SPI2 TX on DMA1
channel 5 with its PFIC IRQ 31 is exercised; SPI1, SPI RX, the other DMA
channels and memory-to-memory DMA run the same F1 models but have no V203
smoke. The SPI high-speed bit (HSCR) has no timing effect. TIM2 update
(IRQ 44) and TIM1 CH1 compare (IRQ 43) are exercised; TIM1 BRK/UP/TRG_COM
(40..42) and TIM3/TIM4 (45/46) are wired but not firmware-tested. Timer
channel outputs and capture inputs are not routed to GPIO pads, so PWM on a
pin and input capture do not work on this part in the model. The model has
no ADC, I2C, CAN or USB, no PFIC interrupt nesting and no V4 priority
grouping. The RCC
decodes PLLMUL with the STM32F1 table, so a PLLMUL setting of x18 reads as x16
in the model; `cpu_hz` states the 144 MHz instead. No CH32V203 has been
compared with the model over SDI, and CH32V203 is not a hosted LabWired
verifier target. This is local model evidence, not a hosted `model_verified`
result or hardware confirmation.

In `exti_blink`, the F1 EXTI vectors (6..10, 23, 40) land on QingKe core
slots and reserved PFIC slots, so the `stm32f1` profile is a negative
control: `evidence/ch32fun-exti-blink-smoke/negative-control/` fails 5/7
(no PA10 toggle, INTFR line 1 stays set). The PFIC is decoded inside the
core, not on the bus, so the smoke cannot read its ISR/IPR words with
`memory_value`.
