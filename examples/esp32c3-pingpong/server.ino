// Ping Pong - Player A
//
// Serves a ball to the other board over a serial wire and counts the rally.
// The other board returns it and draws it on an OLED.
//
// Wiring (TX to RX both ways, plus a shared ground):
//   A GPIO6 (TX) ---> B GPIO7 (RX)
//   A GPIO7 (RX) <--- B GPIO6 (TX)
//   A GND        ---- B GND
//
// Without the ground wire neither board sees a byte.
//
// Every PONG that comes back is answered with the next PING, so the two boards
// keep each other going.

#include <Arduino.h>

static constexpr int PIN_LINK_TX = 6;
static constexpr int PIN_LINK_RX = 7;
static constexpr uint32_t LINK_BAUD = 115200;

// If a return never comes back the rally is over. Serve a new ball instead of
// waiting forever, so pulling a wire and putting it back recovers on its own.
static constexpr uint32_t RETURN_TIMEOUT_MS = 1000;

static uint32_t rally = 0;      // consecutive successful returns
static uint32_t longest = 0;    // best rally this power-cycle
static uint32_t servedAt = 0;
static bool waitingForReturn = false;

static void serve() {
  Serial1.print("PING\n");
  servedAt = millis();
  waitingForReturn = true;
}

void setup() {
  Serial.begin(115200);
  // Serial1 is the link to the other board. Serial stays on USB for messages
  // you read on a laptop, so the two never mix.
  Serial1.begin(LINK_BAUD, SERIAL_8N1, PIN_LINK_RX, PIN_LINK_TX);
  Serial.println("Player A ready - serving");
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
        serve();  // came back, send it again
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
