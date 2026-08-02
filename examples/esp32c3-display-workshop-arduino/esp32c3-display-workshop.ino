// ESP32-C3 Display Workshop
//
// Arduino clock sketch for the workshop OLED modules:
// - 0.96" SSD1306 OLED 128x64 on I2C SDA=GPIO4, SCL=GPIO5
// - 0.91" SSD1306 OLED 128x32 on I2C SDA=GPIO4, SCL=GPIO5

#include <Arduino.h>
#include <Wire.h>

static constexpr int PIN_I2C_SDA = 4;
static constexpr int PIN_I2C_SCL = 5;

#ifndef WORKSHOP_OLED_HEIGHT
#define WORKSHOP_OLED_HEIGHT 64
#endif

static constexpr uint8_t OLED_HEIGHT = WORKSHOP_OLED_HEIGHT;
static constexpr uint8_t OLED_PAGES = OLED_HEIGHT / 8;
static_assert(OLED_HEIGHT == 32 || OLED_HEIGHT == 64, "WORKSHOP_OLED_HEIGHT must be 32 or 64");
static constexpr uint32_t DEMO_CLOCK_START_SECONDS = 19UL * 3600UL;
static constexpr uint32_t DEMO_CLOCK_INTERVAL_MS = 1000UL;

static uint8_t oled[128 * OLED_PAGES];
static uint32_t displayedSeconds = DEMO_CLOCK_START_SECONDS;
static uint32_t lastClockMillis = 0;

static const uint8_t SEG_BITS[10][7] = {
  {1,1,1,1,1,1,0}, {0,1,1,0,0,0,0}, {1,1,0,1,1,0,1}, {1,1,1,1,0,0,1},
  {0,1,1,0,0,1,1}, {1,0,1,1,0,1,1}, {1,0,1,1,1,1,1}, {1,1,1,0,0,0,0},
  {1,1,1,1,1,1,1}, {1,1,1,1,0,1,1}
};

static void oledCmd(uint8_t c) {
  Wire.beginTransmission(0x3C);
  Wire.write(0x00);
  Wire.write(c);
  Wire.endTransmission();
}

static void oledData(const uint8_t *data, size_t n) {
  while (n) {
    size_t chunk = n > 16 ? 16 : n;
    Wire.beginTransmission(0x3C);
    Wire.write(0x40);
    for (size_t i = 0; i < chunk; i++) Wire.write(data[i]);
    Wire.endTransmission();
    data += chunk;
    n -= chunk;
  }
}

static void oledInit() {
  Wire.begin(PIN_I2C_SDA, PIN_I2C_SCL);
  delay(20);
  const uint8_t init[] = {
    0xAE, 0xD5, 0x80, 0xA8, static_cast<uint8_t>(OLED_HEIGHT - 1), 0xD3, 0x00, 0x40,
    0x8D, 0x14, 0x20, 0x00, 0xA1, 0xC8, 0xDA, static_cast<uint8_t>(OLED_HEIGHT == 64 ? 0x12 : 0x02),
    0x81, 0x8F, 0xD9, 0xF1, 0xDB, 0x40, 0xA4, 0xA6, 0xAF
  };
  for (uint8_t c : init) oledCmd(c);
}

static void setPixel(uint8_t *buf, int w, int h, int x, int y) {
  if (x < 0 || y < 0 || x >= w || y >= h) return;
  buf[(y >> 3) * w + x] |= 1 << (y & 7);
}

static void hLine(uint8_t *buf, int w, int h, int x, int y, int len) {
  for (int i = 0; i < len; i++) setPixel(buf, w, h, x + i, y);
}

static void vLine(uint8_t *buf, int w, int h, int x, int y, int len) {
  for (int i = 0; i < len; i++) setPixel(buf, w, h, x, y + i);
}

static void fillRect(uint8_t *buf, int w, int h, int x, int y, int rw, int rh) {
  for (int yy = 0; yy < rh; yy++) {
    hLine(buf, w, h, x, y + yy, rw);
  }
}

static void drawDigit(uint8_t *buf, int w, int h, int x, int y, int digitW, int digitH, int t, int digit) {
  const uint8_t *seg = SEG_BITS[digit % 10];
  const int mid = y + digitH / 2;
  const int middleY = mid - t / 2;
  const int topVertH = middleY - (y + t);
  const int bottomVertH = (y + digitH - t) - (middleY + t);
  if (seg[0]) fillRect(buf, w, h, x + t, y, digitW - 2 * t, t);
  if (seg[1]) fillRect(buf, w, h, x + digitW - t, y + t, t, topVertH);
  if (seg[2]) fillRect(buf, w, h, x + digitW - t, middleY + t, t, bottomVertH);
  if (seg[3]) fillRect(buf, w, h, x + t, y + digitH - t, digitW - 2 * t, t);
  if (seg[4]) fillRect(buf, w, h, x, middleY + t, t, bottomVertH);
  if (seg[5]) fillRect(buf, w, h, x, y + t, t, topVertH);
  if (seg[6]) fillRect(buf, w, h, x + t, middleY, digitW - 2 * t, t);
}

static void drawClock(uint8_t *buf, int w, int h, int hh, int mm, int ss) {
  memset(buf, 0, (w * h) / 8);
  const int digitW = (h >= 48) ? 14 : 10;
  const int digitH = (h >= 48) ? 32 : 20;
  const int t = 2;
  const int gap = (h >= 48) ? 3 : 2;
  const int colonGap = (h >= 48) ? 6 : 4;
  const int totalW = 6 * digitW + 5 * gap + 2 * colonGap;
  const int x = (w - totalW) / 2;
  const int y = (h - digitH) / 2;
  const int d0 = x;
  const int d1 = d0 + digitW + gap;
  const int colonX = d1 + digitW + gap;
  const int d2 = colonX + colonGap;
  const int d3 = d2 + digitW + gap;
  const int colonX2 = d3 + digitW + gap;
  const int d4 = colonX2 + colonGap;
  const int d5 = d4 + digitW + gap;
  drawDigit(buf, w, h, d0, y, digitW, digitH, t, hh / 10);
  drawDigit(buf, w, h, d1, y, digitW, digitH, t, hh % 10);
  fillRect(buf, w, h, colonX, y + digitH / 3, t, t);
  fillRect(buf, w, h, colonX, y + (2 * digitH) / 3, t, t);
  drawDigit(buf, w, h, d2, y, digitW, digitH, t, mm / 10);
  drawDigit(buf, w, h, d3, y, digitW, digitH, t, mm % 10);
  fillRect(buf, w, h, colonX2, y + digitH / 3, t, t);
  fillRect(buf, w, h, colonX2, y + (2 * digitH) / 3, t, t);
  drawDigit(buf, w, h, d4, y, digitW, digitH, t, ss / 10);
  drawDigit(buf, w, h, d5, y, digitW, digitH, t, ss % 10);
}

static void oledFlush() {
  for (uint8_t page = 0; page < OLED_PAGES; page++) {
    oledCmd(0xB0 | page);
    oledCmd(0x00);
    oledCmd(0x10);
    oledData(&oled[page * 128], 128);
  }
}

static void drawOled(uint8_t hh, uint8_t mm, uint8_t ss) {
  drawClock(oled, 128, OLED_HEIGHT, hh, mm, ss);
  oledFlush();
}

static void workshopSerialBegin() {
  Serial.begin(115200);
#if defined(ARDUINO_USB_CDC_ON_BOOT) && ARDUINO_USB_CDC_ON_BOOT
  Serial.setTxTimeoutMs(0);
  Serial0.begin(115200);
#endif
}

static void workshopSerialPrintln(const char *line) {
  Serial.println(line);
#if defined(ARDUINO_USB_CDC_ON_BOOT) && ARDUINO_USB_CDC_ON_BOOT
  Serial0.println(line);
#endif
}

static void renderClockSeconds(uint32_t displaySeconds) {
  uint8_t hh = (displaySeconds / 3600) % 24;
  uint8_t mm = (displaySeconds / 60) % 60;
  uint8_t ss = displaySeconds % 60;
  drawOled(hh, mm, ss);
  char line[32];
  snprintf(line, sizeof(line), "WORKSHOP_TICK %02u:%02u:%02u", hh, mm, ss);
  workshopSerialPrintln(line);
}

void setup() {
  workshopSerialBegin();
  workshopSerialPrintln("ESP32-C3 Display Workshop");
  oledInit();
  displayedSeconds = DEMO_CLOCK_START_SECONDS;
  renderClockSeconds(displayedSeconds);
  lastClockMillis = millis();
}

void loop() {
  const uint32_t now = millis();
  if (static_cast<uint32_t>(now - lastClockMillis) < DEMO_CLOCK_INTERVAL_MS) return;
  lastClockMillis = now;
  displayedSeconds++;
  renderClockSeconds(displayedSeconds);
}
