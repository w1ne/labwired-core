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

This review draft covers ESP32, an active ATmega328PB sibling board, STM32F103,
other STM32, and RP2040, with local smoke evidence and a copy-paste example for
each entry. ATmega328P is excluded from new-design recommendations because it is
NRND. The page remains unpublished pending Andrii's approval.

## Demand-ranked coverage

| Survey position | Representative board | What the local smoke exercises | Availability receipt |
| --- | --- | --- | --- |
| **#1 ESP32, 38%** | [ESP-Mosaico V1.2 / ESP32-S31](../../examples/esp-mosaico-v1-2/README.md); [Espressif ESP32-P4X Function EV](../../examples/esp32p4x-function-ev-board/README.md); [Waveshare ESP32-C3-Zero](../../examples/waveshare-esp32c3-zero/README.md); [ESP32-S3-DevKitC-1 N16R8 v1.0](../../examples/esp32-s3-devkitc-1-n16r8/README.md); [ESP32-S3-DevKitC-1 LEDC servo system](../../examples/esp32-s3-devkitc-servo/README.md) | S31 GPIO7 function-button input drives GPIO8 haptic-control transitions; P4X GPIO8 transitions at J1 pin 5; C3-Zero GPIO10 waveform decoded as a red WS2812 pixel; DevKitC v1.0 RMT drives the GPIO48 pixel; the servo profile captures a 49.92 Hz LEDC waveform on GPIO4 and decodes a lit RGB pixel | [S31 preorder listing, checked 2026-11-09](../../examples/esp-mosaico-v1-2/evidence/vendor-status.md); [P4X listing, checked 2026-11-06](../../examples/esp32p4x-function-ev-board/evidence/vendor-status.md); [Waveshare listing, checked 2026-11-06](../../examples/waveshare-esp32c3-zero/evidence/vendor-status.md); [Espressif status, checked 2026-11-06](../../examples/esp32-s3-devkitc-1-n16r8/evidence/v10/vendor-status.md); [N16R8 stock, checked 2026-11-06](../../examples/esp32-s3-devkitc-servo/evidence/vendor-status.md) |
| **#2 ATmega328 family, 30%** | [ATmega328PB Xplained Mini](../../examples/atmega328pb-xplained-mini/README.md) (active sibling; the survey does not separate PB) | USART0 CDC transcript and the board's D200/PB5 LED transitions. ATmega328P remains excluded because it is NRND. | [Live MCU status and board stock, checked 2026-11-06](../../examples/atmega328pb-xplained-mini/evidence/vendor-status.md) |
| **#3 STM32F103, 17%** | [NUCLEO-F103RB](../../examples/nucleo-f103rb/README.md) | USART2 output and LD2 on PA5 | [ST status and stock, checked 2026-11-03](../../examples/nucleo-f103rb/evidence/vendor-status.md) |
| **#4 other STM32, 13%** | [STM32G071B-DISCO](../../examples/stm32g071b-disco/README.md); [NUCLEO-H723ZG + BME280](../../examples/nucleo-h723zg-bme280/README.md) | G071B-DISCO USART3 output on the ST-LINK virtual COM route; H723ZG reads BME280 ID `0x60` over I2C1 on PB8/PB9 and gates LD1 pulses on the read | [G071B-DISCO status and stock, checked 2026-11-03](../../examples/stm32g071b-disco/evidence/vendor-status.md); [H723ZG and BME280 status, checked 2026-11-09](../../examples/nucleo-h723zg-bme280/evidence/vendor-status.md) |
| **#5 RP2040, 12%; RP2350 is the current successor in target #2** | [Raspberry Pi Pico](../../examples/pico/README.md); [Pico 2 / RP2350](../../examples/pico2/README.md) | Pico: UART0, GP25 LED, and internal temperature ADC. Pico 2: UART0, GP25 LED, and GP26/ADC0 conversion from a modeled potentiometer. RP2350 was not a separate survey row. | [Pico availability checked 2026-11-06](../../examples/pico/evidence/vendor-status.md); [Pico 2 checked 2026-11-09](../../examples/pico2/evidence/adc/vendor-status.md) |

### #1 — ESP32: ESP-Mosaico V1.2 / ESP32-S31

This board is currently orderable as Espressif's Vision Combo Kit preorder.
Use the S31 target and drive the GPIO8 vibration-motor control from the active-low
GPIO7 function button:

```c
#include <stdbool.h>
#include "esp_err.h"
#include "driver/gpio.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

void app_main(void) {
    const gpio_config_t button = {
        .pin_bit_mask = 1ULL << GPIO_NUM_7,
        .mode = GPIO_MODE_INPUT,
        .pull_up_en = GPIO_PULLUP_ENABLE,
        .pull_down_en = GPIO_PULLDOWN_DISABLE,
        .intr_type = GPIO_INTR_DISABLE,
    };
    ESP_ERROR_CHECK(gpio_config(&button));
    ESP_ERROR_CHECK(gpio_reset_pin(GPIO_NUM_8));
    ESP_ERROR_CHECK(gpio_set_direction(GPIO_NUM_8, GPIO_MODE_OUTPUT));
    for (;;) {
        ESP_ERROR_CHECK(gpio_set_level(GPIO_NUM_8,
                                       gpio_get_level(GPIO_NUM_7) == 0));
        vTaskDelay(pdMS_TO_TICKS(10));
    }
}
```

The local smoke presses/releases GPIO7 and checks transitions on GPIO8. It
models the control pin only; it does not simulate vibration. See the
[quickstart](../../examples/esp-mosaico-v1-2/README.md),
[live purchase receipt](../../examples/esp-mosaico-v1-2/evidence/vendor-status.md),
and [committed evidence](../../examples/esp-mosaico-v1-2/evidence/).

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

### #1 — ESP32: ESP32-S3-DevKitC-1 N16R8 v1.0

Select **ESP32-S3 Dev Module** in Arduino IDE and install Adafruit NeoPixel.
The onboard addressable RGB LED is connected to GPIO48 on v1.0 (v1.1 uses
GPIO38). This sketch sets the onboard LED red:

```cpp
#include <Adafruit_NeoPixel.h>

Adafruit_NeoPixel led(1, 48, NEO_GRB + NEO_KHZ800);

void setup() {
    led.begin();
    led.setPixelColor(0, led.Color(255, 0, 0));
    led.show();
}

void loop() {}
```

The repository smoke uses RMT channel 0 and the GPIO48 output matrix, then
checks that the local model decodes a non-black pixel. It does not assert the
pixel's color. It runs the board's fast-boot bare-metal profile; it does not
verify ESP-IDF startup or the Arduino library path. A hosted verification
attempt emitted the UART marker but decoded a black pixel, and a separate
Adafruit NeoPixel library probe decoded no pixels. This draft does not claim
hosted LED verification. See the
[board instructions and smoke](../../examples/esp32-s3-devkitc-1-n16r8/README.md),
[revision status receipt](../../examples/esp32-s3-devkitc-1-n16r8/evidence/v10/vendor-status.md),
and [local evidence](../../examples/esp32-s3-devkitc-1-n16r8/evidence/v10/).

### #2 — ATmega328 family: active ATmega328PB sibling

The survey reports ATmega328 at 30%, but does not separate the P and PB variants.
Microchip's live [ATmega328P product page](https://www.microchip.com/en-us/product/ATmega328P)
reported **Not Recommended for new designs** when checked in real Chrome on
2026-11-06, so this page does not recommend new ATmega328P designs. Microchip's
live [ATmega328PB page](https://www.microchip.com/en-us/product/ATmega328PB)
reported **In Production**; its [Xplained Mini kit](https://www.microchip.com/en-us/development-tool/ATMEGA328PB-XMINI)
was listed Active and in stock by DigiKey. This is a secondary active sibling
for family continuity, not a new demand rank. See the [availability receipt](../../examples/atmega328pb-xplained-mini/evidence/vendor-status.md).

Use an AVR-GCC setup for `atmega328pb`; the user's onboard LED is D200 on PB5:

```c
#include <stdint.h>

#define DDRB  (*(volatile uint8_t *)0x24)
#define PORTB (*(volatile uint8_t *)0x25)

int main(void) {
    DDRB |= (1u << 5);
    for (;;) {
        PORTB ^= (1u << 5);
        for (volatile uint16_t i = 0; i < 50000; ++i) {}
    }
}
```

The repository smoke asserts both the USART0 marker and PB5 transitions in the
focused profile. The mEDBG microcontroller and USB implementation are not simulated.

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

### #4 — other STM32: NUCLEO-H723ZG with BME280

Install the Adafruit BME280 library and Unified Sensor dependency, select
**NUCLEO-H723ZG**, and connect the breakout to 3V3, GND, Arduino D15/SCL (PB8),
and D14/SDA (PB9). The default `Wire` bus uses I2C1 on those pins:

```cpp
#include <Wire.h>
#include <Adafruit_BME280.h>

Adafruit_BME280 bme;

void setup() {
  Serial.begin(115200);
  Wire.begin();
  if (!bme.begin(0x77, &Wire)) {
    Serial.println("BME280 not found");
    while (true) delay(10);
  }
  Serial.println("NUCLEO_H723ZG_BME280_OK");
}

void loop() {
  Serial.print("Temperature C: ");
  Serial.println(bme.readTemperature());
  delay(2000);
}
```

The committed local smoke reads the BME280 ID (`0x60`) through I2C1 and gates
LD1 transitions on that read. It covers the tested sensor path only; it is not
a physical-board check. See the [board quickstart](../../examples/nucleo-h723zg-bme280/README.md),
[live availability receipt](../../examples/nucleo-h723zg-bme280/evidence/vendor-status.md),
and [committed evidence](../../examples/nucleo-h723zg-bme280/evidence/).

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

[Raspberry Pi Pico 2](../../examples/pico2/README.md) adds an ADC0 smoke
example: GP26 reads a potentiometer wiper at the modeled 50% midpoint and
reports 2048 counts, while GP25 drives the onboard LED. Its copy-paste
bare-metal reference and smoke command are in the board guide. The smoke does
not validate Pico SDK ADC calls, USB, or physical hardware. The official
product page says **Available now from $5** and guarantees production through
January 2040; see the [exact-SKU availability receipt](../../examples/pico2/evidence/adc/vendor-status.md).

For a physical setup, connect the potentiometer wiper to GP26 (header pin 31),
its high terminal to ADC_VREF (pin 35), and its low terminal to AGND (pin 33).

This is the GP26/ADC0 sampling core used by the committed bare-metal example:

```rust
const ADC_CS: *mut u32 = 0x400A_0000 as *mut u32;
const ADC_RESULT: *const u32 = 0x400A_0004 as *const u32;

unsafe fn read_gp26_adc0() -> u16 {
    core::ptr::write_volatile(ADC_CS, 1 | (1 << 2)); // EN | START_ONCE
    (core::ptr::read_volatile(ADC_RESULT) & 0x0fff) as u16
}
```

The [Pico 2 board guide](../../examples/pico2/README.md) contains the complete
firmware, wiring, and copy-paste smoke command. The model uses an ideal 3.3 V
reference; it does not model ADC offset, noise, or calibration.

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

WCH's own GPIO_Toggle example, the source of the snippet above, now runs
unmodified in the CH32V003EVT profile. The local run prints
`SystemClk:24000000`, `ChipID:00000000`, and `GPIO Toggle TEST`, then toggles
PD0 every `Delay_Ms(250)` through the modeled QingKe SysTick. The board's
default fit leaves the HSE crystal unconnected, so the SDK falls back to the
24 MHz HSI. The chip ID reads zero because the factory word is not modeled.
Interrupts now work too: a separate PFIC profile runs WCH's unmodified
SYSTICK_Interrupt example, and an SDK-built smoke takes SysTick and software
interrupts (PFIC enable, mask and pend). Nesting and interrupts from the
other peripherals are not modeled yet. Eleven examples from
[ch32fun](https://github.com/cnlohr/ch32fun), the community's own toolchain, also
run unmodified: `blink` (four pins at exactly `Delay_Ms(250)`), `uartdemo`
(`Count: N` on USART1), `systick_irq`, whose 1 kHz ISR counts 500 ms per
loop, `adc_polled`, whose printed 10-bit codes (`511`, `255`, `1023`)
follow a simulated potentiometer on PD4 as the test turns it, and
`i2c_oled`, which draws its test patterns on a 0.91" 128x32 SSD1306 over
I2C1; the smoke checks the drawn diagonals pixel by pixel. `tim2_pwm`
drives three TIM2 PWM outputs (PD4, PD3, PD7); a decoder checks every
256-cycle period of the captured window against the duty ramp the example
programs, and `tim1_pwm` does the same on the advanced timer TIM1, including
its complementary output CH1N on PD0. `systick_irq` prints through ch32fun's default console, the WCH-LinkE
debug-printf channel over SWIO, which the simulator now models from the
probe's side. The CH32V003EVT smoke path also exercises
GPIO, USART1 transmit, and I2C1 with an SHT31 status read. The SHT31 path is a modeled transaction;
CRC, sensor conversion behavior, and physical parity are not claimed. WCH does
not publish a formal lifecycle label on its product page; the receipt records
current reseller stock for the MCU and evaluation board instead. See the
[CH32V003 guide](../../examples/ch32v003-evt/README.md) and [availability
receipt, checked 2026-11-06](../../examples/ch32v003-evt/evidence/vendor-status.md).

The next WCH step is the CH32V203C8T6, a QingKe V4B (RV32IMAC) part, on the
WeAct BluePill-Plus-CH32. Its smoke runs on the reset 8 MHz HSI. It checks a
USART1 marker on PA9, and the UART output only appears once PA9 is set to
alternate function. It also checks edges on the blue PB2 LED. PLL setup,
interrupts, the user key, USB and the other peripherals are not modeled. See the
[BluePill-Plus-CH32 guide](../../examples/weact-bluepill-plus-ch32v203/README.md)
and [availability receipt, checked 2026-11-06](../../examples/weact-bluepill-plus-ch32v203/evidence/vendor-status.md).

## Evidence and limits

Each linked example contains its reproducible smoke command, a narrow list of
modeled peripherals, and committed local run output. These are repository
simulation records. No physical capture or hosted `model_verified` result is
claimed by this page. Vendor availability changes; follow each linked receipt
to the source page and recheck before purchasing.
