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
