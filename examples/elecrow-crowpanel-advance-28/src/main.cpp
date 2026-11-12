// CrowPanel Advance 2.8 V1.2 — minimal UART + ST7789 firmware probe.
// Uses the GPIO assignments in Elecrow's V1.1/V1.2 LovyanGFX driver.
// Does not require LVGL, Wi-Fi, touch, or an external display library.
#include <Arduino.h>
#include <SPI.h>

namespace {
constexpr int kSclk = 42;
constexpr int kMosi = 39;
constexpr int kCs = 40;
constexpr int kDc = 41;
constexpr int kBacklight = 38;
SPIClass lcd(FSPI);  // ESP32-S3 SPI2 host

void command(uint8_t code, const uint8_t* args = nullptr, size_t size = 0) {
  digitalWrite(kCs, LOW);
  digitalWrite(kDc, LOW);
  lcd.transfer(code);
  if (size != 0) {
    digitalWrite(kDc, HIGH);
    lcd.writeBytes(args, size);
  }
  digitalWrite(kCs, HIGH);
}

void rectangle(uint16_t x, uint16_t y, uint16_t w, uint16_t h, uint16_t rgb565) {
  // MADCTL.MV swaps the controller's 240x320 frame-memory axes.
  const uint8_t cols[] = {uint8_t(x >> 8), uint8_t(x),
                          uint8_t((x + w - 1) >> 8), uint8_t(x + w - 1)};
  const uint8_t rows[] = {uint8_t(y >> 8), uint8_t(y),
                          uint8_t((y + h - 1) >> 8), uint8_t(y + h - 1)};
  command(0x2A, cols, sizeof(cols));  // CASET
  command(0x2B, rows, sizeof(rows));  // RASET
  command(0x2C);                     // RAMWR
  uint8_t pixels[128];
  for (size_t i = 0; i < sizeof(pixels); i += 2) {
    pixels[i] = uint8_t(rgb565 >> 8);
    pixels[i + 1] = uint8_t(rgb565);
  }
  digitalWrite(kDc, HIGH);
  digitalWrite(kCs, LOW);
  for (uint16_t row = 0; row < h; ++row) {
    for (uint16_t col = 0; col < w; col += 64) {
      const uint16_t count = (w - col < 64) ? w - col : 64;
      lcd.writeBytes(pixels, count * 2);
    }
  }
  digitalWrite(kCs, HIGH);
}
}  // namespace

void setup() {
  Serial.begin(115200);
  Serial.println("CROWPANEL_UART_OK");
  pinMode(kCs, OUTPUT);
  pinMode(kDc, OUTPUT);
  pinMode(kBacklight, OUTPUT);
  digitalWrite(kCs, HIGH);
  digitalWrite(kDc, HIGH);
  digitalWrite(kBacklight, HIGH);
  lcd.begin(kSclk, -1, kMosi, kCs);
  lcd.beginTransaction(SPISettings(20000000, MSBFIRST, SPI_MODE0));
  command(0x01);  // SWRESET
  delay(10);
  command(0x11);  // SLPOUT
  delay(10);
  const uint8_t rgb565 = 0x55;
  command(0x3A, &rgb565, 1);  // COLMOD, 16-bit pixels
  const uint8_t landscape = 0x20;
  command(0x36, &landscape, 1);  // MADCTL.MV
  command(0x21);  // INVON, Elecrow's reference driver uses inversion
  command(0x29);  // DISPON
  rectangle(24, 24, 48, 48, 0xF800);   // red
  rectangle(136, 24, 48, 48, 0x07E0);  // green
  rectangle(248, 24, 48, 48, 0x001F); // blue
  lcd.endTransaction();
  Serial.println("CROWPANEL_DISPLAY_TRANSFER_OK");
}

void loop() { delay(1000); }
