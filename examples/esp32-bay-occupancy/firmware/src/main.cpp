// Ryan's bay-occupancy firmware.
//
//   Adafruit ESP32 Feather V2 (PID 5400, ESP32-D0WD-V3, 8MB flash / 2MB PSRAM)
//     +-- TCA9548A @ 0x70 (PID 2717)
//     |     +-- ch0..ch3 -> VCNL4010 @ 0x13 (PID 466), one per channel
//     +-- 2.4" TFT FeatherWing V2 (PID 3315), ILI9341 over SPI
//
// This is the APPLICATION, not the bring-up sketch. ryan_lab.ino proves the
// topology answers (mux present, four sensors enumerate, panel alive). This
// file implements the behaviour Ryan's tests 4,5,6,10,11,12 actually ask for:
// thresholds, hysteresis, debouncing, per-bay display, fault reporting, and a
// poll loop that never blocks the display loop.
//
// Four VCNL4010s cannot share a bus: 0x13 is fixed in silicon with no strap
// pins. Every sensor access therefore goes through selectChannel() first, and
// selectChannel() is the ONLY place the mux is written.

#include <Arduino.h>
#include <Wire.h>
#include <Adafruit_VCNL4010.h>
#include <Adafruit_GFX.h>
#include <Adafruit_ILI9341.h>

#define MUX_ADDR 0x70
#define VCNL_ADDR 0x13
#define BAY_COUNT 4

// TFT FeatherWing pin assignment for an ESP32 Feather (Adafruit's own guide).
#define TFT_CS 15
#define TFT_DC 33

// Compile-time proof the toolchain resolved the Feather V2 variant and not
// some other classic-ESP32 board. A DevKit puts I2C on 21/22, not 22/20, and
// has no STEMMA QT power switch at all.
//
// Guarded on the variant so the same source also builds for a bare DevKit.
// That build is not decoration: a DevKit has no mux and no sensors, so it is
// how tests 9 and 11 (missing sensors, fault state) get checked against real
// silicon rather than only against the twin.
#if defined(ARDUINO_ADAFRUIT_FEATHER_ESP32_V2)
static_assert(SDA == 22, "expected Feather V2 SDA on GPIO22");
static_assert(SCL == 20, "expected Feather V2 SCL on GPIO20");
static_assert(NEOPIXEL_I2C_POWER == 2, "expected STEMMA QT power switch on GPIO2");
#endif

// ── Test 4 + 5: configurable thresholds with hysteresis ───────────────────
//
// Two thresholds, not one. A single threshold makes a bay sitting exactly at
// the boundary chatter between states on sensor noise alone. Entry is higher
// than exit, so a count in the DEAD BAND [EXIT, ENTRY) changes nothing and
// the bay holds whatever state it already had. This is the property test 5
// checks, and it is only observable because the two numbers differ.
static uint16_t g_entryThreshold = 2200;  // >= this -> becomes PRESENT
static uint16_t g_exitThreshold = 1800;   // <  this -> becomes EMPTY

// ── Test 6: debouncing ─────────────────────────────────────────────────────
//
// Crossing a threshold once is not enough. A bay must read past the threshold
// on DEBOUNCE_SAMPLES consecutive polls before its state flips. One stray
// sample — the case Ryan calls out as "noisy readings near the thresholds" —
// resets the run and changes nothing.
static const uint8_t DEBOUNCE_SAMPLES = 3;

// ── Test 12: independent cadences ──────────────────────────────────────────
//
// Polling and display run off millis() deadlines, never delay(). A blocking
// delay in either one would stall the other, which is exactly what test 12
// forbids. The periods are deliberately coprime-ish so the two never lock
// into lockstep and hide a coupling bug.
static const uint32_t POLL_INTERVAL_MS = 50;
static const uint32_t DISPLAY_INTERVAL_MS = 120;

enum BayState : uint8_t {
  BAY_EMPTY = 0,
  BAY_PRESENT = 1,
  BAY_FAULT = 2,  // test 11: sensor could not be read
};

struct Bay {
  Adafruit_VCNL4010 sensor;
  BayState state;
  uint16_t lastCount;
  uint8_t pendingRun;    // consecutive samples agreeing on a change
  BayState pendingState;  // what those samples are arguing for
  bool present;           // did begin() succeed on this channel
  uint32_t faultCount;    // consecutive failed reads
};

static Bay g_bays[BAY_COUNT];
static Adafruit_ILI9341 tft = Adafruit_ILI9341(TFT_CS, TFT_DC);

// A read is declared failed after this many consecutive bad polls, so one
// dropped transaction doesn't paint a scary FAULT the operator can't clear.
static const uint32_t FAULT_AFTER_FAILURES = 3;

// ── Mux ────────────────────────────────────────────────────────────────────

// The single place the TCA9548A control register is written. Test 8 (a read on
// one channel cannot change another's state) depends on nothing else touching
// it: if any other code path wrote the mux, channel isolation would be a
// property of luck rather than of construction.
static bool selectChannel(uint8_t channel) {
  if (channel >= BAY_COUNT) return false;
  Wire.beginTransmission(MUX_ADDR);
  Wire.write(1 << channel);
  return Wire.endTransmission() == 0;
}

// Close every channel. With all channels open a VCNL4010 on ch0 would answer
// for ch3 and the isolation tests would pass while proving nothing.
static bool closeAllChannels() {
  Wire.beginTransmission(MUX_ADDR);
  Wire.write(0x00);
  return Wire.endTransmission() == 0;
}

// ── Test 4 + 5 + 6: the state machine ──────────────────────────────────────

// Classify one raw count against the thresholds, given where the bay is now.
// Returns the state the count argues for. Inside the dead band it argues for
// the current state — that IS the hysteresis.
static BayState classify(uint16_t counts, BayState current) {
  if (counts >= g_entryThreshold) return BAY_PRESENT;
  if (counts < g_exitThreshold) return BAY_EMPTY;
  return current == BAY_FAULT ? BAY_EMPTY : current;
}

// Feed one sample into a bay's debounce filter. Returns true if the bay's
// published state changed as a result.
static bool applySample(Bay &bay, uint16_t counts) {
  bay.lastCount = counts;
  BayState argued = classify(counts, bay.state);

  if (argued == bay.state) {
    // Sample agrees with the published state — any pending run dies here.
    // This is what makes a single noisy spike harmless.
    bay.pendingRun = 0;
    return false;
  }
  if (argued != bay.pendingState) {
    // The sample argues for something different from the run in progress;
    // start a fresh run rather than counting disagreeing samples together.
    bay.pendingState = argued;
    bay.pendingRun = 1;
    return false;
  }
  if (++bay.pendingRun >= DEBOUNCE_SAMPLES) {
    bay.state = argued;
    bay.pendingRun = 0;
    return true;
  }
  return false;
}

// ── Test 9 + 11: reading a bay, and failing honestly ───────────────────────

static bool readBay(uint8_t i) {
  Bay &bay = g_bays[i];
  if (!selectChannel(i)) {
    // The mux itself did not ack. Every bay behind it is unreadable.
    return false;
  }
  if (!bay.present) return false;
  uint16_t counts = bay.sensor.readProximity();
  // The Adafruit driver has no error return on readProximity(), so a dead
  // sensor reads as 0 (SDA idles high -> 0xFFFF, or NACK -> 0). Treat both
  // rails as "not a real measurement" rather than as a real bay state: a
  // disconnected sensor must show FAULT, never a confident EMPTY.
  if (counts == 0 || counts == 0xFFFF) return false;

  applySample(bay, counts);
  return true;
}

static void pollBays() {
  for (uint8_t i = 0; i < BAY_COUNT; i++) {
    Bay &bay = g_bays[i];
    if (readBay(i)) {
      bay.faultCount = 0;
    } else if (++bay.faultCount >= FAULT_AFTER_FAILURES) {
      bay.state = BAY_FAULT;
      bay.pendingRun = 0;
    }
  }
  // Leave the bus with every channel closed so an unrelated I2C user cannot
  // accidentally address a sensor through a channel we left open.
  closeAllChannels();
}

// ── Test 10 + 11: the display ──────────────────────────────────────────────

static uint16_t stateColour(BayState s) {
  switch (s) {
    case BAY_PRESENT: return ILI9341_RED;
    case BAY_EMPTY:   return ILI9341_GREEN;
    default:          return ILI9341_YELLOW;  // FAULT
  }
}

static const char *stateLabel(BayState s) {
  switch (s) {
    case BAY_PRESENT: return "PRESENT";
    case BAY_EMPTY:   return "EMPTY  ";
    default:          return "FAULT  ";
  }
}

// Repaint only the bays whose state changed. A full-screen repaint every
// interval would take long enough to starve the poll loop — the coupling
// test 12 exists to catch.
static BayState g_painted[BAY_COUNT];

static void drawBay(uint8_t i) {
  const int16_t y = 40 + i * 50;
  Bay &bay = g_bays[i];
  tft.fillRect(0, y, 240, 46, ILI9341_BLACK);
  tft.setCursor(6, y + 4);
  tft.setTextColor(stateColour(bay.state));
  tft.setTextSize(2);
  tft.print("Bay ");
  tft.print(i);
  tft.print(' ');
  tft.print(stateLabel(bay.state));
  // The raw count under each bay, so a threshold argument can be settled by
  // looking at the panel instead of by re-flashing with prints.
  tft.setCursor(6, y + 24);
  tft.setTextSize(1);
  tft.setTextColor(ILI9341_WHITE);
  if (bay.state == BAY_FAULT) {
    tft.print("no response from sensor");
  } else {
    tft.print("counts ");
    tft.print(bay.lastCount);
  }
  g_painted[i] = bay.state;
}

static void updateDisplay(bool force) {
  for (uint8_t i = 0; i < BAY_COUNT; i++) {
    if (force || g_painted[i] != g_bays[i].state) drawBay(i);
  }
}

// ── Setup ──────────────────────────────────────────────────────────────────

void setup() {
  Serial.begin(115200);

  // Feather V2 gates the STEMMA QT / I2C rail behind GPIO2. Without this the
  // mux and every sensor are simply unpowered and the whole rig reads FAULT.
#if defined(NEOPIXEL_I2C_POWER)
  pinMode(NEOPIXEL_I2C_POWER, OUTPUT);
  digitalWrite(NEOPIXEL_I2C_POWER, HIGH);
  delay(10);
#endif

  Wire.begin();
  tft.begin();
  tft.setRotation(0);
  tft.fillScreen(ILI9341_BLACK);
  tft.setCursor(6, 6);
  tft.setTextColor(ILI9341_WHITE);
  tft.setTextSize(2);
  tft.print("Bay Occupancy");

  Serial.println("bay-occupancy start");

  for (uint8_t i = 0; i < BAY_COUNT; i++) {
    Bay &bay = g_bays[i];
    bay.state = BAY_FAULT;      // nothing is EMPTY until a sensor says so
    bay.pendingState = BAY_FAULT;
    bay.pendingRun = 0;
    bay.lastCount = 0;
    bay.faultCount = 0;
    bay.present = false;

    if (!selectChannel(i)) {
      Serial.print("BAY ");
      Serial.print(i);
      Serial.println(" mux-select FAILED");
      continue;
    }
    // begin() probes the product ID; a missing sensor fails here and the bay
    // stays FAULT for the life of the run rather than reading as an empty bay.
    bay.present = bay.sensor.begin();
    Serial.print("BAY ");
    Serial.print(i);
    Serial.println(bay.present ? " ready" : " ABSENT");
    if (bay.present) bay.state = BAY_EMPTY;
  }
  closeAllChannels();

  updateDisplay(true);
  Serial.println("bay-occupancy ready");
}

// ── Test 12: the non-blocking loop ─────────────────────────────────────────

void loop() {
  static uint32_t nextPoll = 0;
  static uint32_t nextDisplay = 0;
  static uint32_t reportedAt = 0;
  const uint32_t now = millis();

  // Two independent deadlines, no delay() anywhere. Whichever is due runs;
  // neither waits on the other. If the display ever blocked the poll loop,
  // pollCount would stop advancing while displayCount kept going.
  if ((int32_t)(now - nextPoll) >= 0) {
    nextPoll = now + POLL_INTERVAL_MS;
    pollBays();
  }
  if ((int32_t)(now - nextDisplay) >= 0) {
    nextDisplay = now + DISPLAY_INTERVAL_MS;
    updateDisplay(false);
  }

  // Periodic state line: the oracle reads this instead of scraping the panel.
  if ((int32_t)(now - reportedAt) >= 500) {
    reportedAt = now;
    Serial.print("STATE");
    for (uint8_t i = 0; i < BAY_COUNT; i++) {
      Serial.print(' ');
      Serial.print(stateLabel(g_bays[i].state)[0]);  // E / P / F
      Serial.print(g_bays[i].lastCount);
    }
    Serial.println();
  }
}
