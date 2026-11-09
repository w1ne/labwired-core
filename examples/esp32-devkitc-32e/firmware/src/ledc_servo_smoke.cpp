#include <Arduino.h>

constexpr uint8_t kServoSignalPin = 23;  // J3 pin 2 / GPIO23.
constexpr uint32_t kServoHz = 50;
constexpr uint8_t kDutyResolution = 14;
constexpr uint32_t kCenterDuty = 1187;  // 1.449 ms in a 49.92 Hz frame.

void setup() {
  Serial.begin(115200);

  if (!ledcAttach(kServoSignalPin, kServoHz, kDutyResolution)) {
    Serial.println("ESP32_DEVKITC32E_LEDC_ATTACH_FAILED");
    return;
  }
  if (!ledcWrite(kServoSignalPin, kCenterDuty)) {
    Serial.println("ESP32_DEVKITC32E_LEDC_WRITE_FAILED");
    return;
  }

  // The acceptance marker follows LEDC setup and the centered servo command.
  Serial.println("ESP32_DEVKITC32E_LEDC_SERVO_OK");
}

void loop() {}
