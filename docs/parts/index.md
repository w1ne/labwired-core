---
description: "Devices you attach to an MCU in Playground, system YAML, or an agent diagram: sensors, displays, motors, buttons, and more."
---

# Parts (external components)

Devices you attach to an MCU in Playground, system YAML, or an agent diagram: sensors, displays, motors, buttons, and more.

**Add a new part:** [Onboard a part (I²C · SPI · actuators)](../howto/onboard-part.md)

!!! tip "Template"
    Authors: [parts/_TEMPLATE.md](_TEMPLATE.md). Tone: [ESP32-C3 board page](../boards/esp32c3.md).

---

## Agent path

1. `labwired_list` (optionally `kind=component`)  
2. `labwired_describe` with catalog id  
3. `labwired_validate` → `labwired_run` / `labwired_verify`  

---

## Catalog (documented here)

| Part | Catalog / type id | Bus |
|------|-------------------|-----|
| [MCP9808](mcp9808.md) | `mcp9808` | I²C |
| [MMA8451Q](mma8451q.md) | `mma8451q` | I²C |
| [MPU-6050](mpu6050.md) | `mpu6050` | I²C |
| [BMP280](bmp280.md) | `bmp280` | I²C |
| [SSD1306 OLED](ssd1306.md) | `oled-ssd1306` / `oled-ssd1306-128x32` | I²C |
| [LCD1602](lcd1602.md) | `lcd1602` | GPIO / I²C backpack |
| [APA102](apa102.md) | `apa102` | SPI-like |
| [Servo](servo.md) | `servo` | GPIO PWM |
| [Button](button.md) | `button` | GPIO |
| [Buzzer](buzzer.md) | `buzzer` | GPIO |
| [Seven-segment](seven-segment.md) | `seven-segment` | GPIO / SPI |
| [BT201 Bluetooth](bt201.md) | `bt201` | UART |
| [MAX485 RS-485 transceiver](max485.md) | `max485` | UART + GPIO |
| [XY-MD02](xy-md02.md) | `xy-md02` | UART behind a MAX485 |
| [Multiplexed segment display](segment-display-mux.md) | `segment-display-mux` | GPIO |
| [Multiplexed LED matrix](led-matrix-mux.md) | `led-matrix-mux` | GPIO |
| [74HC4051](74hc4051.md) | `74hc4051` | Analog mux (GPIO select) |
| [NAU88L21 audio codec](nau88l21.md) | `nau88l21` | I²C |
| [SHT40](sht40.md) | `sht40` | I²C |
| [BME680](bme680.md) | `bme680` | I²C |
| [ICM-42688-P](icm42688.md) | `icm42688` | I²C |
| [ICM-20948](icm20948.md) | `icm20948` | I²C |
| [SGP40](sgp40.md) | `sgp40` | I²C |
| [TSL2591](tsl2591.md) | `tsl2591` | I²C |
| [DHT20](dht20.md) | `dht20` | I²C |
| [PMSA003I](pmsa003i.md) | `pmsa003i` | I²C |
| [INA226](ina226.md) | `ina226` | I²C |
| [ST7735R TFT](st7735.md) | `st7735` | SPI |
| [GC9A01 round TFT](gc9a01.md) | `gc9a01` | SPI |
| [SSD1309 OLED](ssd1309.md) | `oled-ssd1309` | I²C |
| [INA260](ina260.md) | `ina260` | I²C |
| [INA3221](ina3221.md) | `ina3221` | I²C |
| [MPU-9250](mpu9250.md) | `mpu9250` | I²C |
| [ENS160](ens160.md) | `ens160` | I²C |
| [MCP23017](mcp23017.md) | `mcp23017` | I²C |
| [PCF8575](pcf8575.md) | `pcf8575` | I²C |
| [ADS1015](ads1015.md) | `ads1015` | I²C |
| [ULN2803A](uln2803.md) | `uln2803` | GPIO |
| [RFM95W](rfm95.md) | `rfm95` | SPI |
| [SX1276](sx1276.md) | `sx1276` | SPI |
| [SX1262](sx1262.md) | `sx1262` | SPI |
| [SK6812 RGBW](sk6812.md) | `sk6812` | GPIO |
| [MCP3008](mcp3008.md) | `mcp3008` | SPI |
| [ADS1256](ads1256.md) | `ads1256` | SPI |
| [BMI160](bmi160.md) | `bmi160` | I²C |
| [BMM150](bmm150.md) | `bmm150` | I²C |
| [VEML6075](veml6075.md) | `veml6075` | I²C |
| [HT16K33](ht16k33.md) | `ht16k33` | I²C |
| [TM1638](tm1638.md) | `tm1638` | GPIO |

More devices live under [`configs/devices/`](../../configs/devices/) and in the live catalog. If a part is missing from this table, `labwired_describe` still works when the id is registered.

**Also useful:** TMP102 (`tmp102`), ADXL345 SPI (`adxl345_spi`), DC motor (`dc-motor`) — see [Onboard a part](../howto/onboard-part.md).

---

## Related

- [Onboard hardware](../howto/onboard-hardware.md)
- [I²C sensor example](../examples/i2c_sensor_example.md)
- [MCP tools](../agent/tools.md)
