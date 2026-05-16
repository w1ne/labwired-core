# BME280 Weather Lab

STM32F103 + BME280 environmental sensor over simulated I²C.

## What it does

1. Reads the BME280 chip ID register (0xD0) — should return `0x60` (BME280).
2. Reads temperature calibration coefficients (T1/T2/T3) and prints them to UART.
3. Configures the sensor: humidity/temp/pressure oversample ×1, normal mode.
4. Loops reading raw ADC values for temperature, pressure, and humidity (registers 0xF7–0xFE),
   printing `T_raw= P_raw= H_raw=` lines to UART1.

The static simulator returns factory-calibrated values that compensate to approximately:
- Temperature: ~25 °C
- Humidity: ~50 %RH
- Pressure: ~1013 hPa

Full Bosch compensation math is in BME280 datasheet section 4.2.3.

## Building

```bash
cargo build -p bme280-weather-lab --release --target thumbv7m-none-eabi
```

## Running in LabWired playground

Select the **BME280 Weather** lab from the gallery or chip-row.
The Inspector card shows live temperature, humidity, and pressure values.
