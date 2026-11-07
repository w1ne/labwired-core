#include <Arduino.h>
#include <stdint.h>

namespace {
constexpr uintptr_t kRmtBase = 0x3FF56000;
constexpr uintptr_t kRmtConf0Ch0 = kRmtBase + 0x20;
constexpr uintptr_t kRmtConf1Ch0 = kRmtBase + 0x24;
constexpr uintptr_t kRmtIntRaw = kRmtBase + 0xA0;
constexpr uintptr_t kRmtIntClear = kRmtBase + 0xAC;
constexpr uintptr_t kRmtMem = kRmtBase + 0x800;
constexpr uintptr_t kGpioEnableW1ts = 0x3FF44024;
constexpr uintptr_t kGpioFunc0OutSel = 0x3FF44530;
constexpr uint32_t kRmtOut0Signal = 87;
constexpr uint32_t kRmtOut1Signal = 88;
constexpr uint32_t kRmtTxEndCh0 = 1u << 0;
constexpr uint32_t kRmtTxEndCh1 = 1u << 1;

struct Pulse {
  uint16_t duration;
  bool level;
};

uint32_t pack(const Pulse &first, const Pulse &second) {
  return (static_cast<uint32_t>(first.duration) & 0x7FFFu) |
         (static_cast<uint32_t>(first.level) << 15) |
         ((static_cast<uint32_t>(second.duration) & 0x7FFFu) << 16) |
         (static_cast<uint32_t>(second.level) << 31);
}

void transmitPixel(uint8_t channel, uint8_t pin, uint32_t signal,
                   uint8_t green, uint8_t red, uint8_t blue) {
  // WS2812 sends GRB, most-significant bit first. At 20 MHz these durations
  // form 350 ns/800 ns zero cells and 700 ns/450 ns one cells.
  const uint8_t grb[] = {green, red, blue};
  Pulse pulses[50] = {};
  size_t count = 0;
  for (uint8_t byte : grb) {
    for (int bit = 7; bit >= 0; --bit) {
      const bool one = (byte & (1u << bit)) != 0;
      pulses[count++] = {static_cast<uint16_t>(one ? 14 : 7), true};
      pulses[count++] = {static_cast<uint16_t>(one ? 9 : 16), false};
    }
  }
  pulses[count++] = {1200, false};  // 60 us reset/latch interval.
  pulses[count++] = {0, false};     // RMT end marker.

  auto *gpio_enable = reinterpret_cast<volatile uint32_t *>(kGpioEnableW1ts);
  auto *gpio_matrix = reinterpret_cast<volatile uint32_t *>(kGpioFunc0OutSel + pin * 4);
  *gpio_enable = 1u << pin;
  *gpio_matrix = signal;

  auto *rmt_conf0 = reinterpret_cast<volatile uint32_t *>(kRmtConf0Ch0 + channel * 8);
  auto *rmt_conf1 = reinterpret_cast<volatile uint32_t *>(kRmtConf1Ch0 + channel * 8);
  auto *rmt_int_clear = reinterpret_cast<volatile uint32_t *>(kRmtIntClear);
  auto *rmt_mem = reinterpret_cast<volatile uint32_t *>(kRmtMem + channel * 64 * 4);

  for (size_t i = 0; i < count; i += 2) {
    rmt_mem[i / 2] = pack(pulses[i], pulses[i + 1]);
  }

  *rmt_int_clear = 1u << channel;
  // Channel 0 uses APB (80 MHz), divided by four to a 20 MHz RMT tick.
  *rmt_conf0 = (1u << 31) | (1u << 24) | 4u;
  // REF_ALWAYS_ON selects APB; MEM_RD_RST resets the channel's read pointer.
  *rmt_conf1 = (1u << 17) | (1u << 19) | (1u << 3);
  *rmt_conf1 = (1u << 17) | (1u << 19) | 1u;  // TX_START

}
}  // namespace

void setup() {
  Serial.begin(115200);
  Serial.println("ESP32_DEVKITC32E_RMT_BEGIN");
  // Two independently timed RMT engines drive separate external pixel inputs.
  transmitPixel(0, 23, kRmtOut0Signal, 0x00, 0xFF, 0x00);
  transmitPixel(1, 18, kRmtOut1Signal, 0xFF, 0x00, 0x00);
  const auto *rmt_int_raw = reinterpret_cast<volatile uint32_t *>(kRmtIntRaw);
  for (uint32_t timeout = 0; timeout < 1000000; ++timeout) {
    if ((*rmt_int_raw & (kRmtTxEndCh0 | kRmtTxEndCh1)) ==
        (kRmtTxEndCh0 | kRmtTxEndCh1)) {
      Serial.println("ESP32_DEVKITC32E_RMT_TX_END=0,1");
      return;
    }
  }
  Serial.println("ESP32_DEVKITC32E_RMT_TIMEOUT");
}

void loop() {}
