# Hardware coverage quickstarts — review draft

> **Publication approval: not granted.** This is a repository draft for review;
> do not publish, deploy, or present it as the approved community coverage page
> until Andrii explicitly approves it.

This page points to real, purchasable boards with firmware smoke evidence in
LabWired. “Smoke passed” below means a committed local simulator run for the
listed path. It does not mean a physical-board test, and it does not imply that
other peripherals on the board are modeled.

## Why these boards

JetBrains' [State of Embedded Development 2025](https://lp.jetbrains.com/the-state-of-embedded-2025/)
reports MCU usage of ESP32 38%, ATmega328 30%, STM32F103 17%, other STM32 13%,
and RP2040 12%. It reports kit use of ESP32-based boards at 42%, Arduino at
37%, and STM32 Nucleo/Discovery at 23%. Those are survey shares, not market
share estimates. RP2350 has no separate row in that survey.

This review draft now includes a focused ESP32-P4X Function EV board profile
and its GPIO8 smoke. It remains unpublished pending Andrii's approval.

## Demand-ranked coverage

| Survey position | Representative board | What the local smoke exercises | Availability receipt |
| --- | --- | --- | --- |
| **#1 ESP32, 38%** | [Espressif ESP32-P4X Function EV](../../examples/esp32p4x-function-ev-board/README.md); [Waveshare ESP32-C3-Zero](../../examples/waveshare-esp32c3-zero/README.md) | P4X GPIO8 transitions at J1 pin 5; C3-Zero GPIO10 waveform decoded as a red WS2812 pixel | [P4X listing, checked 2026-11-06](../../examples/esp32p4x-function-ev-board/evidence/vendor-status.md); [Waveshare listing, checked 2026-11-06](../../examples/waveshare-esp32c3-zero/evidence/vendor-status.md) |
| **#2 ATmega328, 30%** | No current recommendation | The existing Uno R3 notes identify ATmega328P as NRND; we do not recommend buying it for a new design. | [Legacy note](../../examples/arduino-uno-blinky/README.md) |
| **#3 STM32F103, 17%** | [NUCLEO-F103RB](../../examples/nucleo-f103rb/README.md) | USART2 output and LD2 on PA5 | [ST status and stock, checked 2026-11-03](../../examples/nucleo-f103rb/evidence/vendor-status.md) |
| **#4 other STM32, 13%** | [STM32G071B-DISCO](../../examples/stm32g071b-disco/README.md) | USART3 output on the ST-LINK virtual COM route | [ST status and stock, checked 2026-11-03](../../examples/stm32g071b-disco/evidence/vendor-status.md) |
| **#5 RP2040, 12%** | [Raspberry Pi Pico](../../examples/pico/README.md) | UART0, GP25 LED edges, and a non-zero internal temperature ADC result | [Raspberry Pi status and availability, checked 2026-11-06](../../examples/pico/evidence/vendor-status.md) |

### #1 — ESP32: Espressif ESP32-P4X Function EV Board

Set the project target to `esp32p4` in ESP-IDF. GPIO8 is at J1 pin 5; attach an
LED in series with a current-limiting resistor from GPIO8 to GND, or use a
logic analyzer to observe the pin:

```c
#include "driver/gpio.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

void app_main(void) {
    gpio_reset_pin(GPIO_NUM_8);
    gpio_set_direction(GPIO_NUM_8, GPIO_MODE_OUTPUT);
    for (;;) {
        gpio_set_level(GPIO_NUM_8, 1);
        vTaskDelay(pdMS_TO_TICKS(500));
        gpio_set_level(GPIO_NUM_8, 0);
        vTaskDelay(pdMS_TO_TICKS(500));
    }
}
```

The local profile directly loads a bare-metal RISC-V image and asserts GPIO8
edges. It covers one HP execution context and GPIO8's output and IO_MUX path;
it does not claim the second HP core, LP core, ESP32-C6 module, ROM boot, flash
controller, USB, Ethernet, audio, camera, display, or other pads. See the
[focused quickstart](../../examples/esp32p4x-function-ev-board/README.md),
[availability receipt](../../examples/esp32p4x-function-ev-board/evidence/vendor-status.md),
and [committed local evidence](../../examples/esp32p4x-function-ev-board/evidence/).

### #1 — ESP32: Waveshare ESP32-C3-Zero

Install Adafruit NeoPixel in Arduino IDE, select **ESP32C3 Dev Module**, and
upload this sketch to light the onboard WS2812 on GPIO10:

```cpp
#include <Adafruit_NeoPixel.h>

Adafruit_NeoPixel led(1, 10, NEO_GRB + NEO_KHZ800);

void setup() {
  led.begin();
  led.setPixelColor(0, led.Color(255, 0, 0));
  led.show();
}

void loop() {}
```

The local smoke sends the same pixel frame using direct GPIO writes and checks
the modeled waveform and decoded color. It does not verify the Arduino core's
NeoPixel waveform path. The narrow board profile omits radio, USB, UART,
buttons, and other header pins. See the [full quickstart and smoke command](../../examples/waveshare-esp32c3-zero/README.md)
and [committed evidence](../../examples/waveshare-esp32c3-zero/evidence/).

### #2 — ATmega328: legacy status

ATmega328 is a high survey result, but Microchip's live [ATmega328P product
page](https://www.microchip.com/en-us/product/ATmega328P) reported **Not
Recommended for new designs** when checked in real Chrome on 2026-11-06. This
page therefore gives no new-design purchase or firmware recommendation for
it. The active Arduino board example below uses a different MCU. The
repository's [Uno R3 note](../../examples/arduino-uno-blinky/README.md) also
records the lifecycle restriction.

### #3 — STM32F103: NUCLEO-F103RB

In an STM32Cube project, configure USART2 on PA2/PA3 and PA5 as the active-high
LD2 output. After `MX_GPIO_Init()` and `MX_USART2_UART_Init()`, this loop sends
a marker and blinks the LED:

```c
const uint8_t message[] = "NUCLEO_F103RB_OK\r\n";
HAL_UART_Transmit(&huart2, (uint8_t *)message, sizeof(message) - 1,
                 HAL_MAX_DELAY);

for (;;) {
    HAL_GPIO_TogglePin(GPIOA, GPIO_PIN_5);
    HAL_Delay(500);
}
```

The repository smoke uses a bare-metal firmware image, not STM32Cube HAL. It
asserts UART output and PA5 transitions in the focused F103RB profile; the
button is mapped but not read, and the broad chip descriptor's USB and backup
register windows are omitted. See the [board guide](../../examples/nucleo-f103rb/README.md)
and [committed run evidence](../../examples/nucleo-f103rb/evidence/).

### #4 — other STM32: STM32G071B-DISCO

Configure USART3 for the board's ST-LINK virtual COM route on PB8/PB9 (AF0).
After `MX_USART3_UART_Init()`, this sends a marker:

```c
const uint8_t message[] = "STM32G071B_DISCO_OK\r\n";
HAL_UART_Transmit(&huart3, (uint8_t *)message, sizeof(message) - 1,
                 HAL_MAX_DELAY);
```

The local smoke uses bare-metal register writes and asserts the USART3
transcript. The focused model includes the core, RCC, GPIOB, and USART3; the
OLED, USB-C/Power Delivery, joystick, reed sensor, and ST-LINK implementation
are outside its scope. See the [board guide](../../examples/stm32g071b-disco/README.md)
and [committed run evidence](../../examples/stm32g071b-disco/evidence/).

### #5 — RP2040: Raspberry Pi Pico

Enable USB stdio in the Pico SDK project. This program reports the on-chip
temperature ADC's raw value over USB stdio. It is a convenience example for a
physical Pico; the simulator smoke captures UART0 instead of USB CDC.

```c
#include <stdio.h>
#include <stdint.h>
#include "pico/stdlib.h"
#include "hardware/adc.h"

int main(void) {
    stdio_init_all();
    adc_init();
    adc_set_temp_sensor_enabled(true);
    sleep_ms(1);
    adc_select_input(4);
    const uint16_t raw = adc_read();
    printf("RP2040_ADC_TEMP raw=%u\n", (unsigned)raw);
    for (;;) tight_loop_contents();
}
```

The smoke checks that a non-zero ADC conversion reaches the result register;
it does not claim a calibrated temperature. It also checks UART0 output and
GP25 GPIO edges. See the [Pico guide](../../examples/pico/README.md) and
[committed run evidence](../../examples/pico/evidence/).

## Other community-relevant boards

### Arduino kit use: Arduino UNO R4 Minima

Arduino is used in 37% of respondents' main projects in the same survey. The
UNO R4 Minima is an active, purchasable board based on Renesas RA4M1, not the
ATmega328. The repository smoke checks SCI2 output on D0/D1 and transitions on
the onboard D13 LED (P111). The following sketch is an Arduino-core example
for the physical board; the simulator smoke uses bare-metal firmware and does
not verify Arduino core behavior:

```cpp
void setup() {
  pinMode(LED_BUILTIN, OUTPUT);
  Serial1.begin(115200);  // D0/D1 header UART
}

void loop() {
  Serial1.println("UNO_R4_MINIMA_OK");
  digitalWrite(LED_BUILTIN, HIGH);
  delay(500);
  digitalWrite(LED_BUILTIN, LOW);
  delay(500);
}
```

See the [UNO R4 Minima guide](../../examples/arduino-uno-r4-minima/README.md)
for the smoke command, model limits, and the [availability receipt](../../examples/arduino-uno-r4-minima/README.md#vendor-status-checked-2026-11-03).
The UNO R4 WiFi profile is also available, but its ESP32-S3 coprocessor and
wireless features are outside the profile; see its [guide](../../examples/arduino-uno-r4-wifi/README.md)
and [vendor receipt](../../examples/arduino-uno-r4-wifi/evidence/vendor-status.md).

### RP2350: newer Raspberry Pi board

RP2350 is not a separate JetBrains MCU survey row. The demand brief treats it
as the current successor alongside RP2040. [Waveshare RP2350-Zero](../../examples/rp2350-zero/README.md)
has a copy-paste Arduino NeoPixel sketch and a local smoke that checks UART0,
GP16 edges, and the decoded onboard WS2812. Its USB CDC console is not modeled;
see its [availability receipt, checked 2026-11-06](../../examples/rp2350-zero/evidence/vendor-status.md).

```cpp
#include <Adafruit_NeoPixel.h>

Adafruit_NeoPixel led(1, 16, NEO_GRB + NEO_KHZ800);

void setup() {
  led.begin();
  led.setPixelColor(0, led.Color(255, 0, 0));
  led.show();
}

void loop() {}
```

### Tier C vendor bet: WCH CH32V003

CH32V003 has no measured MCU rank in the JetBrains table. It is the brief's
strategic community target for low-cost RISC-V and WCH ecosystem coverage. This
WCH SDK example blinks an external LED on PD0; add the LED and a current-limiting
resistor from PD0 to GND:

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

    for (;;) {
        Delay_Ms(250);
        GPIO_WriteBit(GPIOD, GPIO_Pin_0,
                      (state == 0) ? (state = Bit_SET) : (state = Bit_RESET));
    }
}
```

The CH32V003EVT smoke path boots firmware and exercises GPIO, USART1 transmit,
and I2C1 with an SHT31 status read. The SHT31 path is a modeled transaction;
CRC, sensor conversion behavior, and physical parity are not claimed. WCH does
not publish a formal lifecycle label on its product page; the receipt records
current reseller stock for the MCU and evaluation board instead. See the
[CH32V003 guide](../../examples/ch32v003-evt/README.md) and [availability
receipt, checked 2026-11-06](../../examples/ch32v003-evt/evidence/vendor-status.md).

## Evidence and limits

Each linked example contains its reproducible smoke command, a narrow list of
modeled peripherals, and committed local run output. These are repository
simulation records. No physical capture or hosted `model_verified` result is
claimed by this page. Vendor availability changes; follow each linked receipt
to the source page and recheck before purchasing.
