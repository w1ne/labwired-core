#include <Arduino.h>

static constexpr uint8_t MODE_PIN = 4;
static constexpr uint8_t SET_PIN = 5;
static constexpr uint32_t BUTTON_DEBOUNCE_MS = 30;
static constexpr uint32_t SECOND_MS = 1000;
static constexpr char WATCH_BOOT_LINE[] = "WATCH 12:34:00 RUN";

// Retained for debugger/JTAG inspection. The companion schema marker is
// `WAT1` in ASCII, identifying this packing without relying on source text.
extern "C" volatile uint32_t labwired_watch_state __attribute__((used)) = 0;
extern "C" const uint32_t labwired_watch_state_schema __attribute__((used)) = 0x57415431UL;

// Force a real volatile read so link-time garbage collection retains the schema
// symbol for a debugger as well as the mutable watch-state word.
static void retainWatchStateSchema() {
  const volatile uint32_t *schema = &labwired_watch_state_schema;
  (void)*schema;
}

static uint8_t hours = 12;
static uint8_t minutes = 34;
static uint8_t seconds = 0;
static bool setting = false;
static uint16_t state_sequence = 0;
static uint32_t last_tick_ms = 0;

struct DebouncedButton {
  uint8_t pin;
  int raw_level;
  int stable_level;
  uint32_t changed_at_ms;
};

static DebouncedButton mode_button{MODE_PIN, HIGH, HIGH, 0};
static DebouncedButton set_button{SET_PIN, HIGH, HIGH, 0};

static void initializeButton(DebouncedButton &button, uint32_t now) {
  const int level = digitalRead(button.pin);
  button.raw_level = level;
  button.stable_level = level;
  button.changed_at_ms = now;
}

// The falling edge is active-low because both watch controls use INPUT_PULLUP.
static bool fellAfterDebounce(DebouncedButton &button, uint32_t now) {
  const int sampled = digitalRead(button.pin);
  if (sampled != button.raw_level) {
    button.raw_level = sampled;
    button.changed_at_ms = now;
  }

  if (button.stable_level == button.raw_level ||
      static_cast<uint32_t>(now - button.changed_at_ms) < BUTTON_DEBOUNCE_MS) {
    return false;
  }

  const int previous = button.stable_level;
  button.stable_level = button.raw_level;
  return previous == HIGH && button.stable_level == LOW;
}

static void advanceMinute() {
  seconds = 0;
  if (++minutes == 60) {
    minutes = 0;
    hours = static_cast<uint8_t>((hours + 1) % 24);
  }
}

static void advanceSecond() {
  if (++seconds == 60) {
    seconds = 0;
    if (++minutes == 60) {
      minutes = 0;
      hours = static_cast<uint8_t>((hours + 1) % 24);
    }
  }
}

static void publishWatchState() {
  const uint32_t sequence = static_cast<uint32_t>(state_sequence++ & 0x3fffU);
  labwired_watch_state =
      (static_cast<uint32_t>(seconds) & 0x3fU) |
      ((static_cast<uint32_t>(minutes) & 0x3fU) << 6) |
      ((static_cast<uint32_t>(hours) & 0x1fU) << 12) |
      (setting ? (1UL << 17) : 0UL) |
      (sequence << 18);

  if (sequence == 0 && hours == 12 && minutes == 34 && seconds == 0 && !setting) {
    Serial.println(WATCH_BOOT_LINE);
    return;
  }

  char line[32];
  snprintf(
      line,
      sizeof(line),
      "WATCH %02u:%02u:%02u %s",
      static_cast<unsigned>(hours),
      static_cast<unsigned>(minutes),
      static_cast<unsigned>(seconds),
      setting ? "SET" : "RUN");
  Serial.println(line);
}

void setup() {
  pinMode(MODE_PIN, INPUT_PULLUP);
  pinMode(SET_PIN, INPUT_PULLUP);
  Serial.begin(115200);
  retainWatchStateSchema();

  const uint32_t now = millis();
  initializeButton(mode_button, now);
  initializeButton(set_button, now);
  last_tick_ms = now;
  publishWatchState();
}

void loop() {
  const uint32_t now = millis();
  bool changed = false;

  if (fellAfterDebounce(mode_button, now)) {
    setting = !setting;
    // Entering or leaving setup mode never charges the paused elapsed time.
    last_tick_ms = now;
    changed = true;
  } else if (fellAfterDebounce(set_button, now) && setting) {
    advanceMinute();
    last_tick_ms = now;
    changed = true;
  }

  // Unsigned subtraction is wrap-safe across the 32-bit millis() rollover.
  if (!setting && static_cast<uint32_t>(now - last_tick_ms) >= SECOND_MS) {
    last_tick_ms += SECOND_MS;
    advanceSecond();
    changed = true;
  }

  if (changed) {
    publishWatchState();
  }
}
