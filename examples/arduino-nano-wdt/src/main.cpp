// Stock ArduinoCore-avr + avr-libc <avr/wdt.h> watchdog idioms.
#include <Arduino.h>
#include <avr/wdt.h>
#include <avr/sleep.h>

// avr-libc's recommended early MCUSR capture: with WDRF set the watchdog comes
// back enabled at 16 ms, so it must be turned off before main() runs.
uint8_t mcusr_mirror __attribute__((section(".noinit")));
uint32_t magic __attribute__((section(".noinit")));
uint8_t boots __attribute__((section(".noinit")));
uint8_t isr_then_reset __attribute__((section(".noinit")));
void get_mcusr(void) __attribute__((naked, used, section(".init3")));
void get_mcusr(void) {
  mcusr_mirror = MCUSR;
  MCUSR = 0;
  wdt_disable();
}

volatile uint16_t wdt_hits;
volatile bool reset_mode_armed;
ISR(WDT_vect) {
  wdt_hits++;
  if (reset_mode_armed) isr_then_reset = 1;
}

static void wdt_irq_mode(uint8_t prescale, bool with_reset) {
  cli();
  wdt_reset();
  WDTCSR = _BV(WDCE) | _BV(WDE);
  WDTCSR = _BV(WDIE) | (with_reset ? _BV(WDE) : 0) | prescale;
  sei();
}

void setup() {
  Serial.begin(115200);
  if (magic != 0x57445421UL) { magic = 0x57445421UL; boots = 0; isr_then_reset = 0; }
  Serial.print("boot "); Serial.print(boots);
  Serial.print(" mcusr="); if (mcusr_mirror < 16) Serial.print('0');
  Serial.print(mcusr_mirror, HEX);
  Serial.print(" wdtcsr="); Serial.println(WDTCSR, HEX);
  Serial.flush();

  if (boots == 0) {
    // 1. Interrupt mode, 16 ms: delay(100) sees six time-outs.
    wdt_hits = 0;
    wdt_irq_mode(WDTO_15MS, false);
    delay(100);
    wdt_disable();
    Serial.print("isr="); Serial.println(wdt_hits);
    // 2. wdt_reset() keeps a 60 ms reset-mode watchdog away for 300 ms.
    wdt_enable(WDTO_60MS);
    for (uint8_t i = 0; i < 10; i++) { delay(30); wdt_reset(); }
    wdt_disable();
    Serial.println("kick OK");
    // 3. Power-down sleep, woken by the 250 ms watchdog interrupt.
    Serial.flush();
    unsigned long t0 = millis();
    wdt_hits = 0;
    wdt_irq_mode(WDTO_250MS, false);
    set_sleep_mode(SLEEP_MODE_PWR_DOWN);
    sleep_enable();
    sleep_cpu();
    sleep_disable();
    wdt_disable();
    // Timer0 stops in power-down, so millis() must not move.
    Serial.print("pwrdown woke hits="); Serial.print(wdt_hits);
    Serial.print(" dmillis="); Serial.println(millis() - t0);
    // 4. Hang: a 120 ms reset-mode watchdog restarts the chip.
    boots = 1;
    Serial.println("hang"); Serial.flush();
    wdt_enable(WDTO_120MS);
    for (;;) {}
  } else if (boots == 1) {
    // 5. Interrupt + reset mode: the ISR runs once, its vector entry clears
    // WDIE, and the next time-out resets.
    boots = 2;
    reset_mode_armed = true;
    wdt_irq_mode(WDTO_30MS, true);
    for (;;) {}
  } else {
    Serial.print("irq_then_reset="); Serial.println(isr_then_reset);
    Serial.println("WDT DONE");
  }
}

void loop() {}
