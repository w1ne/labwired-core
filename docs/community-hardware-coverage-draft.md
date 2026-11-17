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

### ESP32-C5-DevKitC-1-N8R8 + BME280 + onboard RGB

ESP32 remains demand rank 1 (38% of reported MCU use); C5-specific use is
unmeasured. Espressif's current catalog lists this exact N8R8 board with a
$15 sample-price reference and distributor links. The bare-ELF smoke reads
BME280 ID `0x60` and raw temperature over I2C0 on GPIO8/GPIO9, toggles GPIO5,
and bit-bangs a red WS2812 frame on the onboard GPIO27 LED. The WS2812 observer
decodes the frame; this does not claim RMT support, mask-ROM boot, or physical
board confirmation.

- [Wiring, scope, and copy-paste Arduino example](../examples/esp32c5-devkitc-1-bme280-smoke/README.md)
- [Live Espressif board and module availability receipt](../examples/esp32c5-devkitc-1-bme280-smoke/vendor-status.md)
- [Committed smoke logs and decoded-frame evidence](../examples/esp32c5-devkitc-1-bme280-smoke/evidence/)

### ESP32-H2-DevKitM-1-N4S + onboard RGB

ESP32 is demand rank 1 (38% of reported MCU use); the survey does not score
H2 separately. Espressif's live guide lists the N4S order code as current,
marks the older N4 as NRND, and maps the onboard addressable RGB LED to GPIO8.
DigiKey currently marks the exact N4S board Active and lists it in stock. The
existing bare-RISC-V board smoke now exercises UART0 and bit-bangs a red GRB
frame through GPIO8; the NeoPixel observer must decode one lit pixel. This
adds board-peripheral coverage to the existing H2 descriptor. USB-to-UART,
IO_MUX pad routing, RMT, BLE, Thread, Zigbee, and USB remain outside the smoke.

- [Board scope, copy-paste Arduino sketch, and smoke command](../examples/esp32h2-devkitm-1-n4s/README.md)
- [Live Espressif lifecycle and DigiKey stock receipt](../examples/esp32h2-devkitm-1-n4s/evidence/vendor-status.md)
- [Committed UART, GPIO, and decoded RGB evidence](../examples/esp32h2-devkitm-1-n4s/evidence/)

```cpp
#include <Adafruit_NeoPixel.h>
#include <Arduino.h>

Adafruit_NeoPixel rgb(1, 8, NEO_GRB + NEO_KHZ800);  // onboard LED: GPIO8

void setup() {
  Serial.begin(115200);
  rgb.begin();
  rgb.setPixelColor(0, rgb.Color(255, 0, 0));
  rgb.show();
  Serial.println("H2_N4S_RGB_OK");
}

void loop() {}
```

### ESP32-C3-DevKitM-1-N4X + onboard RGB

ESP32 is demand rank 1 (38% of reported MCU use); C3-specific use is not
separately measured. Espressif's current guide lists the N4X order code with
4 MB flash and marks only the older unsuffixed order code EOL. DigiKey lists
the exact N4X board as Active and in stock. The local bare-RISC-V smoke sends
UART0 output and a red WS2812 frame through GPIO8; the observer must decode one
lit pixel. This adds a current, official C3 board profile to the existing C3
chip model. It does not claim ROM boot, USB-UART bridge, IO_MUX, or timed RMT
playback.

- [Board scope, copy-paste Arduino sketch, and smoke command](../examples/esp32c3-devkitm-1-n4x/README.md)
- [Live Espressif lifecycle and DigiKey stock receipt](../examples/esp32c3-devkitm-1-n4x/evidence/vendor-status.md)
- [Committed UART, GPIO, and decoded RGB evidence](../examples/esp32c3-devkitm-1-n4x/evidence/)

```cpp
#include <Adafruit_NeoPixel.h>
#include <Arduino.h>

Adafruit_NeoPixel rgb(1, 8, NEO_GRB + NEO_KHZ800);  // onboard LED: GPIO8

void setup() {
  Serial.begin(115200);
  rgb.begin();
  rgb.setPixelColor(0, rgb.Color(255, 0, 0));
  rgb.show();
  Serial.println("C3_N4X_RGB_OK");
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

## Arduino kit boards — rank 3, 37% of main-project kits

### Arduino UNO R4 Minima + BME280

Arduino lists the UNO R4 Minima as orderable SKU ABX00080, and Renesas lists
RA4M1 orderable parts as Active. The Adafruit BME280 STEMMA QT breakout was
in stock on 2026-11-17. This adds an IIC1 sensor path to the existing UNO R4
Minima model; RA4M1 itself is not separately ranked in the MCU-use table.

- [Wiring, scope, copy-paste sketch, and smoke instructions](../examples/arduino-uno-r4-minima-bme280/README.md)
- [Live vendor status receipt](../examples/arduino-uno-r4-minima-bme280/vendor-status.md)
- [Committed local smoke evidence](../examples/arduino-uno-r4-minima-bme280/evidence/)

The sketch reads the BME280 chip ID over `Wire`/IIC1 on A4/A5 and reports it
over `Serial1`/SCI2 on D0/D1. USB-C `Serial` uses USBFS, which remains outside
this profile.

```cpp
#include <Arduino.h>
#include <Wire.h>

constexpr uint8_t BME280_ADDRESS = 0x77;

void setup() {
  Serial1.begin(115200);  // SCI2 on D0/D1
  Wire.begin();           // IIC1 on A4/A5
  Wire.setClock(100000);

  Wire.beginTransmission(BME280_ADDRESS);
  Wire.write(0xD0);
  if (Wire.endTransmission(false) != 0 ||
      Wire.requestFrom(static_cast<int>(BME280_ADDRESS), 1) != 1) {
    Serial1.println("BME280_I2C_ERROR");
    return;
  }

  Serial1.print("BME280_ID=0x");
  Serial1.println(Wire.read(), HEX);
}

void loop() {}
```

### ESP32-C61-DevKitC-1-N8R2

ESP32 is demand rank 1 (38% of reported MCU use); the survey does not score
C61 separately. Espressif lists the exact N8R2 board, and Mouser showed 378 in
stock on 2026-11-17. The C61 profile now exercises UART0, GPIO4 output, and
I2C0 routed through GPIO6/GPIO22 to a BME280. It reads the BME280 ID (`0x60`)
and a deterministic raw-temperature vector. ROM/bootloader, electrical IO_MUX,
PSRAM, Wi-Fi, Bluetooth LE, and the onboard RGB LED remain outside scope.

- [Board scope, wiring, and copy-paste ESP-IDF example](../examples/esp32c61-devkitc-1-n8r2-bme280/README.md)
- [Live Espressif, Mouser, Bosch, and Adafruit availability receipt](../examples/esp32c61-devkitc-1-n8r2-bme280/vendor-status.md)
- [Committed UART/GPIO and BME280 smoke evidence](../examples/esp32c61-devkitc-1-n8r2-bme280/evidence/)

```c
#include <stdint.h>

#include "driver/gpio.h"
#include "driver/i2c_master.h"
#include "esp_err.h"
#include "esp_log.h"

#define BME280_ADDRESS 0x77

void app_main(void) {
    i2c_master_bus_handle_t bus;
    const i2c_master_bus_config_t bus_config = {
        .i2c_port = I2C_NUM_0,
        .sda_io_num = GPIO_NUM_6,   // J1 pin 5
        .scl_io_num = GPIO_NUM_22,  // J3 pin 6
        .clk_source = I2C_CLK_SRC_DEFAULT,
        .glitch_ignore_cnt = 7,
    };
    ESP_ERROR_CHECK(i2c_new_master_bus(&bus_config, &bus));

    i2c_master_dev_handle_t sensor;
    const i2c_device_config_t sensor_config = {
        .dev_addr_length = I2C_ADDR_BIT_LEN_7,
        .device_address = BME280_ADDRESS,
        .scl_speed_hz = 100000,
    };
    ESP_ERROR_CHECK(i2c_master_bus_add_device(bus, &sensor_config, &sensor));

    const uint8_t chip_id_register = 0xD0;
    uint8_t chip_id = 0;
    ESP_ERROR_CHECK(i2c_master_transmit_receive(
        sensor, &chip_id_register, 1, &chip_id, 1, 1000));
    if (chip_id != 0x60) {
        ESP_LOGE("bme280", "unexpected chip id: 0x%02x", chip_id);
        return;
    }
    ESP_LOGI("bme280", "BME280_ID=0x%02x", chip_id);
}
```

### Arduino Nano ESP32 + Adafruit BME280

ESP32 remains demand rank 1 (38% of reported MCU use), and Arduino kits were
used by 37% of respondents' main projects. Arduino's current store listing
offers the Nano ESP32 ABX00092; Adafruit listed the BME280 STEMMA QT breakout
in stock on 2026-11-17. The board profile wires I2C0 to the official A4/A5
route, GPIO11/GPIO12. Its local smoke reads BME280 ID `0x60` and the raw
temperature registers. The copy-paste Arduino sketch is hosted model-verified
for the BME280 ID using an explicit GPIO11/GPIO12 diagram; that result assumes
an ideal power rail and does not cover startup or brown-out behavior. The
connected hosted catalog's default A4/A5 description still says GPIO8/GPIO9,
so the example spells out the vendor-published GPIO numbers. This adds
board/peripheral depth to the existing ESP32-S3 model, not a new silicon
family.

- [Board scope, wiring, copy-paste sketch, and smoke command](../examples/arduino-nano-esp32-bme280/README.md)
- [Live Arduino and Adafruit availability receipt](../examples/arduino-nano-esp32-bme280/vendor-status.md)
- [Committed local simulator evidence](../examples/arduino-nano-esp32-bme280/evidence/)
- [Hosted `model_verified` oracle result](../examples/arduino-nano-esp32-bme280/evidence/hosted-verify.json)

```cpp
#include <Arduino.h>
#include <Wire.h>

constexpr uint8_t SDA_PIN = 11;  // A4 / GPIO11
constexpr uint8_t SCL_PIN = 12;  // A5 / GPIO12
constexpr uint8_t BME280_ADDR = 0x77;

void setup() {
  Serial.begin(115200);
  Wire.begin(SDA_PIN, SCL_PIN, 100000);
  Wire.beginTransmission(BME280_ADDR);
  Wire.write(0xD0);
  if (Wire.endTransmission(false) == 0 && Wire.requestFrom(BME280_ADDR, 1) == 1) {
    Serial.printf("NANO_ESP32_BME280_ID=0x%02X\n", Wire.read());
  } else {
    Serial.println("NANO_ESP32_BME280_I2C_ERROR");
  }
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

### Raspberry Pi Pico 2 W + BME280

The RP2040/RP2350 target group is demand group #2; RP2040 measured rank 5 at
12%, while RP2350 was not scored separately. The Pico 2 W is Raspberry Pi's
current RP2350 wireless board. Its profile runs a register-level I2C0 smoke on
GP4/GP5, checks both I/O mux selections, and requires the attached BME280 to
return chip ID 0x60. This adds board and sensor coverage to the existing RP2350
model; the CYW43439 Wi-Fi/Bluetooth path and wireless user LED remain
unmodelled.

- [Wiring, scope, copy-paste Arduino example, and smoke command](../examples/rpi-pico-2-w-bme280/README.md)
- [Live lifecycle and stock receipt](../examples/rpi-pico-2-w-bme280/vendor-status.md)
- [Committed local simulator evidence](../examples/rpi-pico-2-w-bme280/evidence/run/)

### Raspberry Pi Pico 2 + BME280

The RP2040/RP2350 target group is demand group #2; RP2040 measured rank 5 at
12%, while RP2350 was not scored separately. This profile covers the exact
non-wireless Pico 2 on I2C0 GP4/GP5, reads the attached BME280 chip ID `0x60`,
and checks both IO_BANK0 pin-function selections. It extends the existing
RP2350 model to the non-W board variant; it does not claim USB or boot-ROM
behavior. The shared RP2350 descriptor still has storage-only `powman` and
`tbman` stubs, which this smoke does not exercise.

- [Wiring, scope, copy-paste Arduino-Pico example, and smoke command](../examples/pico2-bme280/README.md)
- [Live lifecycle and stock receipt](../examples/pico2-bme280/vendor-status.md)
- [Committed local simulator evidence](../examples/pico2-bme280/evidence/run/)

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

### STM32F103C6T6A low-density MCU

STM32F103 is measured MCU rank 3 (17%). ST currently lists the exact
STM32F103C6T6A as Active and In Stock. This is exact-variant validation within
the existing STM32F103 family, not a new demand pick. The C6 profile boots a
32 KB Flash / 10 KB SRAM image, sends a USART2 marker on PA2/PA3, and toggles an
LED on PA5. The profile uses only low-density peripherals; the USB-device and
backup-register windows remain explicit stubs. ST errata ES0348 says user-mode
firmware reads DBGMCU_IDCODE as zero, which the descriptor reflects. No physical
board or silicon result is claimed.

- [Wiring, model boundary, copy-paste example, and smoke command](../examples/stm32f103c6-smoke/README.md)
- [Live ST lifecycle and stock check](../examples/stm32f103c6-smoke/vendor-status.md)
- [Committed local simulator evidence](../examples/stm32f103c6-smoke/evidence/)

```cpp
#include <Arduino.h>

void setup() {
  pinMode(PA5, OUTPUT);
  Serial2.begin(115200);  // USART2: PA2 TX, PA3 RX
  Serial2.println("STM32F103C6_LOW_DENSITY_OK");
}

void loop() {
  digitalWrite(PA5, HIGH);
  delay(500);
  digitalWrite(PA5, LOW);
  delay(500);
}
```

### STM32F103R6T6A low-density MCU

STM32F103 is measured MCU rank #3 (17%). ST lists the exact `STM32F103R6T6A`
as Active and In stock. This is exact-variant validation in the existing
STM32F103 family, not a new demand pick. The R6 LQFP64 profile runs a
register-level smoke that prints over USART2 on PA2/PA3 and toggles PA5. The
descriptor exposes the low-density SPI1, I2C1, USART1/2, TIM1-3, ADC1/2, and
DMA1 instances; USB and backup registers remain explicit untouched stubs. No
physical board or silicon test is claimed.

- [Wiring, model boundary, copy-paste example, and smoke command](../examples/stm32f103r6-smoke/README.md)
- [Live ST lifecycle and stock check](../examples/stm32f103r6-smoke/vendor-status.md)
- [Committed local simulator evidence](../examples/stm32f103r6-smoke/evidence/)

```cpp
#include <Arduino.h>

void setup() {
  pinMode(PA5, OUTPUT);
  Serial2.begin(115200);  // USART2: PA2 TX, PA3 RX
  Serial2.println("STM32F103R6_LOW_DENSITY_OK");
}

void loop() {
  digitalWrite(PA5, HIGH);
  delay(500);
  digitalWrite(PA5, LOW);
  delay(500);
}
```

## STM32 Nucleo/Discovery — kit rank 4, 23% of main projects

### NUCLEO-U5A5ZJ-Q + STM32U5A5 secure AES

Other STM32 parts were reported by 13% of respondents (MCU rank 4), and
Nucleo/Discovery kits by 23% of respondents' main projects (kit rank 4). ST
lists the NUCLEO-U5A5ZJ-Q as Active and In Stock. This board profile drives the
U5A5-specific SAES peripheral with an AES-128 ECB known-answer block from NIST
SP 800-38A; firmware compares all four returned words before printing the
success marker. The SAES model does not cover GCM/CCM, TrustZone key selection,
side-channel behavior, or hardware timing. This extends the existing U5A5 board
profile and does not add a new MCU descriptor.

- [Board overview and build/run commands](../examples/nucleo-u5a5zj/README.md)
- [Live ST lifecycle and availability receipt](../examples/nucleo-u5a5zj/vendor-status.md)
- [Firmware smoke definition](../examples/nucleo-u5a5zj/saes-smoke.yaml)
- [Committed simulator evidence](../examples/nucleo-u5a5zj/evidence/saes/)

The copy-paste firmware is
[`nucleo_u5a5_saes_smoke.rs`](../crates/firmware-stm32u575-demo/src/bin/nucleo_u5a5_saes_smoke.rs).
Build and run it from the repository root:

```sh
examples/nucleo-u5a5zj/ci/build-saes-smoke.sh
cargo run -q -p labwired-cli -- test --script examples/nucleo-u5a5zj/saes-smoke.yaml
```

Expected output: `U5A5_SAES_AES128_ECB_OK`. This is simulator-model evidence,
not physical-silicon or side-channel validation.

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
