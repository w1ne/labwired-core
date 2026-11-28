# NUCLEO-C031C6

ST's Nucleo-64 for the STM32C031C6 (Cortex-M0+, 48 MHz, 32 KiB flash, 12 KiB
SRAM). Stock STM32duino sketches run on `configs/chips/stm32c031c6.yaml`:
USART2 console, LD4 GPIO, Wire (I2C1), SPI (SPI1), `analogWrite` PWM on
TIM1 and `analogRead` on the G0/C0 ADC1 block.

- Arduino matrix evidence: [evidence/arduino-matrix](evidence/arduino-matrix/README.md)
- Vendor status (read live): [evidence/vendor-status.md](evidence/vendor-status.md)

## Copy-paste Arduino example

In Arduino IDE (STM32 core) select **Nucleo-64 → Nucleo C031C6**:

```cpp
void setup() {
  Serial.begin(115200);          // ST-LINK virtual COM port (USART2)
  analogWrite(LED_BUILTIN, 128); // LD4 on PA5 = TIM1_CH1, 1 kHz 50 %
  Serial.print("A0 = ");
  Serial.println(analogRead(A0)); // PA0 = ADC1_IN0
  Serial.println("hello from the C031");
}

void loop() {}
```

## Analog inputs

`analog-ntc-diode-pa0-pa1-system.yaml` drives a generic Beta-parameter NTC
divider into `board.analog.pa0_volts` (ADC1_IN0) and a Shockley diode load
line into `board.analog.pa1_volts` (ADC1_IN1). The descriptor's
`analog_pins:` map follows ST STM32_open_pin_data for the LQFP48 package
(PA0-PA8 = IN0-IN8, PA11-PA14 = IN11-IN14, PB0/PB1 = IN17/IN18). The C0
numbering differs from the G0: PB0 is IN17 here and IN8 on a G071.

Not routable yet: PB2, PB10, PB11 and PB12 (IN19-IN22). The shared `stm32g0`
ADC model converts CHSEL0..18 only, so the router refuses those pads. The
test is `crates/core/tests/analog/cosim_stm32g0b1_analog.rs`
(`cargo test -p labwired-core --test analog cosim_stm32g0b1`).
