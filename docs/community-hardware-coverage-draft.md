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

### ESP32-S3-DevKitC-1-N16R8 + Adafruit BME280

ESP32 remains demand rank 1 (38% of reported MCU use). Espressif lists the
N16R8 board SKU; the v1.1 guide gives 16 MB flash / 8 MB PSRAM and marks the
older N8/N8R2 variants EOL. Adafruit listed the BME280 breakout in stock on
2026-11-13. The profile uses I2C0 on GPIO8/GPIO9 and asserts the sensor ID plus
raw temperature registers. This adds board and sensor coverage over the
existing S3 model, not new silicon. The hosted Arduino oracle verifies the
copy-paste setup path; its ideal-rail assumption excludes power-on and
brown-out behavior.

- [Wiring, scope, and copy-paste Arduino sketch](../examples/esp32s3-devkitc-1-n16r8-bme280/README.md)
- [Dated Espressif and Adafruit availability receipt](../examples/esp32s3-devkitc-1-n16r8-bme280/vendor-status.md)
- [Committed local smoke logs](../examples/esp32s3-devkitc-1-n16r8-bme280/evidence/)
- [Hosted `model_verified` oracle output](../examples/esp32s3-devkitc-1-n16r8-bme280/evidence/hosted-verify.json)

```cpp
#include <Arduino.h>
#include <Wire.h>

constexpr uint8_t SDA_PIN = 8;
constexpr uint8_t SCL_PIN = 9;
constexpr uint8_t BME280_ADDR = 0x77;

bool readRegister(uint8_t reg, uint8_t &value) {
  Wire.beginTransmission(BME280_ADDR);
  Wire.write(reg);
  if (Wire.endTransmission(false) != 0 || Wire.requestFrom(BME280_ADDR, 1) != 1) {
    return false;
  }
  value = Wire.read();
  return true;
}

void setup() {
  Serial.begin(115200);
  Wire.begin(SDA_PIN, SCL_PIN, 100000);
  uint8_t id = 0;
  if (readRegister(0xD0, id) && id == 0x60) {
    Serial.printf("BME280_ID=0x%02X\n", id);
  } else {
    Serial.println("BME280_I2C_ERROR");
  }
}

void loop() {}
```

### Seeed Studio XIAO ESP32C3 + Adafruit BME280

The XIAO ESP32C3 is a USD 4.90 ESP32-C3 board; Seeed listed it in stock on
2026-11-13. The breakout sits on the board's default Wire pins, D4 (GPIO6) and
D5 (GPIO7). The simulator boots the unmodified Arduino build in full and the
sketch prints the BME280 ID, the raw temperature and the compensated
25.00 degC on the USB console. This adds board and sensor coverage over the
existing C3 model, not new silicon.

- [Wiring, scope, and copy-paste Arduino sketch](../examples/xiao-esp32c3-bme280/README.md)
- [Dated Seeed availability receipt](../examples/xiao-esp32c3-bme280/vendor-status.md)
- [Committed local smoke logs](../examples/xiao-esp32c3-bme280/evidence/)

```cpp
#include <Arduino.h>
#include <Wire.h>

constexpr uint8_t BME280_ADDR = 0x77;

void setup() {
  Serial.begin(115200);
  Wire.begin();  // XIAO ESP32C3: SDA = D4 (GPIO6), SCL = D5 (GPIO7)
  Wire.beginTransmission(BME280_ADDR);
  Wire.write(0xD0);
  uint8_t id = 0;
  if (Wire.endTransmission(false) == 0 && Wire.requestFrom(BME280_ADDR, 1) == 1) {
    id = Wire.read();
  }
  Serial.printf("BME280_ID=0x%02X\n", id);
}

void loop() {}
```

## RP2040 / RP2350 — demand group 2

### Adafruit Feather RP2350 with HSTX (no-PSRAM, Product ID 6000)

RP2040 was reported by 12% of respondents (MCU rank 5); RP2350 was not
measured separately. Adafruit's live product listing showed the no-PSRAM
Feather in stock on 2026-11-13. The local smoke toggles red D13 on GPIO7 and
captures UART0 output on the existing RP2350 model. HSTX/DVI, NeoPixel, USB,
charging, and PSRAM behavior are outside this coverage.

- [Board pin map, scope, and copy-paste Arduino example](../examples/adafruit-feather-rp2350/README.md)
- [Dated vendor availability receipt](../examples/adafruit-feather-rp2350/evidence/vendor-status.md)
- [Committed local simulator evidence](../examples/adafruit-feather-rp2350/evidence/run/)

```cpp
#include <Arduino.h>

constexpr uint8_t LED_PIN = 7;  // D13, onboard red LED

void setup() {
  pinMode(LED_PIN, OUTPUT);
  Serial1.begin(115200);  // UART0 on TX=GP0, RX=GP1
  Serial1.println("FEATHER_RP2350_GPIO7_UART_OK");
}

void loop() {
  digitalWrite(LED_PIN, HIGH);
  delay(500);
  digitalWrite(LED_PIN, LOW);
  delay(500);
}
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
