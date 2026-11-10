#include <Arduino.h>

#define GPIO_FUNC4_OUT_SEL_CFG (*(volatile uint32_t *)0x60004564u)
#define MCPWM0_CLK_CFG (*(volatile uint32_t *)0x6001E000u)
#define MCPWM0_TIMER0_CFG0 (*(volatile uint32_t *)0x6001E004u)
#define MCPWM0_TIMER0_CFG1 (*(volatile uint32_t *)0x6001E008u)
#define MCPWM0_TIMER0_STATUS (*(volatile uint32_t *)0x6001E010u)
#define MCPWM0_GEN0_TSTMP_A (*(volatile uint32_t *)0x6001E040u)
#define MCPWM0_GEN0_A (*(volatile uint32_t *)0x6001E050u)

void setup() {
  Serial.begin(115200);
  pinMode(4, OUTPUT);
  GPIO_FUNC4_OUT_SEL_CFG = 160u;                 // MCPWM0 PWM0A
  MCPWM0_CLK_CFG = 15u;                          // CLK_160M / 16 = 10 MHz
  MCPWM0_GEN0_TSTMP_A = 1500u;
  MCPWM0_GEN0_A = 0x12u;                         // up-count TEZ high, TEA low
  MCPWM0_TIMER0_CFG0 = 9u | (19999u << 8);       // 1 MHz timer, 20 ms period
  MCPWM0_TIMER0_CFG1 = 1u | (2u << 3);            // up-count, continuous

  // Wait for the 1.5 ms compare edge before reporting the UART marker.
  while ((MCPWM0_TIMER0_STATUS & 0xFFFFu) < 1500u) {
    __asm__ volatile("nop");
  }
  Serial.println("ESP32_S3_DEVKITC1_V10_MCPWM_SERVO_OK");
  Serial.flush();
}

void loop() {}
