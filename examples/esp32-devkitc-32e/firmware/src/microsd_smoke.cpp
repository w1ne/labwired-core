#include <Arduino.h>
#include <SD.h>
#include <SPI.h>

#include <stdint.h>
#include <string.h>

namespace {

constexpr int kSpiSck = 18;
constexpr int kSpiMiso = 19;
constexpr int kSpiMosi = 23;
constexpr int kCardCs = 5;
constexpr int kPassPin = 16;
constexpr int kMountedPin = 17;
constexpr char kExpected[] = "LabWired SD card demo.\r\n";
constexpr char kWriteText[] = "round-trip write passed\r\n";

}  // namespace

void setup() {
  Serial.begin(115200);
  pinMode(kCardCs, OUTPUT);
  pinMode(kPassPin, OUTPUT);
  pinMode(kMountedPin, OUTPUT);
  digitalWrite(kCardCs, HIGH);
  digitalWrite(kPassPin, LOW);
  digitalWrite(kMountedPin, LOW);

  // Classic ESP32 VSPI / SPI3 header routing.
  SPI.begin(kSpiSck, kSpiMiso, kSpiMosi, kCardCs);
  const bool mounted = SD.begin(kCardCs, SPI, 400000);
  if (!mounted) {
    Serial.println("ESP32_DEVKITC32E_MICROSD_RW=FAIL mount");
    return;
  }
  digitalWrite(kMountedPin, HIGH);

  File file = SD.open("/README.TXT", FILE_READ);
  char contents[sizeof(kExpected)] = {};
  size_t count = 0;
  if (file) {
    while (file.available() && count < sizeof(contents)) {
      contents[count++] = static_cast<char>(file.read());
    }
    file.close();
  }

  const bool initialReadOk =
      count == sizeof(kExpected) - 1 &&
      memcmp(contents, kExpected, sizeof(kExpected) - 1) == 0;

  File output = SD.open("/ROUND54.TXT", FILE_WRITE);
  size_t written = 0;
  if (output) {
    written = output.write(
        reinterpret_cast<const uint8_t *>(kWriteText), sizeof(kWriteText) - 1);
    output.close();
  }

  File input = SD.open("/ROUND54.TXT", FILE_READ);
  char roundTrip[sizeof(kWriteText)] = {};
  size_t roundTripCount = 0;
  if (input) {
    while (input.available() && roundTripCount < sizeof(roundTrip)) {
      roundTrip[roundTripCount++] = static_cast<char>(input.read());
    }
    input.close();
  }

  const bool writeReadOk = written == sizeof(kWriteText) - 1 &&
      roundTripCount == sizeof(kWriteText) - 1 &&
      memcmp(roundTrip, kWriteText, sizeof(kWriteText) - 1) == 0;
  if (initialReadOk && writeReadOk) {
    digitalWrite(kPassPin, HIGH);
    Serial.println("ESP32_DEVKITC32E_MICROSD_RW=PASS");
  } else {
    Serial.printf("ESP32_DEVKITC32E_MICROSD_RW=FAIL initial=%u written=%u readback=%u\n",
                  static_cast<unsigned>(initialReadOk),
                  static_cast<unsigned>(written),
                  static_cast<unsigned>(roundTripCount));
  }
}

void loop() { delay(1000); }
