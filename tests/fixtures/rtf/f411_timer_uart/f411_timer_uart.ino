// Realtime-factor workload: WeAct Black Pill STM32F411CEU6 (STM32duino) with a
// 1 kHz HardwareTimer interrupt and a UART log every 100 ms.
// Runs forever; the gate stops it on a fixed cycle budget.
#include <HardwareTimer.h>

volatile uint32_t ticks = 0;
HardwareTimer *tim = nullptr;

void onTick() { ticks++; }

void setup() {
  Serial.begin(115200);
  tim = new HardwareTimer(TIM2);
  tim->setOverflow(1000, HERTZ_FORMAT);
  tim->attachInterrupt(onTick);
  tim->resume();
  Serial.println("RTF_READY f411_timer_uart");
}

void loop() {
  static uint32_t last = 0;
  uint32_t t = ticks;
  if (t - last >= 100) {
    last = t;
    Serial.print("ticks=");
    Serial.println(t);
  }
}
