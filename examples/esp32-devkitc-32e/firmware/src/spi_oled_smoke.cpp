#include <Arduino.h>
#include <SPI.h>

#include <stddef.h>
#include <stdint.h>

namespace {

constexpr int kSpiSck = 18;
constexpr int kSpiMiso = 19;
constexpr int kSpiMosi = 23;
constexpr int kOledCs = 5;
constexpr int kOledDc = 17;

void write_bytes(const uint8_t* bytes, size_t count) {
  for (size_t i = 0; i < count; ++i) {
    SPI.transfer(bytes[i]);
  }
}

void write_commands(const uint8_t* commands, size_t count) {
  digitalWrite(kOledDc, LOW);
  digitalWrite(kOledCs, LOW);
  write_bytes(commands, count);
  digitalWrite(kOledCs, HIGH);
}

}  // namespace

void setup() {
  Serial.begin(115200);
  pinMode(kOledCs, OUTPUT);
  pinMode(kOledDc, OUTPUT);
  digitalWrite(kOledCs, HIGH);
  digitalWrite(kOledDc, LOW);

  // Arduino's classic ESP32 SPI object is VSPI (controller SPI3).
  SPI.begin(kSpiSck, kSpiMiso, kSpiMosi, kOledCs);
  SPI.beginTransaction(SPISettings(1000000, MSBFIRST, SPI_MODE0));

  const uint8_t init[] = {
      0xAE,              // display off
      0xD5, 0x80,         // clock divide
      0xA8, 0x3F,         // multiplex ratio: 1/64
      0xD3, 0x00,         // display offset
      0x40,               // start line 0
      0x8D, 0x14,         // charge pump on
      0x20, 0x00,         // horizontal addressing mode
      0xA1, 0xC8,         // segment remap and COM scan direction
      0xDA, 0x12,         // COM pin configuration
      0x81, 0xCF,         // contrast
      0xD9, 0xF1,         // pre-charge period
      0xDB, 0x40,         // VCOMH deselect level
      0xA4, 0xA6,         // resume RAM display, normal polarity
      0xAF,               // display on
      0x21, 0x00, 0x7F,   // columns 0..127
      0x22, 0x00, 0x00,   // page 0
  };
  write_commands(init, sizeof(init));

  // Fill one complete 8-pixel page. This is visible in both the SPI device's
  // framebuffer and on the physical OLED as a solid top band.
  digitalWrite(kOledDc, HIGH);
  digitalWrite(kOledCs, LOW);
  for (unsigned i = 0; i < 128; ++i) {
    SPI.transfer(0xFF);
  }
  digitalWrite(kOledCs, HIGH);
  SPI.endTransaction();

  Serial.println("ESP32_DEVKITC32E_SPI_OLED_PAGE0=PAINTED");
}

void loop() { delay(1000); }
