// Official openwch Arduino core (arduino_core_ch32) on CH32V003F4P6.
// SPIClass drives SPI1 on the variant's default pads PC5 (SCK) / PC6 (MOSI) /
// PC7 (MISO), with a GPIO chip select on PC4. The sketch clocks out the four
// dummy bytes a MAX31855 K-type thermocouple converter expects while CS is
// low, then decodes the 32-bit frame exactly as the datasheet lays it out.
//
// The core's `spi_init()` enables RCC_SPI1EN (APB2PCENR bit 12), programs
// CTLR1 (SPE | MSTR | CPOL/CPHA | baud prescaler), writes CRCR, and its
// `spi_transfer()` polls STATR.TXE, STATR.RXNE and STATR.BSY. That is a real
// full-duplex transfer through the STM32-layout bit engine, not a register
// read-back: the MAX31855's frame crosses MISO on the wire.
#include <Arduino.h>
#include <SPI.h>

static const uint8_t kChipSelect = PC4;

void setup() {
  Serial.begin(115200);
  Serial.println("CH32V003_SPI_BOOT");

  pinMode(kChipSelect, OUTPUT);
  digitalWrite(kChipSelect, HIGH);

  SPI.begin();
  SPI.beginTransaction(SPISettings(1000000, MSBFIRST, SPI_MODE0));

  digitalWrite(kChipSelect, LOW);
  uint8_t b0 = SPI.transfer(0x00);
  uint8_t b1 = SPI.transfer(0x00);
  uint8_t b2 = SPI.transfer(0x00);
  uint8_t b3 = SPI.transfer(0x00);
  digitalWrite(kChipSelect, HIGH);

  SPI.endTransaction();

  uint32_t raw = ((uint32_t)b0 << 24) | ((uint32_t)b1 << 16) |
                 ((uint32_t)b2 << 8) | (uint32_t)b3;
  // [31:18] 14-bit signed hot junction, 0.25 degC/LSB.
  // [15:4]  12-bit signed cold junction, 0.0625 degC/LSB.
  int16_t hot_q = (int16_t)(raw >> 18);
  int16_t cold_q = (int16_t)((raw >> 4) & 0x0FFF);

  Serial.print("raw=0x");
  Serial.println(raw, HEX);
  Serial.print("tc_milli=");
  Serial.println((int32_t)hot_q * 250);
  Serial.print("cj_milli=");
  Serial.println((int32_t)cold_q * 625 / 10);

  bool answered = (raw != 0u) && (raw != 0xFFFFFFFFu);
  bool fault_free = ((raw >> 16) & 1u) == 0u;
  Serial.println((answered && fault_free) ? "CH32V003_SPI_PASS"
                                          : "CH32V003_SPI_FAIL");
  Serial.println("CH32V003_SPI_DONE");
}

void loop() {}
