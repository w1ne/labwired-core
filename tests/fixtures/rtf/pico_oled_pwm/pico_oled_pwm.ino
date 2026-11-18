// Realtime-factor workload: Raspberry Pi Pico (arduino-pico) + SSD1306 128x64
// over I2C0 (GP4/GP5, 0x3C) + PWM fade on GP15. Redraws a counter frame every
// 100 ms. Runs forever; the gate stops it on a fixed cycle budget.
#include <Wire.h>
#include <Adafruit_GFX.h>
#include <Adafruit_SSD1306.h>

Adafruit_SSD1306 display(128, 64, &Wire, -1);
unsigned long lastDraw = 0;
unsigned int frame = 0;

void setup() {
  Serial1.begin(115200);
  Wire.setSDA(4);
  Wire.setSCL(5);
  Wire.begin();
  if (!display.begin(SSD1306_SWITCHCAPVCC, 0x3C)) Serial1.println("RTF_NO_OLED");
  pinMode(15, OUTPUT);
  Serial1.println("RTF_READY pico_oled_pwm");
}

void loop() {
  unsigned long now = millis();
  if (now - lastDraw >= 100) {
    lastDraw = now;
    frame++;
    display.clearDisplay();
    display.setTextSize(2);
    display.setTextColor(SSD1306_WHITE);
    display.setCursor(0, 0);
    display.print("F ");
    display.println(frame);
    display.drawRect(0, 40, frame % 128, 10, SSD1306_WHITE);
    display.display();
    analogWrite(15, (frame * 8) & 0xFF);
  }
}
