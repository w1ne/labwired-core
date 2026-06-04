// LabWired E-Reader — Arduino-ESP32 + GxEPD2 demo for ESP32-WROOM-32 +
// Waveshare 2.9" SSD1680 tri-color e-paper.
//
// The same firmware.elf PlatformIO builds here:
//   * runs unmodified in the LabWired simulator (deterministic Xtensa core +
//     SSD1680 panel model),
//   * flashes to physical ESP32-WROOM-32 hardware,
//   * is regression-gated in CI via the LabWired CLI.
//
// Pin map (Arduino-ESP32-compatible Waveshare default):
//   GPIO5 CS · GPIO17 DC · GPIO16 RST · GPIO4 BUSY · GPIO18 SCK · GPIO23 MOSI
#include <Arduino.h>
#include <GxEPD2_3C.h>
#include <Fonts/FreeSerifBold12pt7b.h>
#include <Fonts/FreeSerif9pt7b.h>

// Waveshare 2.9" tri-color (C90c driver IC). Picking the wrong driver class
// makes the panel report success but render blank.
GxEPD2_3C<GxEPD2_290_C90c, GxEPD2_290_C90c::HEIGHT> display(
    GxEPD2_290_C90c(/*CS=*/5, /*DC=*/17, /*RST=*/16, /*BUSY=*/4));

void drawPage() {
  display.setFullWindow();
  display.firstPage();
  do {
    display.fillScreen(GxEPD_WHITE);

    // Title bar — black ink.
    display.setTextColor(GxEPD_BLACK);
    display.setFont(&FreeSerifBold12pt7b);
    display.setCursor(8, 24);
    display.print("LabWired Reader");

    // Body copy — black ink.
    display.setFont(&FreeSerif9pt7b);
    display.setCursor(8, 50);
    display.print("The simulator IS the");
    display.setCursor(8, 66);
    display.print("hardware. Same firmware");
    display.setCursor(8, 82);
    display.print("ELF runs in your browser,");
    display.setCursor(8, 98);
    display.print("on your bench, and in CI.");

    // Page counter — red ink, bottom-right.
    display.setTextColor(GxEPD_RED);
    display.setCursor(230, 122);
    display.print("Page 1");
  } while (display.nextPage());
}

void setup() {
  Serial.begin(115200);
  delay(200);
  Serial.println();
  Serial.println("[reader] setup() entered");
  Serial.println("[reader] pin map: CS=5 DC=17 RST=16 BUSY=4 SCK=18 MOSI=23");
  Serial.print("[reader] BUSY initial state: ");
  pinMode(4, INPUT);
  Serial.println(digitalRead(4) ? "HIGH (panel busy or floating)" : "LOW (idle)");
  Serial.println("[reader] calling display.init(115200)");
  display.init(115200);
  Serial.println("[reader] display.init() returned");
  display.setRotation(1);
  Serial.println("[reader] calling drawPage()");
  drawPage();
  Serial.println("[reader] drawPage() returned — hibernating");
  display.hibernate();
  Serial.println("[reader] setup() complete — page should be visible");
}

void loop() {
  // Static page — nothing to do. A real reader would advance pages here.
}
