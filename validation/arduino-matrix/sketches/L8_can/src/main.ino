// LabWired Arduino matrix L8 — on-chip CAN loopback (register-level).
// Matches engine unit-test sequences (bxCAN BTR.LBKM / FDCAN TEST.LBCK).
// Avoid CMSIS names (CAN1, CAN_BASE, FDCAN1, RCC_BASE) — they are macros.

#include <Arduino.h>
#include <stdint.h>

static void logBegin() {
  Serial.begin(115200);
#if defined(ARDUINO_ARCH_RP2040) && !defined(ARDUINO_ARCH_MBED)
  // arduino-pico (Pico 2): Serial is native USB CDC and drops writes until
  // the host opens the port (DTR), on silicon and on the twin alike.
  while (!Serial) delay(1);
#endif
#if defined(ARDUINO_USB_CDC_ON_BOOT) && (ARDUINO_USB_CDC_ON_BOOT)
  Serial.setTxTimeoutMs(0);
  Serial0.begin(115200);
#endif
  delay(1);
}

static void logLine(const char *s) {
  Serial.println(s);
#if defined(ARDUINO_USB_CDC_ON_BOOT) && (ARDUINO_USB_CDC_ON_BOOT)
  Serial0.println(s);
#endif
}

static inline void mmio_w(uint32_t addr, uint32_t val) {
  *(volatile uint32_t *)(uintptr_t)addr = val;
}
static inline uint32_t mmio_r(uint32_t addr) {
  return *(volatile uint32_t *)(uintptr_t)addr;
}

#if defined(ARDUINO_NUCLEO_H563ZI) || defined(STM32H563xx) || defined(STM32H563ZITx)
// H5 FDCAN1 — exact enter_loopback from crates/core/src/peripherals/fdcan.rs
static bool lw_can_probe() {
  const uint32_t fd = 0x4000A400u;
  const uint32_t rcc = 0x44020C00u;
  // H5 RCC APB1HENR @ +0xA0, FDCAN1EN bit 9 (RM0481 / chip yaml clock gate)
  mmio_w(rcc + 0xA0, mmio_r(rcc + 0xA0) | (1u << 9));

  mmio_w(fd + 0x18, 0x3u);   // CCCR INIT|CCE
  mmio_w(fd + 0x18, 0xA3u);  // + TEST | MON
  mmio_w(fd + 0x10, 1u << 4); // TEST.LBCK
  // TX element 0 @ SRAMCAN+0x278: std ID 0x123, DLC 1
  mmio_w(fd + 0x800 + 0x278, 0x123u << 18);
  mmio_w(fd + 0x800 + 0x27C, 1u << 16);
  mmio_w(fd + 0x800 + 0x280, 0xA5u);
  mmio_w(fd + 0x18, 0xA2u); // leave INIT, keep TEST|MON (CCE clears)
  mmio_w(fd + 0x0CC, 1u);   // TXBAR buffer 0

  // TX completes on a later peripheral tick — spin.
  for (int i = 0; i < 100000; i++) {
    if ((mmio_r(fd + 0x90) & 0x7Fu) != 0) {
      uint32_t w0 = mmio_r(fd + 0x800 + 0xB0);
      uint32_t w2 = mmio_r(fd + 0x800 + 0xB0 + 8);
      return ((w0 >> 18) & 0x7FFu) == 0x123u && (w2 & 0xFFu) == 0xA5u;
    }
  }
  return false;
}
#define LW_HAS_CAN 1

#elif defined(ARDUINO_NUCLEO_U575ZI_Q) || defined(STM32U575xx) || defined(STM32U575ZITx) || defined(STM32U545xx)
// U5 FDCAN1 — same Bosch M_CAN enter_loopback sequence as the H5. RCC base is
// 0x46020C00 and FDCAN1EN lives in APB1ENR2 @ +0xA0 bit 9 (RM0456), NOT
// APB1ENR1. Internal loopback (TEST.LBCK): no pads wired — FDCAN1 would be
// PB8/PB9 (AF9) on the NUCLEO-U575ZI-Q headers, verified against DS13737.
static bool lw_can_probe() {
  const uint32_t fd = 0x4000A400u;
  const uint32_t rcc = 0x46020C00u;
  // U5 RCC APB1ENR2 @ +0xA0, FDCAN1EN bit 9 (SVD RCC_APB1ENR2.FDCAN1EN)
  mmio_w(rcc + 0xA0, mmio_r(rcc + 0xA0) | (1u << 9));

  mmio_w(fd + 0x18, 0x3u);   // CCCR INIT|CCE
  mmio_w(fd + 0x18, 0xA3u);  // + TEST | MON
  mmio_w(fd + 0x10, 1u << 4); // TEST.LBCK
  // TX element 0 @ SRAMCAN+0x278: std ID 0x123, DLC 1
  mmio_w(fd + 0x800 + 0x278, 0x123u << 18);
  mmio_w(fd + 0x800 + 0x27C, 1u << 16);
  mmio_w(fd + 0x800 + 0x280, 0xA5u);
  mmio_w(fd + 0x18, 0xA2u); // leave INIT, keep TEST|MON (CCE clears)
  mmio_w(fd + 0x0CC, 1u);   // TXBAR buffer 0

  // TX completes on a later peripheral tick — spin.
  for (int i = 0; i < 100000; i++) {
    if ((mmio_r(fd + 0x90) & 0x7Fu) != 0) {
      uint32_t w0 = mmio_r(fd + 0x800 + 0xB0);
      uint32_t w2 = mmio_r(fd + 0x800 + 0xB0 + 8);
      return ((w0 >> 18) & 0x7FFu) == 0x123u && (w2 & 0xFFu) == 0xA5u;
    }
  }
  return false;
}
#define LW_HAS_CAN 1

#elif defined(ARDUINO_NUCLEO_L476RG) || defined(STM32L476xx)
static bool lw_can_probe() {
  const uint32_t can = 0x40006400u;
  const uint32_t rcc = 0x40021000u;
  mmio_w(rcc + 0x58, mmio_r(rcc + 0x58) | (1u << 25));
  mmio_w(can + 0x00, 0x1);
  mmio_w(can + 0x1C, 0x405C0009u | (1u << 30));
  mmio_w(can + 0x200, 0x2A1C0E01u);
  mmio_w(can + 0x204, 0);
  mmio_w(can + 0x20C, 1);
  mmio_w(can + 0x214, 0);
  mmio_w(can + 0x240, 0);
  mmio_w(can + 0x244, 0);
  mmio_w(can + 0x21C, 1);
  mmio_w(can + 0x200, 0x2A1C0E00u);
  mmio_w(can + 0x00, 0);
  mmio_w(can + 0x184, (1u << 16) | 1u);
  mmio_w(can + 0x188, 0xA5u);
  mmio_w(can + 0x180, (0x123u << 21) | 1u);
  for (int i = 0; i < 10000; i++) {
    if ((mmio_r(can + 0x0C) & 0x3u) != 0) {
      uint32_t rir = mmio_r(can + 0x1B0);
      uint32_t rdl = mmio_r(can + 0x1B8);
      mmio_w(can + 0x0C, (1u << 5));
      return ((rir >> 21) & 0x7FFu) == 0x123u && (rdl & 0xFFu) == 0xA5u;
    }
  }
  return false;
}
#define LW_HAS_CAN 1

#elif defined(STM32F1xx) || defined(ARDUINO_BLUEPILL_F103C8) || defined(ARDUINO_GENERIC_F103C8TX) || \
    defined(ARDUINO_BLUEPILL_F103CB) || defined(STM32F103xB) || defined(STM32F103xE) || defined(CH32V20x) || defined(CH32V30x)
// CH32V20x and CH32V30x keep the F1 bxCAN1 map at 0x40006400 and CAN1EN at
// RCC APB1PCENR (0x4002101C) bit 25 (ch32v20x.h / ch32v30x.h CAN1_BASE,
// RCC_APB1Periph_CAN1).
static bool lw_can_probe() {
  const uint32_t can = 0x40006400u;
  const uint32_t rcc = 0x40021000u;
  mmio_w(rcc + 0x1C, mmio_r(rcc + 0x1C) | (1u << 25));
  mmio_w(can + 0x00, 0x1);
  mmio_w(can + 0x1C, 0x405C0009u | (1u << 30));
  mmio_w(can + 0x200, 0x2A1C0E01u);
  mmio_w(can + 0x204, 0);
  mmio_w(can + 0x20C, 1);
  mmio_w(can + 0x214, 0);
  mmio_w(can + 0x240, 0);
  mmio_w(can + 0x244, 0);
  mmio_w(can + 0x21C, 1);
  mmio_w(can + 0x200, 0x2A1C0E00u);
  mmio_w(can + 0x00, 0);
  mmio_w(can + 0x184, (1u << 16) | 1u);
  mmio_w(can + 0x188, 0xA5u);
  mmio_w(can + 0x180, (0x123u << 21) | 1u);
  for (int i = 0; i < 10000; i++) {
    if ((mmio_r(can + 0x0C) & 0x3u) != 0) {
      uint32_t rir = mmio_r(can + 0x1B0);
      uint32_t rdl = mmio_r(can + 0x1B8);
      mmio_w(can + 0x0C, (1u << 5));
      return ((rir >> 21) & 0x7FFu) == 0x123u && (rdl & 0xFFu) == 0xA5u;
    }
  }
  return false;
}
#define LW_HAS_CAN 1

#elif defined(STM32F407xx) || defined(STM32F40_41xxx) || defined(ARDUINO_DISCO_F407VG) || \
    defined(ARDUINO_BLACK_F407VE) || defined(ARDUINO_BLACK_F407VG) || defined(ARDUINO_DIYMORE_F407VGT) || \
    defined(STM32F446xx) || defined(ARDUINO_NUCLEO_F446RE)
// F4 bxCAN1 @ 0x40006400; RCC APB1ENR @ 0x40023800+0x40, CAN1EN bit 25
// (RM0090 for the F407, RM0390 §6.3.13 / §30 for the F446: same block).
static bool lw_can_probe() {
  const uint32_t can = 0x40006400u;
  const uint32_t rcc = 0x40023800u;
  mmio_w(rcc + 0x40, mmio_r(rcc + 0x40) | (1u << 25));
  mmio_w(can + 0x00, 0x1);
  mmio_w(can + 0x1C, 0x405C0009u | (1u << 30)); // BTR + LBKM
  mmio_w(can + 0x200, 0x2A1C0E01u);
  mmio_w(can + 0x204, 0);
  mmio_w(can + 0x20C, 1);
  mmio_w(can + 0x214, 0);
  mmio_w(can + 0x240, 0);
  mmio_w(can + 0x244, 0);
  mmio_w(can + 0x21C, 1);
  mmio_w(can + 0x200, 0x2A1C0E00u);
  mmio_w(can + 0x00, 0);
  mmio_w(can + 0x184, (1u << 16) | 1u);
  mmio_w(can + 0x188, 0xA5u);
  mmio_w(can + 0x180, (0x123u << 21) | 1u);
  for (int i = 0; i < 10000; i++) {
    if ((mmio_r(can + 0x0C) & 0x3u) != 0) {
      uint32_t rir = mmio_r(can + 0x1B0);
      uint32_t rdl = mmio_r(can + 0x1B8);
      mmio_w(can + 0x0C, (1u << 5));
      return ((rir >> 21) & 0x7FFu) == 0x123u && (rdl & 0xFFu) == 0xA5u;
    }
  }
  return false;
}
#define LW_HAS_CAN 1

#elif defined(ARDUINO_NUCLEO_G0B1RE) || defined(STM32G0B1xx)
// G0B1 FDCAN1 — the H5 enter_loopback sequence again. Registers at
// 0x40006400, SRAMCAN a separate block at 0x4000B400 (stm32g0b1xx.h
// FDCAN1_BASE / SRAMCAN_BASE). RCC APBENR1 @ 0x40021000 + 0x3C, FDCANEN bit 12
// (RCC_APBENR1_FDCANEN_Pos).
static bool lw_can_probe() {
  const uint32_t fd = 0x40006400u;
  const uint32_t ram = 0x4000B400u;
  const uint32_t rcc = 0x40021000u;
  mmio_w(rcc + 0x3C, mmio_r(rcc + 0x3C) | (1u << 12));

  mmio_w(fd + 0x18, 0x3u);   // CCCR INIT|CCE
  mmio_w(fd + 0x18, 0xA3u);  // + TEST | MON
  mmio_w(fd + 0x10, 1u << 4); // TEST.LBCK
  // TX element 0 @ SRAMCAN+0x278: std ID 0x123, DLC 1
  mmio_w(ram + 0x278, 0x123u << 18);
  mmio_w(ram + 0x27C, 1u << 16);
  mmio_w(ram + 0x280, 0xA5u);
  mmio_w(fd + 0x18, 0xA2u); // leave INIT, keep TEST|MON (CCE clears)
  mmio_w(fd + 0x0CC, 1u);   // TXBAR buffer 0

  // TX completes on a later peripheral tick — spin.
  for (int i = 0; i < 100000; i++) {
    if ((mmio_r(fd + 0x90) & 0x7Fu) != 0) {
      uint32_t w0 = mmio_r(ram + 0xB0);
      uint32_t w2 = mmio_r(ram + 0xB0 + 8);
      return ((w0 >> 18) & 0x7FFu) == 0x123u && (w2 & 0xFFu) == 0xA5u;
    }
  }
  return false;
}
#define LW_HAS_CAN 1

#elif defined(ARDUINO_ESP32C3_DEV) || defined(CONFIG_IDF_TARGET_ESP32C3)
// ESP32-C3 TWAI @ 0x6002B000 — the S3 register map (twai_struct.h identical).
// Ungate the APB clock and release reset first, as on silicon
// (SYSTEM_PERIP_CLK_EN0 @0x600C0010 / PERIP_RST_EN0 @0x600C0018, TWAI bit 19),
// then self-RX + decode std ID 0x123 / data 0xA5.
static bool lw_can_probe() {
  const uint32_t sys = 0x600C0000u;
  mmio_w(sys + 0x10, mmio_r(sys + 0x10) | (1u << 19));
  mmio_w(sys + 0x18, mmio_r(sys + 0x18) & ~(1u << 19));
  const uint32_t twai = 0x6002B000u;
  mmio_w(twai + 0x00, 0x0); // leave reset mode
  mmio_w(twai + 0x40, 0x01u); // DLC=1
  mmio_w(twai + 0x44, 0x24u); // ID[10:3] of 0x123
  mmio_w(twai + 0x48, 0x60u); // ID[2:0]<<5
  mmio_w(twai + 0x4C, 0xA5u);
  mmio_w(twai + 0x04, 1u << 4); // CMD.self_rx_req
  if ((mmio_r(twai + 0x08) & 1u) == 0) {
    return false;
  }
  if ((mmio_r(twai + 0x74) & 0x7Fu) != 1u) {
    return false;
  }
  uint8_t fi = (uint8_t)(mmio_r(twai + 0x40) & 0xFFu);
  uint8_t id_hi = (uint8_t)(mmio_r(twai + 0x44) & 0xFFu);
  uint8_t id_lo = (uint8_t)(mmio_r(twai + 0x48) & 0xFFu);
  uint8_t data0 = (uint8_t)(mmio_r(twai + 0x4C) & 0xFFu);
  uint16_t sid = ((uint16_t)id_hi << 3) | ((uint16_t)id_lo >> 5);
  return fi == 0x01u && sid == 0x123u && data0 == 0xA5u;
}
#define LW_HAS_CAN 1

#elif defined(ARDUINO_ESP32S3_DEV) || defined(CONFIG_IDF_TARGET_ESP32S3)
// ESP32-S3 TWAI @ 0x6002B000 — self-RX + decode std ID 0x123 / data 0xA5.
// Model stores one payload byte per 32-bit word slot (0x40,0x44,0x48,0x4C…).
static bool lw_can_probe() {
  const uint32_t twai = 0x6002B000u;
  mmio_w(twai + 0x00, 0x0); // leave reset mode
  // SJA1000 SFF layout across word slots: FI, ID[10:3], ID[2:0]<<5, data0
  mmio_w(twai + 0x40, 0x01u); // DLC=1
  mmio_w(twai + 0x44, 0x24u); // ID[10:3] of 0x123
  mmio_w(twai + 0x48, 0x60u); // ID[2:0]<<5
  mmio_w(twai + 0x4C, 0xA5u);
  mmio_w(twai + 0x04, 1u << 4); // CMD.self_rx_req
  if ((mmio_r(twai + 0x08) & 1u) == 0) {
    return false;
  }
  if ((mmio_r(twai + 0x74) & 0x7Fu) != 1u) {
    return false;
  }
  uint8_t fi = (uint8_t)(mmio_r(twai + 0x40) & 0xFFu);
  uint8_t id_hi = (uint8_t)(mmio_r(twai + 0x44) & 0xFFu);
  uint8_t id_lo = (uint8_t)(mmio_r(twai + 0x48) & 0xFFu);
  uint8_t data0 = (uint8_t)(mmio_r(twai + 0x4C) & 0xFFu);
  uint16_t sid = ((uint16_t)id_hi << 3) | ((uint16_t)id_lo >> 5);
  return fi == 0x01u && sid == 0x123u && data0 == 0xA5u;
}
#define LW_HAS_CAN 1

#elif defined(ARDUINO_ESP32S2_DEV) || defined(CONFIG_IDF_TARGET_ESP32S2)
// ESP32-S2 TWAI @ 0x3F42B000 (esp32s2.peripherals.ld) — the S3 twai_dev_t
// map, so the same self-RX + RX_MSG_CNT + std ID 0x123 / data 0xA5 check.
static bool lw_can_probe() {
  const uint32_t twai = 0x3F42B000u;
  mmio_w(twai + 0x00, 0x0); // leave reset mode
  mmio_w(twai + 0x40, 0x01u); // DLC=1
  mmio_w(twai + 0x44, 0x24u); // ID[10:3] of 0x123
  mmio_w(twai + 0x48, 0x60u); // ID[2:0]<<5
  mmio_w(twai + 0x4C, 0xA5u);
  mmio_w(twai + 0x04, 1u << 4); // CMD.self_rx_req
  if ((mmio_r(twai + 0x08) & 1u) == 0) {
    return false;
  }
  if ((mmio_r(twai + 0x74) & 0x7Fu) != 1u) {
    return false;
  }
  uint8_t fi = (uint8_t)(mmio_r(twai + 0x40) & 0xFFu);
  uint8_t id_hi = (uint8_t)(mmio_r(twai + 0x44) & 0xFFu);
  uint8_t id_lo = (uint8_t)(mmio_r(twai + 0x48) & 0xFFu);
  uint8_t data0 = (uint8_t)(mmio_r(twai + 0x4C) & 0xFFu);
  uint16_t sid = ((uint16_t)id_hi << 3) | ((uint16_t)id_lo >> 5);
  return fi == 0x01u && sid == 0x123u && data0 == 0xA5u;
}
#define LW_HAS_CAN 1

#elif defined(ARDUINO_ESP32_DEV) || defined(CONFIG_IDF_TARGET_ESP32)
// Classic ESP32 TWAI @ 0x3FF6B000 — self-RX + ID/data buffer check
static bool lw_can_probe() {
  const uint32_t twai = 0x3FF6B000u;
  mmio_w(twai + 0x00, 0x0); // leave reset mode
  mmio_w(twai + 0x40, 0x01u);
  mmio_w(twai + 0x44, 0x24u);
  mmio_w(twai + 0x48, 0x60u);
  mmio_w(twai + 0x4C, 0xA5u);
  mmio_w(twai + 0x04, 1u << 4); // CMD.self_rx
  if ((mmio_r(twai + 0x08) & 1u) == 0) {
    return false;
  }
  uint8_t fi = (uint8_t)(mmio_r(twai + 0x40) & 0xFFu);
  uint8_t id_hi = (uint8_t)(mmio_r(twai + 0x44) & 0xFFu);
  uint8_t id_lo = (uint8_t)(mmio_r(twai + 0x48) & 0xFFu);
  uint8_t data0 = (uint8_t)(mmio_r(twai + 0x4C) & 0xFFu);
  uint16_t sid = ((uint16_t)id_hi << 3) | ((uint16_t)id_lo >> 5);
  return fi == 0x01u && sid == 0x123u && data0 == 0xA5u;
}
#define LW_HAS_CAN 1

#elif defined(ARDUINO_UNOR4_WIFI) || defined(ARDUINO_UNOR4_MINIMA)
// RA4M1 CAN0 @ 0x40050000 (FSP R7FA4M1AB.h R_CAN0_Type), internal loopback
// (TCR.TSTE | TSTM = 11) in the hardware manual's start-up order: leave
// sleep into reset mode, set BCR, halt, select the test mode, load the
// mailboxes, operate. Module stop: MSTPCRB @ 0x40047000 bit 2 (MSTPB2).
// MB0 sends std ID 0x123 / data 0xA5; MB1 receives it (every mask invalid,
// so the match is exact).
static bool lw_can_wait16(uint32_t addr, uint16_t mask, uint16_t want) {
  for (int i = 0; i < 10000; i++) {
    if ((*(volatile uint16_t *)(uintptr_t)addr & mask) == want) {
      return true;
    }
  }
  return false;
}

static bool lw_can_probe() {
  const uint32_t can = 0x40050000u;
  const uint32_t ctlr = can + 0x840, str = can + 0x842;
  volatile uint8_t *mctl = (volatile uint8_t *)(uintptr_t)(can + 0x820);
  mmio_w(0x40047000u, mmio_r(0x40047000u) & ~(1u << 2));

  // CANM = 01 (reset), SLPM = 0; wait RSTST = 1, SLPST = 0.
  *(volatile uint16_t *)(uintptr_t)ctlr = 0x0100;
  if (!lw_can_wait16(str, 0x0500, 0x0100)) return false;
  // 500 kbit/s from PCLKB 24 MHz: BRP = 2, TSEG1 = 11 Tq, TSEG2 = 4 Tq.
  mmio_w(can + 0x844, 0xA0020300u);
  // CANM = 10 (halt); TCR is written only here.
  *(volatile uint16_t *)(uintptr_t)ctlr = 0x0200;
  if (!lw_can_wait16(str, 0x0300, 0x0200)) return false;
  *(volatile uint8_t *)(uintptr_t)(can + 0x858) = 0x07;
  mmio_w(can + 0x428, 0xFFFFFFFFu); // MKIVLR: every mask invalid
  mmio_w(can + 0x42C, 0);           // MIER: polled
  mmio_w(can + 0x200, 0x123u << 18);                      // MB0.ID
  *(volatile uint16_t *)(uintptr_t)(can + 0x204) = 1;     // MB0.DL
  *(volatile uint8_t *)(uintptr_t)(can + 0x206) = 0xA5;   // MB0.D[0]
  mmio_w(can + 0x210, 0x123u << 18);                      // MB1.ID
  // CANM = 00 (operation).
  *(volatile uint16_t *)(uintptr_t)ctlr = 0x0000;
  if (!lw_can_wait16(str, 0x0700, 0x0000)) return false;
  mctl[1] = 0x40; // RECREQ
  mctl[0] = 0x80; // TRMREQ
  for (int i = 0; i < 100000; i++) {
    if (mctl[1] & 0x01u) { // NEWDATA
      uint32_t id = mmio_r(can + 0x210);
      uint16_t dl = *(volatile uint16_t *)(uintptr_t)(can + 0x214);
      uint8_t d0 = *(volatile uint8_t *)(uintptr_t)(can + 0x216);
      bool sent = (mctl[0] & 0x01u) != 0; // SENTDATA
      return sent && ((id >> 18) & 0x7FFu) == 0x123u && (dl & 0xFu) == 1u && d0 == 0xA5u;
    }
  }
  return false;
}
#define LW_HAS_CAN 1

#else
static bool lw_can_probe() { return false; }
#define LW_HAS_CAN 0
#endif

void setup() {
  logBegin();
  logLine("LW_L8_BOOT");
#if LW_HAS_CAN
  if (lw_can_probe()) {
    logLine("LW_L8_OK");
  } else {
    logLine("LW_L8_FAIL");
  }
#else
  logLine("LW_L8_FAIL no_can");
#endif
}

void loop() {}
