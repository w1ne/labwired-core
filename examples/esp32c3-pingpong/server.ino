// Ping Pong - Player A
//
// Serves a ball to the other board over a serial wire, counts the rally, and
// draws the match on its own OLED so BOTH players show the game live.
//
// Wiring:
//   A GPIO6 (TX) ---> B GPIO7 (RX)
//   A GPIO7 (RX) <--- B GPIO6 (TX)
//   A GND        ---- B GND
//   OLED A SDA -> GPIO4, SCL -> GPIO5, VCC -> 3V3, GND -> GND
//
// Player B uses the same OLED pins on its own board (separate panel).

#include <Arduino.h>
#include <Wire.h>
#include <string.h>

static constexpr int PIN_LINK_TX = 6;
static constexpr int PIN_LINK_RX = 7;
static constexpr int PIN_I2C_SDA = 4;
static constexpr int PIN_I2C_SCL = 5;
static constexpr uint32_t LINK_BAUD = 115200;
static constexpr uint8_t OLED_ADDR = 0x3C;
static constexpr uint8_t OLED_PAGES = 8;
static constexpr int SCREEN_W = 128;
static constexpr int SCREEN_H = 64;

// If a return never comes back the rally is over. Serve a new ball instead of
// waiting forever, so pulling a wire and putting it back recovers on its own.
static constexpr uint32_t RETURN_TIMEOUT_MS = 1000;

static uint8_t fb[SCREEN_W * OLED_PAGES];
static uint32_t rally = 0;
static uint32_t longest = 0;
static uint32_t servedAt = 0;
static bool waitingForReturn = false;
static int ballX = 8;
static int ballDir = 1;

// ---------------------------------------------------------------- OLED ----

static void oledCmd(uint8_t c) {
  Wire.beginTransmission(OLED_ADDR);
  Wire.write(0x00);
  Wire.write(c);
  Wire.endTransmission();
}

static void oledFlush() {
  oledCmd(0x21); oledCmd(0); oledCmd(SCREEN_W - 1);
  oledCmd(0x22); oledCmd(0); oledCmd(OLED_PAGES - 1);
  size_t sent = 0;
  while (sent < sizeof(fb)) {
    size_t chunk = min((size_t)16, sizeof(fb) - sent);
    Wire.beginTransmission(OLED_ADDR);
    Wire.write(0x40);
    for (size_t i = 0; i < chunk; i++) Wire.write(fb[sent + i]);
    Wire.endTransmission();
    sent += chunk;
  }
}

static void oledInit() {
  static const uint8_t seq[] = {
    0xAE, 0xD5, 0x80, 0xA8, 0x3F, 0xD3, 0x00, 0x40, 0x8D, 0x14,
    0x20, 0x00, 0xA1, 0xC8, 0xDA, 0x12, 0x81, 0xCF, 0xD9, 0xF1,
    0xDB, 0x40, 0xA4, 0xA6, 0xAF,
  };
  for (uint8_t c : seq) oledCmd(c);
}

static void pset(int x, int y) {
  if (x < 0 || x >= SCREEN_W || y < 0 || y >= SCREEN_H) return;
  fb[x + (y / 8) * SCREEN_W] |= (1 << (y % 8));
}

static void fillRect(int x, int y, int w, int h) {
  for (int dy = 0; dy < h; dy++)
    for (int dx = 0; dx < w; dx++) pset(x + dx, y + dy);
}

static const uint8_t DIGITS[10][5] = {
  {0x3E,0x51,0x49,0x45,0x3E}, {0x00,0x42,0x7F,0x40,0x00},
  {0x42,0x61,0x51,0x49,0x46}, {0x21,0x41,0x45,0x4B,0x31},
  {0x18,0x14,0x12,0x7F,0x10}, {0x27,0x45,0x45,0x45,0x39},
  {0x3C,0x4A,0x49,0x49,0x30}, {0x01,0x71,0x09,0x05,0x03},
  {0x36,0x49,0x49,0x49,0x36}, {0x06,0x49,0x49,0x29,0x1E},
};

static void drawDigit(int x, int y, uint8_t d, int scale) {
  for (int col = 0; col < 5; col++) {
    uint8_t bits = DIGITS[d][col];
    for (int row = 0; row < 7; row++) {
      if (bits & (1 << row)) fillRect(x + col * scale, y + row * scale, scale, scale);
    }
  }
}

static void drawNumber(int x, int y, uint32_t n, int scale) {
  char buf[11];
  int len = 0;
  if (n == 0) buf[len++] = 0;
  while (n > 0 && len < 10) { buf[len++] = n % 10; n /= 10; }
  for (int i = 0; i < len; i++) {
    drawDigit(x + i * 6 * scale, y, buf[len - 1 - i], scale);
  }
}

static void advanceBall() {
  ballX += ballDir * 6;
  if (ballX > SCREEN_W - 12) ballDir = -1;
  if (ballX < 8) ballDir = 1;
}

static void render() {
  memset(fb, 0, sizeof(fb));
  fillRect(2, 20, 3, 20);
  fillRect(SCREEN_W - 5, 20, 3, 20);
  fillRect(ballX, 28, 4, 4);
  drawNumber(44, 4, rally, 2);
  drawNumber(96, 52, longest, 1);
  oledFlush();
}

static void serve() {
  Serial1.print("PING\n");
  servedAt = millis();
  waitingForReturn = true;
  advanceBall();
  render();
}

void setup() {
  Serial.begin(115200);
  Serial1.begin(LINK_BAUD, SERIAL_8N1, PIN_LINK_RX, PIN_LINK_TX);
  Wire.begin(PIN_I2C_SDA, PIN_I2C_SCL);
  oledInit();
  render();
  Serial.println("Player A ready - serving (OLED live)");
  serve();
}

void loop() {
  static char inbox[8];
  static uint8_t filled = 0;

  while (Serial1.available()) {
    char c = Serial1.read();
    if (c == '\n') {
      inbox[filled] = '\0';
      if (strcmp(inbox, "PONG") == 0) {
        rally++;
        if (rally > longest) longest = rally;
        Serial.print("rally ");
        Serial.println(rally);
        serve();
      }
      filled = 0;
    } else if (filled < sizeof(inbox) - 1) {
      inbox[filled++] = c;
    }
  }

  if (waitingForReturn && millis() - servedAt > RETURN_TIMEOUT_MS) {
    Serial.print("missed - rally ended at ");
    Serial.println(rally);
    rally = 0;
    serve();
  }
}
