// ESP32-S3-DevKitC-1 v1.1 onboard RGB LED smoke.
//
// The board guide maps its WS2812 data line to GPIO38. This program writes one
// GRB red pixel into RMT0 memory, routes RMT0 through the GPIO matrix, waits
// for TX_END, then leaves the line low for the LED's reset/latch interval.
#include <Arduino.h>

namespace {
constexpr uintptr_t kGpioFunc38OutSelCfg = 0x600045EC;
constexpr uintptr_t kRmtBase = 0x60016000;
constexpr uintptr_t kRmtCh0Conf0 = kRmtBase + 0x20;
constexpr uintptr_t kRmtIntRaw = kRmtBase + 0x70;
constexpr uintptr_t kRmtIntClr = kRmtBase + 0x7C;
constexpr uintptr_t kRmtMem = kRmtBase + 0x400;
constexpr uint32_t kRmtOut0Signal = 81;
constexpr uint32_t kTxStart = 1u;
constexpr uint32_t kTxEnd = 1u;

uint32_t rmtBit(bool one) {
  // At 240 MHz with DIV_CNT=30, one RMT tick is 125 ns. The high pulse is
  // 375 ns for 0 and 750 ns for 1; both are separated by the decoder's 500 ns
  // threshold. The low interval is 625 ns.
  const uint32_t highTicks = one ? 6u : 3u;
  constexpr uint32_t lowTicks = 5u;
  return highTicks | (1u << 15) | (lowTicks << 16);
}

bool sendRedPixel() {
  auto *gpioRoute = reinterpret_cast<volatile uint32_t *>(kGpioFunc38OutSelCfg);
  auto *channelConf = reinterpret_cast<volatile uint32_t *>(kRmtCh0Conf0);
  auto *interruptRaw = reinterpret_cast<volatile uint32_t *>(kRmtIntRaw);
  auto *interruptClear = reinterpret_cast<volatile uint32_t *>(kRmtIntClr);
  auto *memory = reinterpret_cast<volatile uint32_t *>(kRmtMem);

  *gpioRoute = kRmtOut0Signal;

  // WS2812 transmits most-significant-bit first in GRB order. 0x00FF00 is red.
  constexpr uint32_t kRedGrb = 0x00FF00;
  for (int bit = 23, word = 0; bit >= 0; --bit, ++word) {
    memory[word] = rmtBit((kRedGrb >> bit) & 1u);
  }
  memory[24] = 0; // RMT end marker

  *interruptClear = kTxEnd;
  constexpr uint32_t kChannelConfig = (30u << 8) | (1u << 16); // divider / one block
  *channelConf = kChannelConfig;
  *channelConf = kChannelConfig | kTxStart;

  for (uint32_t i = 0; i < 10000000; ++i) {
    if ((*interruptRaw & kTxEnd) != 0) {
      *interruptClear = kTxEnd;
      // At 240 MHz this bounded low interval exceeds the WS2812 reset time.
      for (uint32_t i = 0; i < 16000; ++i) {
        asm volatile("nop");
      }
      return true;
    }
  }

  return false;
}
} // namespace

// The local CLI fast-boot fixture enters here, bypassing Arduino/IDF startup.
// It shares the same RMT and GPIO code as setup() and writes directly to UART0.
extern "C" __attribute__((used, noinline)) void labwired_entry() {
  const bool sent = sendRedPixel();
  auto *fifo = reinterpret_cast<volatile uint32_t *>(0x60000000);
  auto *status = reinterpret_cast<volatile uint32_t *>(0x6000001C);
  const char *marker = sent ? "DEVKITC1_V11_RGB38_TX_DONE\r\n"
                            : "DEVKITC1_V11_RGB38_TX_TIMEOUT\r\n";
  for (const char *p = marker; *p; ++p) {
    const char c = *p;
    for (uint32_t i = 0; i < 100000; ++i) {
      if (((*status >> 16) & 0x3FFu) < 128u) break;
    }
    *fifo = static_cast<uint8_t>(c);
  }
  for (;;) asm volatile("nop");
}

void setup() {
  Serial.begin(115200);
  Serial.println(sendRedPixel() ? "DEVKITC1_V11_RGB38_TX_DONE"
                                : "DEVKITC1_V11_RGB38_TX_TIMEOUT");
}

void loop() { delay(1000); }
