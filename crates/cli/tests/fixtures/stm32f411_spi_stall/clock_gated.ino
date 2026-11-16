// Built by the hosted LabWired compile for stm32f411ceu6-blackpill, then
// stripped. SPI1 is initialised through beginTransaction() (which calls
// spi_init on every core version), then its APB2 clock is gated off before
// the transfer: an unclocked SPI reads SR = 0, so spi_transfer() waits on
// SPI1_SR.TXE forever.
#include <SPI.h>
void setup() {
  Serial.begin(115200);
  SPI.setSCLK(PA5); SPI.setMISO(PA6); SPI.setMOSI(PA7);
  SPI.begin();
  SPI.beginTransaction(SPISettings(1000000, MSBFIRST, SPI_MODE0));
  __HAL_RCC_SPI1_CLK_DISABLE();
  Serial.println("gated");
  uint8_t r = SPI.transfer(0xFF);
  Serial.print("done ");
  Serial.println(r, HEX);
}
void loop() {}
