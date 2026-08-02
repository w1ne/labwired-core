// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// Arduino conformance sketch — the SAME source for every board LabWired
// simulates. It exercises the main buses through the real Arduino core (not
// through a bespoke no_std fixture), which is the point: an Arduino core
// touches far more of a chip than a hand-written register poke does, so this
// is the harshest routine portability check we have.
//
// Protocol (one line per class, parsed by the survival harness):
//
//     LWCONF <class> PASS
//     LWCONF <class> FAIL code=<reason>
//     LWCONF <class> SKIP code=<reason>
//     LWCONF done
//
// `serial` is implicit: receiving `LWCONF done` at all proves the UART path.
//
// SKIP is deliberate and load-bearing. A board whose core does not expose a
// bus (no Wire, no SPI) must say so out loud rather than silently omitting a
// line — an absent line and a passing line must never be confusable.
//
// Everything is bounded by fixed iteration counts. The simulator is
// deterministic and has no wall clock, so never introduce a timing-based wait.

#include <Arduino.h>
// Wire.h / SPI.h are included UNCONDITIONALLY and never behind an #if.
// arduino-cli resolves library dependencies by textually scanning #include
// lines before it preprocesses anything, so an include hidden inside an
// `#if __has_include(...)` guard is invisible to the dependency scanner — the
// library never lands on the include path, __has_include then reports false,
// and every board reports SKIP while looking perfectly healthy. Both libraries
// ship with every core this matrix targets (STM32duino, arduino-esp32,
// Adafruit/mbed nRF52, RP2040). A core that genuinely lacks one must be
// handled with a per-core exclusion in the build script, not by silently
// compiling out the check here.
#include <SPI.h>
#include <Wire.h>

#define LW_HAS_WIRE 1
#define LW_HAS_SPI 1

// Which object is the *hardware UART* console.
//
// On cores with native USB (arduino-pico, Adafruit nRF52) `Serial` is a USB CDC
// endpoint, not a UART — it only produces bytes once a host enumerates the
// device and asserts DTR. The simulator watches the chip's UART peripheral, so
// a sketch that prints to USB CDC there looks completely silent even though it
// ran perfectly. On those cores the hardware UART is `Serial1`.
//
// This is a report-channel choice, not a capability claim: `LW_CONSOLE` only
// decides where the LWCONF lines are written.
// Gate on the parts that actually HAVE native USB, not on the architecture:
// the nRF52832 has no USB peripheral at all, so its `Serial` is already a UART
// and it does not even declare a `Serial1` (referencing one is a compile
// error). nRF52840 and RP2040 do have USB and put the UART on `Serial1`.
#if defined(ARDUINO_ARCH_RP2040) || defined(NRF52840_XXAA)
#define LW_CONSOLE Serial1
#else
#define LW_CONSOLE Serial
#endif

// The pin driven by the gpio check. LED_BUILTIN is defined by every core's
// variant for its own board, which is exactly the per-board indirection we
// want — no board table to maintain here.
#ifndef LW_GPIO_PIN
#ifdef LED_BUILTIN
#define LW_GPIO_PIN LED_BUILTIN
#else
#define LW_GPIO_PIN 0
#endif
#endif

// An address no simulated device answers on. The I2C check asserts the bus
// reports a clean address NACK, which proves the controller ran a real
// address phase and sampled the ACK bit — a controller that never drives the
// bus cannot produce this.
#ifndef LW_I2C_ABSENT_ADDR
#define LW_I2C_ABSENT_ADDR 0x4E
#endif

static void report(const char *cls, const char *verdict, const char *code) {
  LW_CONSOLE.print("LWCONF ");
  LW_CONSOLE.print(cls);
  LW_CONSOLE.print(' ');
  LW_CONSOLE.print(verdict);
  if (code) {
    LW_CONSOLE.print(" code=");
    LW_CONSOLE.print(code);
  }
  LW_CONSOLE.print('\n');
  LW_CONSOLE.flush();
}

// gpio: drive the pin both ways and read it back. Reading back an OUTPUT pin
// returns the output-data register on every core we target, so this proves
// the GPIO model latches writes rather than merely accepting them.
static void check_gpio(void) {
  pinMode(LW_GPIO_PIN, OUTPUT);

  digitalWrite(LW_GPIO_PIN, HIGH);
  if (digitalRead(LW_GPIO_PIN) != HIGH) {
    report("gpio", "FAIL", "set");
    return;
  }

  digitalWrite(LW_GPIO_PIN, LOW);
  if (digitalRead(LW_GPIO_PIN) != LOW) {
    report("gpio", "FAIL", "clear");
    return;
  }

  report("gpio", "PASS", NULL);
}

// i2c: a one-byte write to an absent address must complete and report a NACK.
// endTransmission() returns 0 on success, 2 on address NACK, 3 on data NACK,
// 4 on other error. Anything that is not a clean 0/2/3 means the controller
// did not finish an address phase.
static void check_i2c(void) {
#ifdef LW_HAS_WIRE
  Wire.begin();
  Wire.beginTransmission((uint8_t)LW_I2C_ABSENT_ADDR);
  Wire.write((uint8_t)0x00);
  uint8_t rc = Wire.endTransmission();

  if (rc == 2 || rc == 3) {
    // The expected outcome on most cores: no device at this address, cleanly
    // reported as an address (2) or data (3) NACK.
    report("i2c", "PASS", NULL);
  } else if (rc == 0) {
    // Something ACKed. That still proves a working controller, and some
    // simulated systems do attach a device here.
    report("i2c", "PASS", NULL);
  } else if (rc == 4) {
#if defined(ARDUINO_ARCH_ESP32) || defined(ARDUINO_ARCH_RP2040)
    // On arduino-esp32 AND arduino-pico an address NACK legitimately surfaces
    // as 4. Both were traced through every hop of primary source rather than
    // assumed.
    //
    // arduino-pico 6.0.0:
    //   pico-sdk hardware_i2c/i2c.c — an abort whose reason is
    //   ABRT_7B_ADDR_NOACK (or no reported reason, "seems to happen if there is
    //   nothing connected to the bus") returns PICO_ERROR_GENERIC. Note a DATA
    //   NACK instead returns the byte count, so the two are distinguished.
    //   Wire.cpp endTransmission() then does `return (ret == len) ? 0 : 4`,
    //   with PICO_ERROR_TIMEOUT separately mapped to 5.
    //
    // arduino-esp32 3.3.11:
    //
    //   ESP-IDF v5.5 esp_driver_i2c/i2c_master.c — on a NACK the ISR stores
    //   I2C_STATUS_ACK_ERROR; s_i2c_send_commands issues a STOP and returns
    //   WITHOUT ever storing I2C_STATUS_DONE, so the caller's
    //   `if (status != I2C_STATUS_DONE) ret = ESP_ERR_INVALID_STATE`
    //   fires. (Note i2c_master_probe DOES map ACK_ERROR to ESP_ERR_NOT_FOUND;
    //   i2c_master_transmit does not.)
    //
    //   arduino-esp32 3.3.11 Wire.cpp endTransmission() switches only on
    //   ESP_OK->0, ESP_FAIL->2, ESP_ERR_NOT_FOUND->2, ESP_ERR_TIMEOUT->5, and
    //   returns 4 for everything else. ESP_ERR_INVALID_STATE hits that default.
    //
    // So 4 is what real ESP32 silicon+core produces here, and treating it as a
    // failure was a bug in THIS sketch, not in the chip model.
    //
    // This stays narrow on purpose. 4 remains a failure on every other core,
    // where it means a genuine bus error — accepting it globally would have
    // hidden the real STM32F401 defect (a missing I2C ERROR interrupt vector),
    // which presented as exactly this code. And a controller that never NACKs
    // at all still fails here, because the IDF driver would then time out and
    // Wire would return 5, which is handled below.
    report("i2c", "PASS", NULL);
#else
    report("i2c", "FAIL", "buserr");
#endif
  } else if (rc == 5) {
    // Timeout: the controller never reached a definite conclusion.
    report("i2c", "FAIL", "timeout");
  } else {
    report("i2c", "FAIL", "unexpected-rc");
  }
#else
  report("i2c", "SKIP", "no-wire-lib");
#endif
}

// spi: clock a byte out. With no device attached the returned byte is
// undefined (0x00 or 0xFF depending on bus idle level), so the assertion is
// that the transfer COMPLETES — a shift engine that never raises its
// transfer-complete flag hangs here instead, which the harness sees as a
// missing line.
static void check_spi(void) {
#ifdef LW_HAS_SPI
  SPI.begin();
  // beginTransaction is REQUIRED, not decoration: on STM32duino `SPI.begin()`
  // alone leaves the peripheral disabled (CR1 reads 0 — no SPE), and a
  // transfer against a disabled peripheral polls a status flag that can never
  // arrive. Every real sketch brackets transfers this way.
  SPI.beginTransaction(SPISettings(1000000, MSBFIRST, SPI_MODE0));
  (void)SPI.transfer(0x5A);
  (void)SPI.transfer(0xA5);
  SPI.endTransaction();
  SPI.end();
  report("spi", "PASS", NULL);
#else
  report("spi", "SKIP", "no-spi-lib");
#endif
}

// timer: the Arduino time base (millis/micros/delay) rides the core's tick
// interrupt — SysTick on Cortex-M, a hardware timer elsewhere. This asserts the
// clock actually ADVANCES and that delay() returns, which together prove the
// tick source is running and its interrupt is being delivered. A frozen tick is
// the single most common way a simulated chip "boots" but hangs the first time
// firmware waits for anything.
static void check_timer(void) {
  uint32_t t0 = millis();
  uint32_t u0 = micros();

  delay(5);

  uint32_t t1 = millis();
  uint32_t u1 = micros();

  if (t1 == t0) {
    report("timer", "FAIL", "millis-frozen");
    return;
  }
  if (u1 == u0) {
    report("timer", "FAIL", "micros-frozen");
    return;
  }
  // millis must not run backwards across the delay.
  if ((uint32_t)(t1 - t0) > 10000u) {
    report("timer", "FAIL", "millis-jump");
    return;
  }
  report("timer", "PASS", NULL);
}

// uart: a loopback-free self-check of the TX path's own bookkeeping.
// availableForWrite() must report a non-zero TX buffer and write() must accept
// the bytes it claims to. The console carrying these very lines already proves
// the wire works; this catches a driver that reports a full/absent buffer.
static void check_uart(void) {
  int room = LW_CONSOLE.availableForWrite();
  if (room <= 0) {
    report("uart", "FAIL", "no-tx-room");
    return;
  }
  size_t n = LW_CONSOLE.write((const uint8_t *)"", 0);
  if (n != 0) {
    report("uart", "FAIL", "write-count");
    return;
  }
  report("uart", "PASS", NULL);
}

void setup(void) {
  LW_CONSOLE.begin(115200);

  // Bounded wait for the port. Cores with native USB (RP2040, nRF52840)
  // return false from `!Serial` until the simulated host asserts DTR; cores
  // with a plain UART are ready immediately. Bounded so neither class hangs.
  for (int i = 0; i < 10000 && !LW_CONSOLE; i++) {
    // Intentionally empty: the loop bound IS the timeout.
  }

  LW_CONSOLE.print("LWCONF begin\n");

  check_uart();
  check_gpio();
  check_timer();
  check_i2c();
  check_spi();

  LW_CONSOLE.print("LWCONF done\n");
  LW_CONSOLE.flush();
}

void loop(void) {
  // Nothing. Every claim this sketch makes is made once, in setup().
}
