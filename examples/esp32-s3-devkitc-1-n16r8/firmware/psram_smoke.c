typedef unsigned int uint32_t;
typedef unsigned char uint8_t;

#define UART0_FIFO (*(volatile uint32_t *)0x60000000u)
#define UART0_STATUS (*(volatile uint32_t *)0x6000001Cu)
#define MMU_TABLE ((volatile uint32_t *)0x600C5000u)
#define PSRAM_BASE ((volatile uint32_t *)0x3D000000u)
#define PSRAM_SIZE_BYTES 0x00800000u
#define PSRAM_LAST_WORD \
  ((volatile uint32_t *)(0x3D000000u + PSRAM_SIZE_BYTES - sizeof(uint32_t)))
#define PSRAM_MMU_FIRST_ENTRY 0x100u
#define PSRAM_MMU_ACCESS_SPIRAM (1u << 15)

#define TXFIFO_LEN 128u
#define SAMPLE_WORDS 256u

static void uart_write(const char *text) {
  for (const char *byte = text; *byte != '\0'; ++byte) {
    for (uint32_t spin = 0; spin < 100000u; ++spin) {
      if (((UART0_STATUS >> 16) & 0x3FFu) < TXFIFO_LEN) {
        break;
      }
    }
    UART0_FIFO = (uint8_t)*byte;
  }
}

static int psram_probe(void) {
  // Map the 128 × 64 KiB physical PSRAM pages into the 0x3D000000–0x3D7FFFFF
  // byte-addressable D-cache window. This smoke enters after ROM startup and
  // therefore programs the same MMU table the ROM would normally populate.
  for (uint32_t page = 0; page < PSRAM_SIZE_BYTES / 0x10000u; ++page) {
    MMU_TABLE[PSRAM_MMU_FIRST_ENTRY + page] = PSRAM_MMU_ACCESS_SPIRAM | page;
  }

  for (uint32_t i = 0; i < SAMPLE_WORDS; ++i) {
    PSRAM_BASE[i] = 0xA5C30000u ^ (i * 0x01010101u);
  }

  for (uint32_t i = 0; i < SAMPLE_WORDS; ++i) {
    const uint32_t expected = 0xA5C30000u ^ (i * 0x01010101u);
    if (PSRAM_BASE[i] != expected) {
      return 0;
    }
  }

  // Check that the first, middle, and final words in the 8 MiB window are
  // independent locations, including the final byte range declared in YAML.
  volatile uint32_t *const middle = (volatile uint32_t *)0x3D400000u;
  PSRAM_BASE[0] = 0x13579BDFu;
  *middle = 0x2468ACE0u;
  *PSRAM_LAST_WORD = 0x5AA55AA5u;

  return PSRAM_BASE[0] == 0x13579BDFu && *middle == 0x2468ACE0u &&
         *PSRAM_LAST_WORD == 0x5AA55AA5u;
}

__attribute__((noreturn, used, section(".entry")))
void _start(void) {
  if (psram_probe()) {
    uart_write("ESP32_S3_N16R8_PSRAM_PASS bytes=8388608\n");
  } else {
    uart_write("ESP32_S3_N16R8_PSRAM_FAIL\n");
  }

  for (;;) {
    __asm__ volatile("nop" ::: "memory");
  }
}
