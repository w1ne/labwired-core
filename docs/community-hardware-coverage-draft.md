# Community hardware coverage — draft

**Draft only. Not published.** Publication still needs Andrii's explicit OK.

This page starts the demand-ranked coverage list with runnable board examples.
Each entry links to the exact hardware scope, copy-paste example, vendor status,
and simulator evidence. More demand-ranked targets remain to be added.

## ESP32 — rank 1, 38% of reported MCU use

### ESP32-C6-DevKitC-1-N8 + BME280

The exact N8 board is listed in Espressif's current user guide; the dated
2026-11-10 receipt records 178 in stock at DigiKey. The 8 MB profile
boots a direct-register smoke that reads BME280 ID `0x60` and raw temperature
over I2C0 on GPIO6/GPIO7. This adds board depth on the existing ESP32-C6 model,
not new silicon.

- [Wiring, scope, and copy-paste example](../examples/esp32c6-devkitc-1-n8-bme280/README.md)
- [Dated Espressif, DigiKey, and Adafruit availability receipt](../examples/esp32c6-devkitc-1-n8-bme280/evidence/bme280/vendor-status.md)
- [Committed local smoke evidence](../examples/esp32c6-devkitc-1-n8-bme280/evidence/bme280/)

The hosted catalog lists the C6 DevKitC as `design_only`, so this draft links
the exact local CLI smoke and does not claim hosted `labwired_verify` coverage
or physical hardware validation.

```cpp
#include <Adafruit_BME280.h>
#include <Arduino.h>
#include <Wire.h>

Adafruit_BME280 bme;

void setup() {
  Serial.begin(115200);
  Wire.begin(6, 7, 100000);  // SDA=GPIO6, SCL=GPIO7
  if (!bme.begin(0x77, &Wire)) {
    Serial.println("BME280 not found");
    return;
  }
  Serial.printf("BME280_ID=0x%02X\n", bme.sensorID());
  Serial.printf("Temperature C: %.2f\n", bme.readTemperature());
}

void loop() {}
```

## STM32F103 — rank 3, 17% of reported MCU use

### NUCLEO-F103RB + BME280

The NUCLEO-F103RB's STM32F103RBT6 boots a register-level smoke, reads the
BME280's `CHIP_ID` (`0x60`) over the I2C1 board route (D15/SCL = PB8,
D14/SDA = PB9), and pulses LD2 after the successful read.

- [Wiring, scope, and copy-paste Arduino sketch](../examples/nucleo-f103rb-bme280/README.md)
- [Live vendor status](../examples/nucleo-f103rb-bme280/evidence/vendor-status.md)
- [Committed local simulator evidence](../examples/nucleo-f103rb-bme280/evidence/)

```cpp
#include <Wire.h>
#include <Adafruit_BME280.h>

Adafruit_BME280 bme;

void setup() {
  Serial.begin(115200);
  Wire.begin();  // NUCLEO-F103RB D14/D15: PB9/PB8, I2C1
  if (!bme.begin(0x77, &Wire)) {
    Serial.println("BME280 not found");
    while (true) delay(10);
  }
  Serial.println("NUCLEO_F103RB_BME280_OK");
}

void loop() {
  Serial.print("Temperature C: ");
  Serial.println(bme.readTemperature());
  Serial.print("Pressure hPa: ");
  Serial.println(bme.readPressure() / 100.0F);
  Serial.print("Humidity %: ");
  Serial.println(bme.readHumidity());
  delay(2000);
}
```

### STM32F103C8T6 Blue Pill

The exact STM32F103C8T6 is still Active and in volume production on ST's live
product page. The retailer listing checked for this round showed the Blue Pill
board in stock. This is board depth for the measured rank-3 STM32F103 target,
not a new chip family. The smoke checks the reference board's active-low PC13
LED and USART1 header output. It makes no USB, debugger, clone-uniformity, or
physical-hardware claim.

- [Wiring, scope, copy-paste Arduino example, and smoke command](../examples/stm32f103-blue-pill/README.md)
- [Dated ST lifecycle and board availability receipt](../examples/stm32f103-blue-pill/vendor-status.md)
- [Committed local simulator evidence](../examples/stm32f103-blue-pill/evidence/)

```cpp
#include <Arduino.h>

void setup() {
  pinMode(PC13, OUTPUT);
  Serial1.begin(115200);  // USART1 TX=PA9, RX=PA10
  Serial1.println("BLUEPILL_F103C8_OK");
}

void loop() {
  digitalWrite(PC13, LOW);   // active-low user LED on
  delay(500);
  digitalWrite(PC13, HIGH);  // user LED off
  delay(500);
}
```

## WCH — Tier C vendor bet (not a measured survey rank)

### WCH CH32V003EVT

This exact evaluation board is currently listed in stock by two retailers. The
profile uses its CH32V003F4P6 and the board's LED connector: LED1 on P4-1 is
jumpered to PC0 and toggled as an active-low output. A separate smoke writes
through polled USART1 TX on the exposed PD5 pin. USB-C is power-only. The model
does not claim WCH-Link, USB data, PFIC, ADC, I2C, SPI, or timers.

- [Board wiring, scope, and copy-paste C example](../examples/wch-ch32v003evt/README.md)
- [Live stock and lifecycle receipt](../examples/wch-ch32v003evt/vendor-status.md)
- [Committed local simulator evidence](../examples/wch-ch32v003evt/evidence/)

```c
#include <stdint.h>

#define RCC_APB2PCENR (*(volatile uint32_t *)0x40021018u)
#define GPIOC_CFGLR   (*(volatile uint32_t *)0x40011000u)
#define GPIOC_BSHR    (*(volatile uint32_t *)0x40011010u)
#define GPIOC_BCR     (*(volatile uint32_t *)0x40011014u)

void main(void) {
  RCC_APB2PCENR |= 1u << 4;  // enable GPIOC
  GPIOC_CFGLR = 0x2u;        // PC0 push-pull output
  for (;;) {
    GPIOC_BSHR = 1u;         // LED1 off
    for (volatile uint32_t i = 0; i < 500; ++i) __asm__ volatile("nop");
    GPIOC_BCR = 1u;          // LED1 on
    for (volatile uint32_t i = 0; i < 500; ++i) __asm__ volatile("nop");
  }
}
```
