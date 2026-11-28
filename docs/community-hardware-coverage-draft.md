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

### Waveshare ESP32-C6-Zero-M + BME280

ESP32 remains rank 1 (38% of reported MCU use); demand for this exact board is
not measured separately. Waveshare's current page lists the C6-Zero-M header
SKU, and Electrokit listed 232 units in stock on 2026-11-18. Their flash
specifications disagree: Waveshare lists C6FH8 / 8 MB, while Electrokit lists
C6FH4 / 4 MB. The model uses the conservative 4 MB profile. The smoke reads
BME280 ID `0x60` and raw temperature over I2C0 on GPIO6/GPIO7. The onboard RGB,
USB, radio, LP core, and reset/strap behavior are outside this profile.

- [Wiring, scope, and copy-paste Arduino example](../examples/esp32c6-zero-m-bme280/README.md)
- [Live Waveshare and Electrokit availability receipt](../examples/esp32c6-zero-m-bme280/vendor-status.md)
- [Committed local smoke evidence](../examples/esp32c6-zero-m-bme280/evidence/)

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
separately measured. Checked 2026-11-18, Espressif's current guide lists the
N4X order code with 4 MB flash and marks only the older unsuffixed order code
EOL. DigiKey lists the exact N4X board as Active and in stock. The local
bare-RISC-V smoke sends UART0 output and bit-bangs one red WS2812 frame through
GPIO8; the observer must decode one lit pixel. This adds a current, official
C3 board profile to the existing C3 chip model. It does not claim ROM boot,
USB-UART bridge, IO_MUX, or timed RMT playback.

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

### ESP8684-DevKitM-1-H4X + UART0 and red LED

ESP32 remains demand rank 1 (38% of reported MCU use); ESP32-C2 is not ranked
separately. Espressif's live guide lists the H4X order code with 4 MB flash and
marks the older H4 variant NRND. Mouser showed 35 H4X boards in stock on
2026-11-17. The board smoke sends a UART0 marker and observes GPIO0 transitions
on the onboard red LED channel. This adds a working board profile and C2 clock
pacing; it does not claim ROM boot, the USB-UART bridge, or pad electrical
routing.

- [Board scope, copy-paste ESP-IDF example, and smoke command](../examples/esp8684-devkitm-1-h4x/README.md)
- [Live Espressif lifecycle and Mouser availability receipt](../examples/esp8684-devkitm-1-h4x/evidence/vendor-status.md)
- [Committed UART0 and GPIO0 smoke evidence](../examples/esp8684-devkitm-1-h4x/evidence/)

```c
#include "driver/gpio.h"
#include "driver/uart.h"
#include "esp_rom_sys.h"
#include <stddef.h>

void app_main(void) {
    const uart_config_t uart_cfg = {
        .baud_rate = 115200,
        .data_bits = UART_DATA_8_BITS,
        .parity = UART_PARITY_DISABLE,
        .stop_bits = UART_STOP_BITS_1,
        .flow_ctrl = UART_HW_FLOWCTRL_DISABLE,
        .source_clk = UART_SCLK_DEFAULT,
    };
    uart_driver_install(UART_NUM_0, 256, 0, 0, NULL, 0);
    uart_param_config(UART_NUM_0, &uart_cfg);
    gpio_reset_pin(GPIO_NUM_0);
    gpio_set_direction(GPIO_NUM_0, GPIO_MODE_OUTPUT);
    static const char marker[] = "C2_H4X_GPIO0_UART_OK\n";
    uart_write_bytes(UART_NUM_0, marker, sizeof(marker) - 1);
    for (;;) {
        gpio_set_level(GPIO_NUM_0, 1);
        esp_rom_delay_us(250000);
        gpio_set_level(GPIO_NUM_0, 0);
        esp_rom_delay_us(250000);
    }
}
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

### Arduino Nano Every + Adafruit BME280

Arduino lists the Nano Every as orderable SKU ABX00028, and Microchip's live
page reported ATMEGA4809 as In Production on 2026-11-19. The Adafruit BME280
breakout was in stock on the same date. The ATmega4809 is not ranked
separately; the Nano Every is the in-production Arduino board in the classic
Nano form factor. This adds a TWI0 host model to the ATmega4809, so stock
`Wire` runs.

- [Wiring, scope, copy-paste sketch, and smoke instructions](../examples/arduino-nano-every-bme280/README.md)
- [Live vendor status receipt](../examples/arduino-nano-every-bme280/vendor-status.md)
- [Committed local smoke evidence and controls](../examples/arduino-nano-every-bme280/evidence/)

The sketch uses the stock Adafruit BME280 library on A4/A5 and prints
compensated readings on USB `Serial`: `T=25.00 C`, `P=1013.25 hPa`,
`RH=50.00 %` with the sensor model at its defaults. Timer pad outputs and
the TWI0 pads are not modelled. No physical board test was run.

```cpp
#include <Arduino.h>
#include <Wire.h>
#include <Adafruit_BME280.h>

Adafruit_BME280 bme;

void setup() {
  pinMode(LED_BUILTIN, OUTPUT);
  Serial.begin(115200);
  if (!bme.begin(0x77)) {
    Serial.println("BME280 not found at 0x77");
    return;
  }
  Serial.print("NANO_EVERY_BME280 ID=0x");
  Serial.println(bme.sensorID(), HEX);
  Serial.print("T=");
  Serial.print(bme.readTemperature(), 2);
  Serial.println(" C");
  Serial.print("P=");
  Serial.print(bme.readPressure() / 100.0F, 2);
  Serial.println(" hPa");
  Serial.print("RH=");
  Serial.print(bme.readHumidity(), 2);
  Serial.println(" %");
  digitalWrite(LED_BUILTIN, HIGH);
}

void loop() {}
```

### Arduino Nano 33 IoT + Adafruit BME280

Arduino lists the Nano 33 IoT as orderable SKU ABX00027, and Microchip's live
page reported ATSAMD21G18 as In Production on 2026-11-19. The ATSAMD21G18A is
not ranked separately; the board is on the Arduino kits line (37%). This adds
a USB device model to the ATSAMD21G18A with a scripted host, so a stock
sketch whose `Serial` is the native USB port prints.

- [Wiring, scope, copy-paste sketch, and smoke instructions](../examples/arduino-nano-33-iot-bme280/README.md)
- [Live vendor status receipt](../examples/arduino-nano-33-iot-bme280/vendor-status.md)
- [Committed local smoke evidence and controls](../examples/arduino-nano-33-iot-bme280/evidence/)

The sketch waits for the terminal (`while (!Serial)`), reads the sensor with
the stock Adafruit BME280 library on A4/A5 and prints compensated readings on
USB `Serial`: `T=25.00 C`, `P=1013.25 hPa`, `RH=50.00 %` with the sensor
model at its defaults. The on-board Wi-Fi module, crypto chip and IMU are not
attached. No physical board test was run.

```cpp
#include <Arduino.h>
#include <Wire.h>
#include <Adafruit_BME280.h>

Adafruit_BME280 bme;

void setup() {
  pinMode(LED_BUILTIN, OUTPUT);
  Serial.begin(115200);
  // Native USB: wait until a terminal opens the port (DTR), or the first
  // lines are lost.
  while (!Serial) {
    delay(1);
  }
  if (!bme.begin(0x77)) {
    Serial.println("BME280 not found at 0x77");
    return;
  }
  Serial.print("NANO_33_IOT_BME280 ID=0x");
  Serial.println(bme.sensorID(), HEX);
  Serial.print("T=");
  Serial.print(bme.readTemperature(), 2);
  Serial.println(" C");
  Serial.print("P=");
  Serial.print(bme.readPressure() / 100.0F, 2);
  Serial.println(" hPa");
  Serial.print("RH=");
  Serial.print(bme.readHumidity(), 2);
  Serial.println(" %");
  digitalWrite(LED_BUILTIN, HIGH);
}

void loop() {}
```

### Arduino Nano 33 BLE Rev2 + Adafruit BME280

Arduino lists the Nano 33 BLE Rev2 as SKU ABX00071; on 2026-11-20 its own
store showed it sold out and one authorized distributor listed 100 in stock.
The nRF52840 is not ranked separately; the board is on the Arduino kits line
(37%). This adds a USB device model to the nRF52840 with a scripted host, and
the TWI master that Mbed OS uses for `Wire`, so a stock ArduinoCore-mbed
sketch whose `Serial` is the native USB port prints.

- [Wiring, scope, copy-paste sketch, and smoke instructions](../examples/arduino-nano-33-ble-bme280/README.md)
- [Live vendor status receipt](../examples/arduino-nano-33-ble-bme280/vendor-status.md)
- [Committed local smoke evidence and controls](../examples/arduino-nano-33-ble-bme280/evidence/)

The sketch waits for the terminal (`while (!Serial)`), reads the sensor with
the stock Adafruit BME280 library on A4/A5 and prints compensated readings on
USB `Serial`: `T=25.00 C`, `P=1013.25 hPa`, `RH=50.00 %` with the sensor
model at its defaults. The board's bootloader is not modelled; the on-board
IMU and the BLE stack are not attached or exercised. No physical board test
was run.

```cpp
#include <Arduino.h>
#include <Wire.h>
#include <Adafruit_BME280.h>

Adafruit_BME280 bme;

void setup() {
  pinMode(LED_BUILTIN, OUTPUT);
  Serial.begin(115200);
  // Native USB: wait until a terminal opens the port (DTR), or the first
  // lines are lost.
  while (!Serial) {
    delay(1);
  }
  if (!bme.begin(0x77)) {
    Serial.println("BME280 not found at 0x77");
    return;
  }
  Serial.print("NANO_33_BLE_BME280 ID=0x");
  Serial.println(bme.sensorID(), HEX);
  Serial.print("T=");
  Serial.print(bme.readTemperature(), 2);
  Serial.println(" C");
  Serial.print("P=");
  Serial.print(bme.readPressure() / 100.0F, 2);
  Serial.println(" hPa");
  Serial.print("RH=");
  Serial.print(bme.readHumidity(), 2);
  Serial.println(" %");
  digitalWrite(LED_BUILTIN, HIGH);
}

void loop() {}
```

### Arduino Nano Every + Adafruit MAX31855

Same board and the same live checks on 2026-11-19: Nano Every SKU ABX00028
orderable, ATMEGA4809 In Production. The Adafruit MAX31855 breakout was in
stock on that date; the MAX31855 lifecycle page at Analog Devices was not
readable. This adds an SPI0 host model to the ATmega4809, so stock `SPI`
runs.

- [Wiring, scope, copy-paste sketch, and smoke instructions](../examples/arduino-nano-every-max31855/README.md)
- [Live vendor status receipt](../examples/arduino-nano-every-max31855/vendor-status.md)
- [Committed local smoke evidence and controls](../examples/arduino-nano-every-max31855/evidence/)

The sketch uses the stock Adafruit MAX31855 library on D13 / D12 with CS on
D10 and prints both junction temperatures on USB `Serial`: `TC=25.00 C`,
`CJ=22.0000 C` with the converter model at its defaults. SPI0 Buffer mode
and client mode are refused; the SPI clock phase and polarity and timer pad
outputs are not modelled. No physical board test was run.

```cpp
#include <Arduino.h>
#include <SPI.h>
#include <Adafruit_MAX31855.h>

static const uint8_t kChipSelect = 10;

Adafruit_MAX31855 thermocouple(kChipSelect);

void setup() {
  Serial.begin(115200);
  thermocouple.begin();
  Serial.println("NANO_EVERY_MAX31855");
  double hot = thermocouple.readCelsius();
  if (isnan(hot)) {
    Serial.print("MAX31855 fault 0x");
    Serial.println(thermocouple.readError(), HEX);
    return;
  }
  Serial.print("TC=");
  Serial.print(hot, 2);
  Serial.println(" C");
  Serial.print("CJ=");
  Serial.print(thermocouple.readInternal(), 4);
  Serial.println(" C");
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

### Raspberry Pi Pico W + BME280

RP2040 is survey rank 5 at 12% of reported MCU use and belongs to target-list
group #2. Raspberry Pi's current Pico W product brief keeps the board in
production through January 2036; Mouser listed exact board SKU SC0918 in stock.
The profile reads the BME280 chip ID `0x60` on I2C0 GP4/GP5 and reports the
result over UART0 GP0. Wi-Fi, Bluetooth, USB, and the CYW43439-driven onboard
LED remain outside the profile.

- [Wiring, scope, copy-paste Arduino-Pico example, and smoke command](../examples/pico-w-bme280/README.md)
- [Live Raspberry Pi, Mouser, and Adafruit status receipt](../examples/pico-w-bme280/vendor-status.md)
- [Committed local simulator evidence](../examples/pico-w-bme280/evidence/run/)

```cpp
#include <Arduino.h>
#include <Wire.h>

constexpr uint8_t BME280_ADDRESS = 0x77;

void setup() {
  Serial1.setTX(0);
  Serial1.setRX(1);
  Serial1.begin(115200);  // UART0 TX on GP0
  Wire.setSDA(4);         // I2C0 SDA on GP4
  Wire.setSCL(5);         // I2C0 SCL on GP5
  Wire.begin();

  Wire.beginTransmission(BME280_ADDRESS);
  Wire.write(0xD0);  // BME280 chip-ID register
  if (Wire.endTransmission(false) == 0 &&
      Wire.requestFrom(BME280_ADDRESS, static_cast<uint8_t>(1)) == 1 &&
      Wire.read() == 0x60) {
    Serial1.println("PICO_W_BME280_ID=0x60_I2C0_GP4_GP5");
  } else {
    Serial1.println("PICO_W_BME280_I2C_ERROR");
  }
}

void loop() {}
```

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
- [BME280 on I2C1 PB6/PB7: wiring, Arduino example, CHIP_ID smoke and evidence](../examples/stm32f103-blue-pill-bme280/README.md)
- [PWM + L298N: stock `analogWrite()` measured at 1 kHz, with a motor reaching 325.9 rpm](../examples/stm32f103-blue-pill/README.md#hardware-pwm-stock-analogwrite-driving-an-l298n)
- [PWM receipt: measured waveform, negative controls, and what is still stubbed](receipts/2026-11-25-stm32f103-bluepill-pwm/README.md)

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

### NUCLEO-H533RE + STM32H533RE + BME280

Other STM32 is MCU demand rank 4 (13%), and Nucleo/Discovery kits rank 4
(23%). ST's live eStore lists the NUCLEO-H533RE as Active and in stock. The
profile boots firmware that reads the BME280 `CHIP_ID` (`0x60`) on I2C1 over
Arduino D15/D14 (PB6/PB7), prints a USART2 success marker, and toggles LD2 on
PA5. H533RE-specific usage was not measured. The model exposes only RCC,
GPIOA/GPIOB, USART2, I2C1, and those five pins; other peripherals remain
unmapped. It is simulator evidence, with no H533RE silicon capture or physical
board result.

With I2C1 configured for PB6/PB7 (AF4), this HAL call checks the BME280 ID at
its default address:

```c
#include "main.h"  // CubeMX-generated hi2c1 declaration
#include <stdbool.h>
#include <stdint.h>

static bool bme280_connected(void) {
  uint8_t id = 0;
  return HAL_I2C_Mem_Read(&hi2c1, (uint16_t)(0x77u << 1), 0xD0u,
                          I2C_MEMADD_SIZE_8BIT, &id, 1, 100u) == HAL_OK &&
         id == 0x60u;
}
```

- [Wiring, scope, and copy-paste C example](../examples/nucleo-h533re/README.md)
- [Live ST lifecycle and availability receipt](../examples/nucleo-h533re/vendor-status.md)
- [Committed local smoke evidence](../examples/nucleo-h533re/evidence/run/)

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

CH32V203 follow-up work is a Tier C WCH capability investment, not a measured
survey pick. It is outside the MCU survey's ranked list; future demand picks
should return to the Tier A/B targets.

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

### WCH CH32V203C8T6 — USART1 RX interrupt

The exact CH32V203C8T6 was listed by LCSC as in stock on 2026-11-27; WCH's live
product page describes the CH32V203 family and links its datasheet. The current
model boots a ch32fun-based fixture, receives injected `wch!` through USART1
RXNE, vectors IRQ 53 through the QingKe PFIC, and transmits the received bytes
back through USART1 TX. The no-IRQ negative control prints a timeout marker
instead of echoing bytes. This is local simulator evidence; it is not hardware
confirmation. USART2 IRQ
54 is wired but has no firmware smoke yet. I2C, CAN and USB
remain outside the model.

- [Copy-paste C example, smoke command, and stated model scope](../examples/ch32v203c8t6/README.md#usart-rx-irq)
- [Live WCH product and LCSC availability receipt](../examples/ch32v203c8t6/evidence/vendor-status.md)
- [Committed USART1 RX IRQ smoke evidence](../examples/ch32v203c8t6/evidence/ch32fun-usart-rx-irq-smoke/)

### WCH CH32V203C8T6 — SPI2 TX on DMA1

A ch32fun-based fixture runs SPI2 as a 16-bit master fed by DMA1 channel 5 in
circular mode. The channel 5 interrupt (PFIC IRQ 31) logs half-transfer and
transfer-complete for three laps, and USART1 prints
`SPI2_DMA events=HTHTHT laps=3 cntr=8`. With the DMA channel IRQ lines removed
from the descriptor, no handler runs. This is local simulator evidence; it is
not hardware confirmation. SPI1, SPI RX and the other DMA channels have no
V203 smoke.

- [Copy-paste C example, smoke command, and stated model scope](../examples/ch32v203c8t6/README.md#spi2-tx-dma)
- [Committed SPI2 DMA smoke evidence](../examples/ch32v203c8t6/evidence/spi-dma-smoke/)

### WCH CH32V203C8T6 — TIM2 update and TIM1 compare interrupts

A ch32fun-based fixture runs TIM2 and TIM1 at 1 kHz. The TIM2 update
interrupt (PFIC IRQ 44) and the TIM1 CH1 compare interrupt (PFIC IRQ 43)
count events, and USART1 prints `TIM2_UP=11 ms=10 TIM1_CC1=11`: 10 update
periods measure 10 ms of SysTick time. With the TIM2 IRQ line removed from
the descriptor, no TIM2 handler runs. This is local simulator evidence; it is
not hardware confirmation. Timer outputs and capture inputs do not reach GPIO
pads, and TIM3/TIM4 have no V203 smoke.

- [Copy-paste C example, smoke command, and stated model scope](../examples/ch32v203c8t6/README.md#timer-interrupts)
- [Committed timer interrupt smoke evidence](../examples/ch32v203c8t6/evidence/tim-irq-smoke/)

### WCH CH32V203C8T6 — ADC1 end-of-conversion interrupt and ADC2

A ch32fun-based fixture calibrates ADC1 and ADC2 and starts single
conversions by software. With PA0 held at 1650 mV and PA1 at 825 mV, USART1
prints `ADC1_IN0=2047 VREF=1489 ADC2_IN1=1023 irqs=2`: ADC1 returns IN0 and
VREFINT through the EOC interrupt (PFIC IRQ 34), and ADC2 is polled. With the
ADC1 IRQ line removed from the descriptor, no ADC1 handler runs. This is
local simulator evidence; it is not hardware confirmation. The V20x PGA,
TKey and dual-ADC modes are not modelled.

- [Copy-paste C example, smoke command, and stated model scope](../examples/ch32v203c8t6/README.md#adc)
- [Committed ADC smoke evidence](../examples/ch32v203c8t6/evidence/adc-irq-smoke/)

### WCH CH32V203C8T6 — I2C1 polled and I2C2 interrupt-driven BME280 reads

A ch32fun-based fixture runs I2C1 (PB6/PB7) and I2C2 (PB10/PB11) as masters,
each with a BME280 model. USART1 prints
`I2C1 id=60 dig_T1=28528 T2l=43 I2C2 id=60 ev_irqs=8`: I2C1 reads the chip id
and three calibration bytes polled, and I2C2 reads the chip id through the
event interrupt (PFIC IRQ 49). With the I2C2 IRQ line removed from the
descriptor, the I2C2 read times out. This is local simulator evidence; it is
not hardware confirmation. Slave mode, the error interrupts and the I2C1
remap to PB8/PB9 are not smoke-tested.

- [Copy-paste C example, smoke command, and stated model scope](../examples/ch32v203c8t6/README.md#i2c)
- [Committed I2C smoke evidence](../examples/ch32v203c8t6/evidence/i2c-bme280-smoke/)

### WeAct BluePill Plus (CH32V203C8T6)

A buyable WeAct core board (live AliExpress listing, 2026-11-28: "+1,000
sold", €3.35) on the existing CH32V203C8T6 model. The board profile wires the
blue LED on PB2 and the user KEY on PA0, as WeAct's MIT board repository
documents. A clean-room rewrite of WeAct's own CH32V203 template prints the
WeAct banner on USART1, blinks PB2 every 250 ms, and blinks every 100 ms after
a KEY press. A checker measures the PB2 half-periods from the run:
251.06 ms before the press, 100.43 ms after it. With the press removed, the
rate stays at 250 ms and the smoke fails. The firmware stays on the 8 MHz HSI
because the model has no V20x PLL x18. USB is not modelled. This is local
simulator evidence; it is not hardware confirmation.

- [Board scope, copy-paste example, and smoke command](../examples/weact-bluepill-plus-ch32v203/README.md)
- [Live WeAct listing receipt](../examples/weact-bluepill-plus-ch32v203/vendor-status.md)
- [Committed smoke, checker and negative-control evidence](../examples/weact-bluepill-plus-ch32v203/evidence/template-smoke/)
