// Built by the hosted LabWired compile (PlatformIO ststm32, Arduino core
// 3.0.0) for stm32f411ceu6-blackpill, then stripped. In that core
// SPIClass::begin() goes through configSpi(), which skips spi_init() because
// the default settings equal the default-initialized member, so the SPI
// handle's Instance stays NULL and spi_transfer() polls NULL+0x08 (SR.TXE)
// forever. Silicon hangs the same way. Fixed upstream in stm32duino
// 6d6800953f ("fix(spi): init never called at begin").
#include <SPI.h>
void setup() {
  Serial.begin(115200);
  SPI.setSCLK(PA5); SPI.setMISO(PA6); SPI.setMOSI(PA7);
  SPI.begin();
  Serial.println("begun");
  uint8_t r = SPI.transfer(0xFF);
  Serial.print("done ");
  Serial.println(r, HEX);
}
void loop() {}
