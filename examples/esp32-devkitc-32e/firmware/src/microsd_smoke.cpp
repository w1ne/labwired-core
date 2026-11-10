#include <Arduino.h>
#include <SPI.h>

#include <stdint.h>

namespace {

constexpr int kSpiSck = 18;
constexpr int kSpiMiso = 19;
constexpr int kSpiMosi = 23;
constexpr int kCardCs = 5;
constexpr int kPassPin = 16;

uint8_t send_command(uint8_t command, uint32_t argument, uint8_t crc,
                     uint8_t *tail, uint8_t tail_size) {
  digitalWrite(kCardCs, LOW);
  SPI.transfer(static_cast<uint8_t>(0x40 | command));
  SPI.transfer(static_cast<uint8_t>(argument >> 24));
  SPI.transfer(static_cast<uint8_t>(argument >> 16));
  SPI.transfer(static_cast<uint8_t>(argument >> 8));
  SPI.transfer(static_cast<uint8_t>(argument));
  const uint8_t response = SPI.transfer(crc);
  for (uint8_t i = 0; i < tail_size; ++i) {
    tail[i] = SPI.transfer(0xFF);
  }
  digitalWrite(kCardCs, HIGH);
  return response;
}

}  // namespace

void setup() {
  Serial.begin(115200);
  pinMode(kCardCs, OUTPUT);
  pinMode(kPassPin, OUTPUT);
  digitalWrite(kCardCs, HIGH);
  digitalWrite(kPassPin, LOW);

  // Classic ESP32 VSPI / SPI3 header routing.
  SPI.begin(kSpiSck, kSpiMiso, kSpiMosi, kCardCs);
  SPI.beginTransaction(SPISettings(400000, MSBFIRST, SPI_MODE0));
  // Give a real card the required 74+ startup clocks before the first command.
  for (int i = 0; i < 10; ++i) SPI.transfer(0xFF);

  uint8_t r7[4] = {};
  const uint8_t idle = send_command(0, 0, 0x95, nullptr, 0);
  const uint8_t cmd8 = send_command(8, 0x000001AA, 0x87, r7, sizeof(r7));
  const uint8_t app = send_command(55, 0, 0x01, nullptr, 0);
  const uint8_t ready = send_command(41, 0x40000000, 0x01, nullptr, 0);
  SPI.endTransaction();

  if (idle == 0x01 && cmd8 == 0x01 && r7[0] == 0x00 && r7[1] == 0x00 &&
      r7[2] == 0x01 && r7[3] == 0xAA && app == 0x01 && ready == 0x00) {
    digitalWrite(kPassPin, HIGH);
    Serial.println("ESP32_DEVKITC32E_MICROSD_INIT=PASS");
  } else {
    Serial.printf("ESP32_DEVKITC32E_MICROSD_INIT=FAIL idle=%02x cmd8=%02x "
                  "r7=%02x%02x%02x%02x app=%02x ready=%02x\n",
                  idle, cmd8, r7[0], r7[1], r7[2], r7[3], app, ready);
  }
}

void loop() { delay(1000); }
