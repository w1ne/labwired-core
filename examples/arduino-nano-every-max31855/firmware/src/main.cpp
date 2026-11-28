#include <Arduino.h>
#include <SPI.h>
#include <Adafruit_MAX31855.h>

// Copyable ArduinoCore-megaavr example for the Nano Every with an Adafruit
// MAX31855 thermocouple breakout on the hardware SPI pins: D13 (SCK, PE2),
// D12 (MISO, PE1) and D10 (CS, PB1). SPI is SPI0 on its PE[3:0] route; Serial
// is USART3 behind the board's SAMD11 USB bridge. D13 is also LED_BUILTIN, so
// this sketch does not use the LED.
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
