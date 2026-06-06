// Minimal Unity test. Runs identically on real ESP32-S3 silicon and inside
// the LabWired simulator — the parity oracle compares the serial output of
// both. Keep it dependency-free so the only thing under test is the
// firmware-execution path (boot -> FreeRTOS scheduler -> first task -> setup).
#include <Arduino.h>
#include <unity.h>

void test_addition(void) { TEST_ASSERT_EQUAL_INT(4, 2 + 2); }
void test_string(void)   { TEST_ASSERT_EQUAL_STRING("labwired", "labwired"); }

// Exercises the hardware single-precision FPU: mul.s + add.s + float→int.
// `volatile` defeats constant-folding so real FP instructions are emitted.
void test_float(void) {
  volatile float a = 3.5f, b = 2.0f;
  float c = a * b + 1.0f;            // 8.0  (mul.s, add.s)
  TEST_ASSERT_EQUAL_INT(8, (int)c);  // (int) → trunc-to-zero
}

void setup() {
  // Arduino HardwareSerial on UART0 → ESP-IDF interrupt-driven UART driver.
  Serial0.begin(115200);
  Serial0.println("LWUART-SERIAL-OK");
  Serial0.flush();

  // Give the USB-CDC host time to attach before Unity starts printing.
  delay(2000);
  UNITY_BEGIN();
  RUN_TEST(test_addition);
  RUN_TEST(test_string);
  RUN_TEST(test_float);
  UNITY_END();
}

void loop() {}
