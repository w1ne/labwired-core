// LabWired ESP32 libgcc bit-op check — stock arduino-esp32 3.x.
// GCC lowers these builtins to calls into the mask ROM (__popcountsi2,
// __popcountdi2, __paritysi2); littlefs uses the same __popcountsi2.
#include <Arduino.h>
volatile uint32_t U = 0x00F0F001u;
volatile uint64_t A = 0x123456789ULL;
void setup() {
  Serial.begin(115200);
  delay(10);
  int pc = __builtin_popcount(U), pl = __builtin_popcountll(A), par = __builtin_parity(U);
  int one = __builtin_popcount((uint32_t)(U & 1u));
  Serial.printf("popcount=%d popcountll=%d parity=%d popcount1=%d\n", pc, pl, par, one);
  if (pc == 9 && pl == 15 && par == 1 && one == 1)
    Serial.println("LW_BITOPS_OK");
  else
    Serial.println("LW_BITOPS_FAIL");
}
void loop() { delay(1000); }
