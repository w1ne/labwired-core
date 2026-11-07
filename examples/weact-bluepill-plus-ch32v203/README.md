# WeAct BluePill-Plus-CH32 (CH32V203C8T6)

**Demand position:** Tier C vendor bet (WCH), downstream of CH32V003. The
CH32V203 has no row in the JetBrains State of Embedded survey; it is the WCH
QingKe V4B part that hobbyists use as a RISC-V "blue pill". This is the second
WCH part and the first RV32IMAC (V4-series) QingKe profile in LabWired.

## What this profile runs

Firmware boots RV32IMAC code from the CH32V203C8T6 memory map (64 KB flash at
0x08000000, 20 KB SRAM), stays on the reset 8 MHz HSI, enables GPIOA, GPIOB
and USART1 through RCC_APB2PCENR, configures PA9 as alternate-function
push-pull, sets 115200 baud, polls TXE, and sends `CH32V203_BLUEPILL_PLUS_OK`.
It then toggles the blue user LED on PB2. The smoke asserts the serial marker
and at least eight PB2 edges; the committed run recorded 34.

The UART output is pad-gated: rebuilding the same firmware without the PA9
alternate-function setting produces no UART bytes and the smoke fails.

The engine reuses its STM32F1-shaped RCC, GPIO and USART models for the
exercised subset, because the CH32V20x registers used here share their
offsets and bit positions. This is not a claim that the CH32V203 clock tree
or other peripherals are STM32F1-compatible.

**Not modeled:** PFIC/interrupts, PLL/HSE clock setup (WCH's `SystemInit`
raises the clock to 96/144 MHz), the PA0 user key, USB-C, CAN, timers, ADC,
RTC, the unplaced SPI flash, WCH SDI debug, and physical-board parity.

## Copy-paste example (WCH SDK / MounRiver Studio)

From WeAct's [CH32V203 template](https://github.com/WeActStudio/WeActStudio.BluePill-Plus-CH32/tree/master/Examples/CH32V203/01-template).
Connect PA9 to a USB-UART adapter's RX input at 115200 8N1.

```c
#include "debug.h"

int main(void) {
    GPIO_InitTypeDef gpio = {0};

    Delay_Init();
    USART_Printf_Init(115200);          /* USART1, TX on PA9 */
    printf("CH32V203_BLUEPILL_PLUS_OK\r\n");

    RCC_APB2PeriphClockCmd(RCC_APB2Periph_GPIOB, ENABLE);
    gpio.GPIO_Pin = GPIO_Pin_2;          /* blue LED, active high */
    gpio.GPIO_Mode = GPIO_Mode_Out_PP;
    gpio.GPIO_Speed = GPIO_Speed_50MHz;
    GPIO_Init(GPIOB, &gpio);

    for (;;) {
        GPIO_SetBits(GPIOB, GPIO_Pin_2);
        Delay_Ms(250);
        GPIO_ResetBits(GPIOB, GPIO_Pin_2);
        Delay_Ms(250);
    }
}
```

The simulator smoke uses the bare register-level equivalent in
[`firmware.rs`](firmware.rs), not the WCH SDK, because the SDK's startup
raises the clock through the PLL, which this profile does not model.

## Run the smoke

```bash
examples/weact-bluepill-plus-ch32v203/build-firmware.sh
cargo run -q -p labwired-cli -- test \
  --script examples/weact-bluepill-plus-ch32v203/onboarding-smoke.yaml
```

Evidence: [`evidence/`](evidence/) (run log, UART transcript, result JSON,
availability receipt).

## Unmodified ch32fun on the QingKe V4 profile

A second chip profile, [`ch32v203c8t6-ch32fun-smoke`](../../configs/chips/ch32v203c8t6-ch32fun-smoke.yaml),
runs [ch32fun](https://github.com/cnlohr/ch32fun) (MIT) CH32V20x examples
built with ch32fun's own compile and link lines and with no source changes
([`build-ch32fun-examples.sh`](build-ch32fun-examples.sh)). The ELFs are
committed in [`ch32fun/`](ch32fun/).

It adds what those examples need and the narrow profile above lacks:

- **The QingKe V4 core profile** (`core: qingke-v4b`, the name ch32-rs/ch32-data
  uses). It accepts the CORECFGR CSR (0xBC0) that ch32fun's startup writes,
  and it takes PFIC interrupts through the mtvec mode-3 address table, using
  the QingKe V2 trap path. A generic RV32 core traps on that CSR at reset.
- **The QingKe V4 SysTick** (`qingke_v4_systick`). It has a 64-bit CNT and CMP,
  counts up or down (MODE), and supports INIT. ch32fun's `DelaySysTick` polls
  the 64-bit counter.
- **The HSI ×18 PLL handshake and EXTEN_CTR.PLL_HSI_PRE** that ch32fun's
  SystemInit performs. The model states the resulting 144 MHz as `cpu_hz`; it
  does not derive it.
- **EXTI on the PFIC** (`profile: ch32v20x`). The sixteen GPIO lines drive
  PFIC 22..26, 39 and 56 as level requests, and AFIO_EXTICR1..4 select the port.
- **The WCH-LinkE debug printf channel** at 0xE0000380.

| Smoke | ch32fun example | What it proves | Result |
| --- | --- | --- | --- |
| [`ch32fun-blink-smoke.yaml`](ch32fun-blink-smoke.yaml) | `examples_v20x/blink` | PA0/PA2/PA4/PA7 edges 36,000,015 cycles apart, i.e. 250.0001 ms at 144 MHz; RCC CFGR0 SWS=PLL, PLLMUL ×18 | 6/6 |
| [`ch32fun-exti-blink-smoke.yaml`](ch32fun-exti-blink-smoke.yaml) | `examples_v20x/exti_blink` | each rising PA1 edge enters EXTI1_IRQHandler (PFIC 23), which toggles PA10 ~30 cycles later; the release does nothing | 9/9 |
| [`ch32fun-debugprintf-smoke.yaml`](ch32fun-debugprintf-smoke.yaml) | `examples_v20x/debugprintfdemo` | `+0`, `-1[`, … `-5[` on the debug-printf terminal; the probe took every request | 5/5 |

Negative controls (committed with their logs):
- `evidence/ch32fun-blink-smoke/negative-control/`: with `core: rv32imac`,
  the core traps (mcause 2) on `csrw 0xbc0` at 0x1cc in `handle_reset`, and
  no pad toggles (0/6).
- `evidence/ch32fun-exti-blink-smoke/negative-control/`: with the EXTI on the
  STM32F1 profile, INTFR latches but no PFIC request is raised, and PA10
  never toggles (7/9).

```bash
examples/weact-bluepill-plus-ch32v203/build-ch32fun-examples.sh   # optional: ELFs are committed
cargo run -q -p labwired-cli -- test \
  --script examples/weact-bluepill-plus-ch32v203/ch32fun-exti-blink-smoke.yaml
```

**Not modelled in this profile:** USART/SPI/I2C/timers/ADC/DMA/USB/CAN/RTC,
PFIC nesting and V4 priority grouping, HSE, low-power modes, host-to-target
debug input, and physical-board parity. The PD0/PC0 toggles in
debugprintfdemo are not asserted: on LQFP48, PD0 is OSC_IN and PC0 is not
bonded.
