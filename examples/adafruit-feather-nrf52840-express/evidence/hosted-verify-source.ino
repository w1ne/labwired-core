#include <stdint.h>

#define REG32(address) (*(volatile uint32_t *)(address))
#define UARTE0 0x40002000u
#define P0_PIN_CNF_25 0x50000764u
#define TXD_PIN 25u
#define MESSAGE "FEATHER_NRF52840_UART_OK\r\n"

static char message[] = MESSAGE;

void setup() {
  // Feather D0/TX is nRF52840 P0.25; set the pad to output for UARTE0.
  REG32(P0_PIN_CNF_25) = 0x3u;
  REG32(UARTE0 + 0x50Cu) = TXD_PIN;       // PSEL.TXD
  REG32(UARTE0 + 0x524u) = 0x01D60000u;   // 115200 baud
  REG32(UARTE0 + 0x500u) = 8u;            // ENABLE = UARTE
}

void loop() {
  REG32(UARTE0 + 0x120u) = 0;                // EVENTS_ENDTX
  REG32(UARTE0 + 0x544u) = (uint32_t)message; // TXD.PTR (RAM)
  REG32(UARTE0 + 0x548u) = sizeof(message) - 1;
  REG32(UARTE0 + 0x008u) = 1;                // TASKS_STARTTX
  while (REG32(UARTE0 + 0x120u) == 0) { }
  for (;;) { }
}
