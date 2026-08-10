// Acceptance sketch for USB_SERIAL_JTAG interrupt modelling.
// Built with -DARDUINO_USB_CDC_ON_BOOT=1 -DARDUINO_USB_MODE=1 so `Serial`
// is HWCDC (interrupt-driven), NOT HardwareSerial/UART0.
#include <Arduino.h>

static uint32_t n = 0;

void setup() {
  Serial.begin(115200);
  Serial.println("LW_CDC_SETUP");
}

void loop() {
  Serial.print("LW_CDC_LOOP ");
  Serial.println(n++);
  delay(50);
}
