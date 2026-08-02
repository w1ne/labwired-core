// LabWired E-Reader — tiny GxEPD2 demo for ESP32-WROOM-32 + Waveshare 2.9"
// tri-color panel. The same firmware this sketch builds:
//   * runs unmodified in the LabWired playground (deterministic Xtensa sim),
//   * flashes to physical ESP32-WROOM-32 hardware via espflash,
//   * runs in GitHub Actions CI via labwired-cli for regression gating.
//
// Pin map (Arduino-ESP32-compatible Waveshare default):
//   GPIO5  CS
//   GPIO17 DC
//   GPIO16 RST
//   GPIO4  BUSY
//   GPIO18 SCK
//   GPIO23 MOSI
//   GPIO32 NEXT button (to GND, internal pull-up)
//   GPIO33 PREV button (to GND, internal pull-up)

#include <GxEPD2_3C.h>
#include <Fonts/FreeSerifBold12pt7b.h>
#include <Fonts/FreeSerif9pt7b.h>

// Waveshare 2.9" tri-color (C90c) — matches what an Arduino-ESP32 reference firmware
// on this same physical hardware uses (verified in
// the GxEPD2 library examples). Wrong driver class
// = panel refreshes without errors but shows blank (which is what we
// saw with Z13c on the first flash attempt).
GxEPD2_3C<GxEPD2_290_C90c, GxEPD2_290_C90c::HEIGHT> display(
    GxEPD2_290_C90c(/*CS=*/5, /*DC=*/17, /*RST=*/16, /*BUSY=*/4));

// Page buttons. Wired straight to GND and held high by the ESP32's internal
// pull-ups, so a press reads LOW — no external resistors on the bench.
constexpr uint8_t PIN_NEXT = 32;
constexpr uint8_t PIN_PREV = 33;

// A tri-color full refresh takes roughly a second of real panel time, which is
// far longer than any contact bounce, but the sketch still debounces so a noisy
// press can't queue up a second redraw.
constexpr unsigned long DEBOUNCE_MS = 40;

struct Page {
  const char *title;
  const char *body[4];
};

// Body lines are pre-broken: GxEPD2 has no word wrap, and hard-coding the breaks
// keeps every page inside the 296x128 panel without measuring text at runtime.
const Page PAGES[] = {
    {"LabWired Reader",
     {"The simulator IS the",
      "hardware. The same",
      "firmware runs in your",
      "browser and on the bench."}},
    {"Deterministic",
     {"Every run replays the",
      "same cycles, so a test",
      "that passes here passes",
      "on real silicon too."}},
    {"Press the buttons",
     {"NEXT and PREV redraw",
      "the panel from firmware,",
      "the same way a real",
      "reader turns its pages."}},
};

constexpr uint8_t PAGE_COUNT = sizeof(PAGES) / sizeof(PAGES[0]);

uint8_t currentPage = 0;

// Forward declaration: the Arduino IDE auto-generates prototypes, but tools that
// compile the .ino straight as a .cpp (e.g. the proto.cat compile service) don't
// — so without this, setup()'s call to drawPage() fails to compile.
void drawPage();

void setup() {
  Serial.begin(115200);
  delay(200);
  Serial.println();
  Serial.println("[reader] setup() entered");
  Serial.println("[reader] pin map: CS=5 DC=17 RST=16 BUSY=4 SCK=18 MOSI=23");
  Serial.println("[reader] buttons: NEXT=32 PREV=33 (active low)");
  pinMode(PIN_NEXT, INPUT_PULLUP);
  pinMode(PIN_PREV, INPUT_PULLUP);
  Serial.print("[reader] BUSY initial state: ");
  pinMode(4, INPUT);
  Serial.println(digitalRead(4) ? "HIGH (panel busy or floating)" : "LOW (idle)");
  Serial.println("[reader] calling display.init(115200) — will hang here if BUSY stays HIGH");
  display.init(115200);  // full GxEPD2 debug output now
  Serial.println("[reader] display.init() returned");
  display.setRotation(1);
  Serial.println("[reader] calling drawPage()");
  drawPage();
  Serial.println("[reader] drawPage() returned");
  Serial.println("[reader] setup() complete — page should be visible");
}

void drawPage() {
  const Page &page = PAGES[currentPage];
  display.setFullWindow();
  display.firstPage();
  do {
    display.fillScreen(GxEPD_WHITE);

    // Title bar — black ink.
    display.setTextColor(GxEPD_BLACK);
    display.setFont(&FreeSerifBold12pt7b);
    display.setCursor(8, 24);
    display.print(page.title);

    // Body copy — black ink.
    display.setFont(&FreeSerif9pt7b);
    int16_t y = 50;
    for (uint8_t line = 0; line < 4; line++) {
      display.setCursor(8, y);
      display.print(page.body[line]);
      y += 16;
    }

    // Page counter — red ink, bottom-right.
    display.setTextColor(GxEPD_RED);
    display.setCursor(230, 122);
    display.print("Page ");
    display.print(currentPage + 1);
    display.print("/");
    display.print(PAGE_COUNT);
  } while (display.nextPage());
}

// Reports a single press per physical press: true only on the HIGH->LOW edge,
// so holding a button down does not flip through every page.
bool pressed(uint8_t pin, bool &wasDown, unsigned long &lastChange) {
  const bool down = digitalRead(pin) == LOW;
  const unsigned long now = millis();
  if (down == wasDown) return false;
  if (now - lastChange < DEBOUNCE_MS) return false;
  lastChange = now;
  wasDown = down;
  return down;
}

void loop() {
  static bool nextDown = false;
  static bool prevDown = false;
  static unsigned long nextChange = 0;
  static unsigned long prevChange = 0;

  int8_t step = 0;
  if (pressed(PIN_NEXT, nextDown, nextChange)) step = 1;
  else if (pressed(PIN_PREV, prevDown, prevChange)) step = -1;

  if (step != 0) {
    // Wrap in both directions so neither button ever becomes a dead end.
    currentPage = (currentPage + PAGE_COUNT + step) % PAGE_COUNT;
    Serial.print("[reader] page -> ");
    Serial.println(currentPage + 1);
    drawPage();
  }

  delay(10);
}
