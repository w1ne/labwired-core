# WCH CH32V003F4P6 on the CH32V003EVT

**Demand rank:** Tier C strategic vendor bet, not a measured MCU rank in the
JetBrains survey. The measured ranks remain ESP32 (38%), ATmega328 (30%),
STM32F103 (17%), other STM32 (13%), and RP2040 (12%). This round extends the
first WCH/QingKe profile from GPIO/USART1 coverage to I2C1 with an SHT31 sensor.

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

The I2C profile adds the CH32V003's reset-mapped I2C1 pins (PC1/SDA and PC2/SCL)
and a clock-gated I2C1 register window. Firmware sends the SHT31 status command
`0xF32D`, reads the modeled power-on status `0x8010`, and prints
`CH32V003_SHT31_STATUS_OK` on USART1 only when both bytes match. The smoke
therefore exercises the sensor transaction, not merely the MCU's I2C registers.
I2C slave mode, alternate pin remaps, timing accuracy, CRC checking, sensor
conversion behavior, and physical-board parity remain outside this profile.

The engine reuses its STM32F1-shaped GPIO, USART, and I2C register behavior for
the exercised subset. WCH's I2C1 CR1/CR2/DR event map shares the offsets used by
that controller model; the CH32-specific clock gate and default PC1/PC2 I2C pad
mapping are declared separately. The USART descriptor selects the separate
CH32 PD5 TX pad map. This is not a claim that the CH32V003 clock tree or all of
its peripherals are STM32F1-compatible.

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

## Copy-paste SHT31 status read

The [official CH32 Arduino core](https://github.com/openwch/arduino_core_ch32)
supports I2C master mode for CH32V003. Its default `Wire` pins are PC1/SDA and
PC2/SCL. Connect a 3.3 V SHT31-D breakout to those pins and GND.

```cpp
#include <Wire.h>

void setup() {
  Serial.begin(115200);
  Wire.begin();  // CH32V003 default I2C1: PC1/SDA, PC2/SCL

  Wire.beginTransmission(0x44);
  Wire.write(0xF3);  // SHT31 read-status command
  Wire.write(0x2D);
  if (Wire.endTransmission() != 0) {
    Serial.println("SHT31 command failed");
    return;
  }

  delay(1);
  if (Wire.requestFrom((uint8_t)0x44, (uint8_t)2) != 2) {
    Serial.println("SHT31 status read failed");
    return;
  }
  uint16_t status = ((uint16_t)Wire.read() << 8) | Wire.read();
  Serial.print("SHT31 status: 0x");
  Serial.println(status, HEX);
}

void loop() {}
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

examples/ch32v003-evt/build-sht31-firmware.sh
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v003-evt/i2c-smoke.yaml \
  --output-dir /tmp/ch32v003-sht31-smoke
```

The committed I2C run evidence is in [`evidence/sht31-smoke/`](evidence/sht31-smoke/);
the earlier GPIO and USART receipts are alongside it. The fixtures are
handwritten assembly without calls or an ABI-dependent stack frame because the
upstream Rust toolchain warns that its RV32E ABI support is incomplete. These
are deterministic local engine runs, not physical-board captures or hosted
`labwired_verify` results.
