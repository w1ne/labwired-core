# NUCLEO-C031C6

ST's Nucleo-64 for the STM32C031C6 (Cortex-M0+, 48 MHz, 32 KiB flash, 12 KiB
SRAM). Stock STM32duino sketches run on `configs/chips/stm32c031c6.yaml`:
USART2 console, LD4 GPIO, Wire (I2C1), SPI (SPI1) and `analogWrite` PWM on
TIM1. `analogRead` is not modelled yet.

- Arduino matrix evidence: [evidence/arduino-matrix](evidence/arduino-matrix/README.md)
- Vendor status (read live): [evidence/vendor-status.md](evidence/vendor-status.md)

## Copy-paste Arduino example

In Arduino IDE (STM32 core) select **Nucleo-64 → Nucleo C031C6**:

```cpp
void setup() {
  Serial.begin(115200);          // ST-LINK virtual COM port (USART2)
  analogWrite(LED_BUILTIN, 128); // LD4 on PA5 = TIM1_CH1, 1 kHz 50 %
  Serial.println("hello from the C031");
}

void loop() {}
```
