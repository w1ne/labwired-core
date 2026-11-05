# WCH CH32V003F4P6 on the CH32V003EVT

**Demand rank:** Tier C strategic vendor bet, not a measured MCU rank in the
JetBrains survey. The measured ranks remain ESP32 (38%), ATmega328 (30%),
STM32F103 (17%), other STM32 (13%), and RP2040 (12%). This round extends the
first WCH/QingKe profile from GPIO-only to USART1 transmit.

## What these profiles run

The GPIO profile boots RV32EC firmware from the CH32V003F4P6 memory map, stays
at the reset 24 MHz clock, enables GPIOD through APB2PCENR, configures PD0 as
push-pull output, and toggles it. Its smoke asserts at least eight GPIO edges.

The USART profile adds a clock-gated USART1 register block and the CH32V003
default TX route on PD5. Firmware configures PD5 for alternate-function output,
sets 115200 baud from the reset clock, polls TXE, and sends
`CH32V003_USART_OK`; the same run continues toggling PD0. Its smoke asserts
both the serial marker and GPIO edges. USART receive, PLL setup, WCH SDI, other
CH32 peripherals, supply/reset sequencing, and physical-board parity are not
modeled. Stack analysis is unsupported for this architecture.

The engine reuses its STM32F1-shaped GPIO and USART register behavior for the
subset used by the smoke. The `ch32v003` RCC alias resolves APB2PCENR at offset
`0x18`; the USART descriptor selects the separate CH32 PD5 pad map explicitly.
This is not a claim that the CH32V003 clock tree or all of its peripherals are
STM32F1-compatible.

For the GPIO smoke, connect an LED and current-limiting resistor between PD0
and GND. The [WCH-CH32V003EVT board reference](https://www.olimex.com/Products/RISC-V/WCH-CH32V003EVT/resources/CH32V00x-Evaluation-Board-Reference-EN.pdf)
documents the board's LED-row pins; `external_led_pd0` is the explicit test
connection, not a factory-installed PD0 LED. For USART, connect PD5 to a
USB-UART adapter's RX input and share ground. The example uses 115200 8N1.

## Copy-paste GPIO example

This follows WCH's official [GPIO_Toggle sample](https://github.com/openwch/ch32v003/blob/main/EVT/EXAM/GPIO/GPIO_Toggle/User/main.c).
The full WCH SDK supplies `debug.h` and delay helpers.

```c
#include "debug.h"

int main(void) {
    u8 state = 0;
    GPIO_InitTypeDef gpio = {0};

    SystemCoreClockUpdate();
    Delay_Init();
    RCC_APB2PeriphClockCmd(RCC_APB2Periph_GPIOD, ENABLE);
    gpio.GPIO_Pin = GPIO_Pin_0;
    gpio.GPIO_Mode = GPIO_Mode_Out_PP;
    gpio.GPIO_Speed = GPIO_Speed_30MHz;
    GPIO_Init(GPIOD, &gpio);

    while (1) {
        Delay_Ms(250);
        GPIO_WriteBit(GPIOD, GPIO_Pin_0,
                      (state == 0) ? (state = Bit_SET) : (state = Bit_RESET));
    }
}
```

## Copy-paste USART1 transmit example

This follows WCH's official [USART_IAP example](https://github.com/openwch/ch32v003/blob/main/EVT/EXAM/USART_IAP/CH32V003_IAP/User/iap.c),
using transmit-only mode at 115200 baud.

```c
#include "debug.h"

int main(void) {
    GPIO_InitTypeDef gpio = {0};
    USART_InitTypeDef usart = {0};

    RCC_APB2PeriphClockCmd(RCC_APB2Periph_GPIOD | RCC_APB2Periph_USART1,
                           ENABLE);
    gpio.GPIO_Pin = GPIO_Pin_5;
    gpio.GPIO_Speed = GPIO_Speed_30MHz;
    gpio.GPIO_Mode = GPIO_Mode_AF_PP;
    GPIO_Init(GPIOD, &gpio);

    usart.USART_BaudRate = 115200;
    usart.USART_WordLength = USART_WordLength_8b;
    usart.USART_StopBits = USART_StopBits_1;
    usart.USART_Parity = USART_Parity_No;
    usart.USART_HardwareFlowControl = USART_HardwareFlowControl_None;
    usart.USART_Mode = USART_Mode_Tx;
    USART_Init(USART1, &usart);
    USART_Cmd(USART1, ENABLE);

    for (;;) {
        while (USART_GetFlagStatus(USART1, USART_FLAG_TXE) == RESET) {}
        USART_SendData(USART1, 'O');
    }
}
```

## Build and run

```sh
examples/ch32v003-evt/build-firmware.sh
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v003-evt/onboarding-smoke.yaml \
  --output-dir /tmp/ch32v003-smoke

examples/ch32v003-evt/build-usart-firmware.sh
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v003-evt/usart-smoke.yaml \
  --output-dir /tmp/ch32v003-usart-smoke
```

The USART run evidence is in [`evidence/usart-smoke/`](evidence/usart-smoke/).
The fixture is handwritten assembly without calls or an ABI-dependent stack
frame because the upstream Rust toolchain warns that its RV32E ABI support is
incomplete. These are deterministic local engine runs, not physical-board
captures or hosted `labwired_verify` results.
