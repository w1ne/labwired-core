// ESP32-C3 Display Workshop
//
// One Arduino sketch drives the workshop display set:
// - SSD1306 OLED 128x64 / 128x32 on I2C SDA=GPIO4, SCL=GPIO5
// - Nokia 5110 / PCD8544 on SPI SCK=GPIO6, MOSI=GPIO7, DC=GPIO2, CS=GPIO10, RST=GPIO3
// - TM1637 4-digit LED clock on CLK=GPIO0, DIO=GPIO1
// - 2.9" tri-color e-paper on SPI SCK=GPIO6, MOSI=GPIO7, DC=GPIO2, CS=GPIO10, RST=GPIO3, BUSY=GPIO4

#include <Arduino.h>
#include <SPI.h>
#include <Wire.h>

static constexpr int PIN_TM_CLK = 0;
static constexpr int PIN_TM_DIO = 1;
static constexpr int PIN_DC = 2;
static constexpr int PIN_RST = 3;
static constexpr int PIN_BUSY = 4;
static constexpr int PIN_I2C_SDA = 4;
static constexpr int PIN_I2C_SCL = 5;
static constexpr int PIN_SPI_SCK = 6;
static constexpr int PIN_SPI_MOSI = 7;
static constexpr int PIN_SPI_MISO_UNUSED = 8;
static constexpr int PIN_CS = 10;

#ifndef WORKSHOP_OLED_HEIGHT
#define WORKSHOP_OLED_HEIGHT 64
#endif

#define WORKSHOP_TARGET_ALL 0
#define WORKSHOP_TARGET_OLED 1
#define WORKSHOP_TARGET_NOKIA5110 2
#define WORKSHOP_TARGET_TM1637 3
#define WORKSHOP_TARGET_EPAPER 4

#ifndef WORKSHOP_DISPLAY_TARGET
#define WORKSHOP_DISPLAY_TARGET WORKSHOP_TARGET_ALL
#endif

static constexpr uint8_t OLED_HEIGHT = WORKSHOP_OLED_HEIGHT;
static constexpr uint8_t OLED_PAGES = OLED_HEIGHT / 8;
static_assert(OLED_HEIGHT == 32 || OLED_HEIGHT == 64, "WORKSHOP_OLED_HEIGHT must be 32 or 64");
static_assert(
  WORKSHOP_DISPLAY_TARGET == WORKSHOP_TARGET_ALL ||
  WORKSHOP_DISPLAY_TARGET == WORKSHOP_TARGET_OLED ||
  WORKSHOP_DISPLAY_TARGET == WORKSHOP_TARGET_NOKIA5110 ||
  WORKSHOP_DISPLAY_TARGET == WORKSHOP_TARGET_TM1637 ||
  WORKSHOP_DISPLAY_TARGET == WORKSHOP_TARGET_EPAPER,
  "WORKSHOP_DISPLAY_TARGET must be one of the WORKSHOP_TARGET_* constants"
);

static constexpr bool USE_OLED =
  WORKSHOP_DISPLAY_TARGET == WORKSHOP_TARGET_ALL ||
  WORKSHOP_DISPLAY_TARGET == WORKSHOP_TARGET_OLED;
static constexpr bool USE_NOKIA =
  WORKSHOP_DISPLAY_TARGET == WORKSHOP_TARGET_ALL ||
  WORKSHOP_DISPLAY_TARGET == WORKSHOP_TARGET_NOKIA5110;
static constexpr bool USE_TM1637 =
  WORKSHOP_DISPLAY_TARGET == WORKSHOP_TARGET_ALL ||
  WORKSHOP_DISPLAY_TARGET == WORKSHOP_TARGET_TM1637;
static constexpr bool USE_EPAPER =
  WORKSHOP_DISPLAY_TARGET == WORKSHOP_TARGET_ALL ||
  WORKSHOP_DISPLAY_TARGET == WORKSHOP_TARGET_EPAPER;
static constexpr bool USE_SPI = USE_NOKIA || USE_EPAPER;

static uint8_t oled[128 * OLED_PAGES];
static uint8_t nokia[84 * 6];
static uint32_t lastSecond = 0;
static bool epaperDone = false;

static const uint8_t SEG_DIGITS[10] = {
  0x3F, 0x06, 0x5B, 0x4F, 0x66, 0x6D, 0x7D, 0x07, 0x7F, 0x6F
};

static const uint8_t SEG_BITS[10][7] = {
  {1,1,1,1,1,1,0}, {0,1,1,0,0,0,0}, {1,1,0,1,1,0,1}, {1,1,1,1,0,0,1},
  {0,1,1,0,0,1,1}, {1,0,1,1,0,1,1}, {1,0,1,1,1,1,1}, {1,1,1,0,0,0,0},
  {1,1,1,1,1,1,1}, {1,1,1,1,0,1,1}
};

static void tmDelay() { delayMicroseconds(5); }
static void tmClk(bool high) { digitalWrite(PIN_TM_CLK, high ? HIGH : LOW); tmDelay(); }
static void tmDio(bool high) { digitalWrite(PIN_TM_DIO, high ? HIGH : LOW); tmDelay(); }
static void tmStart() { tmDio(HIGH); tmClk(HIGH); tmDio(LOW); tmClk(LOW); }
static void tmStop() { tmClk(LOW); tmDio(LOW); tmClk(HIGH); tmDio(HIGH); }
static void tmWrite(uint8_t b) {
  for (int i = 0; i < 8; i++) {
    tmClk(LOW);
    tmDio((b >> i) & 1);
    tmClk(HIGH);
  }
  tmClk(LOW);
  tmDio(HIGH);
  tmClk(HIGH);
  tmClk(LOW);
}

static void tmDisplay(uint8_t h, uint8_t m, bool colon) {
  uint8_t data[4] = {
    SEG_DIGITS[(h / 10) % 10],
    static_cast<uint8_t>(SEG_DIGITS[h % 10] | (colon ? 0x80 : 0)),
    SEG_DIGITS[(m / 10) % 10],
    SEG_DIGITS[m % 10],
  };
  tmStart(); tmWrite(0x40); tmStop();
  tmStart(); tmWrite(0xC0);
  for (uint8_t b : data) tmWrite(b);
  tmStop();
  tmStart(); tmWrite(0x8F); tmStop();
}

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

static void drawDigit(uint8_t *buf, int w, int h, int x, int y, int s, int digit) {
  const uint8_t *seg = SEG_BITS[digit % 10];
  int sw = 4 * s, vh = 5 * s;
  if (seg[0]) hLine(buf, w, h, x + s, y, sw);
  if (seg[1]) vLine(buf, w, h, x + sw + s, y + s, vh);
  if (seg[2]) vLine(buf, w, h, x + sw + s, y + vh + 2 * s, vh);
  if (seg[3]) hLine(buf, w, h, x + s, y + 2 * vh + 2 * s, sw);
  if (seg[4]) vLine(buf, w, h, x, y + vh + 2 * s, vh);
  if (seg[5]) vLine(buf, w, h, x, y + s, vh);
  if (seg[6]) hLine(buf, w, h, x + s, y + vh + s, sw);
}

static void drawClock(uint8_t *buf, int w, int h, int hh, int mm) {
  memset(buf, 0, (w * h) / 8);
  int s = (w >= 100) ? 3 : 2;
  int y = (h >= 48) ? 8 : 4;
  int x = (w >= 100) ? 7 : 3;
  int step = 6 * s + 4;
  drawDigit(buf, w, h, x, y, s, hh / 10);
  drawDigit(buf, w, h, x + step, y, s, hh % 10);
  setPixel(buf, w, h, x + 2 * step - 1, y + 5 * s);
  setPixel(buf, w, h, x + 2 * step - 1, y + 8 * s);
  drawDigit(buf, w, h, x + 2 * step + 3, y, s, mm / 10);
  drawDigit(buf, w, h, x + 3 * step + 3, y, s, mm % 10);
}

static void oledFlush() {
  for (uint8_t page = 0; page < OLED_PAGES; page++) {
    oledCmd(0xB0 | page);
    oledCmd(0x00);
    oledCmd(0x10);
    oledData(&oled[page * 128], 128);
  }
}

static void spiSelect(bool active) {
  digitalWrite(PIN_CS, active ? LOW : HIGH);
}

static void spiCommand(uint8_t c) {
  digitalWrite(PIN_DC, LOW);
  spiSelect(true);
  SPI.transfer(c);
  spiSelect(false);
}

static void spiData(uint8_t d) {
  digitalWrite(PIN_DC, HIGH);
  spiSelect(true);
  SPI.transfer(d);
  spiSelect(false);
}

static void spiPanelReset() {
  digitalWrite(PIN_RST, LOW);
  delay(5);
  digitalWrite(PIN_RST, HIGH);
}

static void nokiaInit() {
  spiPanelReset();
  spiCommand(0x21);
  spiCommand(0xB8);
  spiCommand(0x14);
  spiCommand(0x20);
  spiCommand(0x0C);
}

static void nokiaFlush() {
  spiCommand(0x40);
  spiCommand(0x80);
  digitalWrite(PIN_DC, HIGH);
  spiSelect(true);
  for (uint8_t b : nokia) SPI.transfer(b);
  spiSelect(false);
}

static void epaperPaintOnce() {
  if (epaperDone) return;
  epaperDone = true;
  spiCommand(0x12);
  delay(5);
  spiCommand(0x24);
  digitalWrite(PIN_DC, HIGH);
  spiSelect(true);
  for (int i = 0; i < 4736; i++) SPI.transfer((i % 17) < 8 ? 0x00 : 0xFF);
  spiSelect(false);
  spiCommand(0x26);
  digitalWrite(PIN_DC, HIGH);
  spiSelect(true);
  for (int i = 0; i < 4736; i++) SPI.transfer(0xFF);
  spiSelect(false);
  spiCommand(0x22);
  spiData(0xF7);
  spiCommand(0x20);
}

static void drawSpiDisplays(uint8_t hh, uint8_t mm, bool includeEpaper) {
  drawClock(nokia, 84, 48, hh, mm);
  nokiaFlush();
  if (includeEpaper) {
    epaperPaintOnce();
    nokiaFlush();
  }
}

static void drawI2cDisplays(uint8_t hh, uint8_t mm) {
  drawClock(oled, 128, OLED_HEIGHT, hh, mm);
  oledFlush();
}

static void drawAllDisplays(uint8_t hh, uint8_t mm, bool colon) {
  if (USE_TM1637) tmDisplay(hh, mm, colon);
  if (USE_NOKIA) drawSpiDisplays(hh, mm, false);
  if (USE_OLED) drawI2cDisplays(hh, mm);
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

void setup() {
  workshopSerialBegin();
  workshopSerialPrintln("ESP32-C3 Display Workshop");
  if (USE_TM1637) {
    pinMode(PIN_TM_CLK, OUTPUT);
    pinMode(PIN_TM_DIO, OUTPUT);
    tmDisplay(12, 34, true);
  }
  if (USE_OLED) {
    oledInit();
    drawI2cDisplays(12, 34);
  }
  if (USE_SPI) {
    pinMode(PIN_DC, OUTPUT);
    pinMode(PIN_RST, OUTPUT);
    pinMode(PIN_CS, OUTPUT);
    if (USE_EPAPER) pinMode(PIN_BUSY, INPUT);
    digitalWrite(PIN_CS, HIGH);
    SPI.begin(PIN_SPI_SCK, PIN_SPI_MISO_UNUSED, PIN_SPI_MOSI, PIN_CS);
    SPI.beginTransaction(SPISettings(2000000, MSBFIRST, SPI_MODE0));
    if (USE_NOKIA) {
      nokiaInit();
      drawSpiDisplays(12, 34, false);
    }
    if (USE_EPAPER) {
      spiPanelReset();
      epaperPaintOnce();
    }
  }
  workshopSerialPrintln("WORKSHOP_TICK 00:00");
  lastSecond = millis() / 1000;
}

void loop() {
  uint32_t seconds = millis() / 1000;
  if (seconds == lastSecond) return;
  lastSecond = seconds;
  uint8_t hh = (seconds / 3600) % 24;
  uint8_t mm = (seconds / 60) % 60;
  drawAllDisplays(hh, mm, seconds & 1);
  char line[24];
  snprintf(line, sizeof(line), "WORKSHOP_TICK %02u:%02u", hh, mm);
  workshopSerialPrintln(line);
}
