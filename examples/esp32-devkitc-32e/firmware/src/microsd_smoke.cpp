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
    Serial.println("ESP32_DEVKITC32E_MICROSD_READ=FAIL mount");
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

  if (count == sizeof(kExpected) - 1 &&
      memcmp(contents, kExpected, sizeof(kExpected) - 1) == 0) {
    digitalWrite(kPassPin, HIGH);
    Serial.println("ESP32_DEVKITC32E_MICROSD_READ=PASS");
  } else {
    Serial.printf("ESP32_DEVKITC32E_MICROSD_READ=FAIL bytes=%u\n",
                  static_cast<unsigned>(count));
  }
}

void loop() { delay(1000); }
