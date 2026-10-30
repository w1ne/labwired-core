/*
 * LabWired CC2538DK flash-controller test (SPDX-License-Identifier: MIT).
 *
 * The steps of Renode's tests/platforms/CC2538/cc2538_flash_controller.robot
 * ("Should Write, Read and Erase Flash using Flash Controller"), written
 * against the public CC2538 ROM utility API (SWRU333A §3.2, Contiki-NG
 * dev/rom-util.h): the flash is changed only through ROM_PageErase /
 * ROM_ProgramFlash, which drive FLASH_CTRL (SWRU319C §8).
 *
 * Renode's test ELF is not redistributable source, so this is an
 * independent program printing the same milestones. It differs in one
 * line: FLASH SIZE is what ROM_GetFlashSize reads from DIECFG0 on a
 * CC2538SF53, 0x80000; Renode prints 0x10000 because its FLASH_CTRL model
 * returns the instant-of-reset DIECFG0 (FLASH_SIZE = 0, 64 KB).
 */
#include "contiki.h"
#include "dev/flash.h"
#include "dev/rom-util.h"

#include <stdint.h>
#include <stdio.h>

/* The two pages below the lock-bit (CCA) page: outside the image and
 * Coffee. FLASH_CCA_ADDR is the CCA structure inside that page. */
#define CCA_PAGE   (FLASH_CCA_ADDR & ~(uint32_t)(FLASH_PAGE_SIZE - 1))
#define TEST_ADDR  (CCA_PAGE - 2 * FLASH_PAGE_SIZE)
#define TEST_WORDS 256
#define ERASED     0xFFFFFFFFu

static uint32_t buf[TEST_WORDS];

PROCESS(flash_test_process, "Flash test");
AUTOSTART_PROCESSES(&flash_test_process);

static int
matches(const uint32_t *expected, uint32_t addr, unsigned words)
{
  return rom_util_memcmp(expected, (const void *)addr, words * 4) == 0;
}

static int
erased(uint32_t addr, uint32_t bytes)
{
  const uint32_t *p = (const uint32_t *)addr;
  uint32_t i;

  for(i = 0; i < bytes / 4; i++) {
    if(p[i] != ERASED) {
      return 0;
    }
  }
  return 1;
}

static void
report(int ok, const char *what)
{
  printf("%s %s\n", ok ? "[OK]" : "[FAIL]", what);
}

PROCESS_THREAD(flash_test_process, ev, data)
{
  unsigned i;
  int32_t rc;

  PROCESS_BEGIN();

  printf("CC2538 FLASH TEST\n");
  printf("FLASH SIZE: 0x%lx\n", (unsigned long)rom_util_get_flash_size());

  /* 1. A buffer filled by ROM_memset, programmed into an erased page. */
  rc = rom_util_page_erase(TEST_ADDR, FLASH_PAGE_SIZE);
  rom_util_memset(buf, 0x5A, sizeof(buf));
  rc |= rom_util_program_flash(buf, TEST_ADDR, sizeof(buf));
  report(rc == 0 && matches(buf, TEST_ADDR, TEST_WORDS),
         "Data written with memset successfully");

  /* 2. A word pattern into the second page: every bit position and both
   * halves of each 64-bit flash location. */
  rc = rom_util_page_erase(TEST_ADDR + FLASH_PAGE_SIZE, FLASH_PAGE_SIZE);
  for(i = 0; i < TEST_WORDS; i++) {
    buf[i] = (i * 0x9E3779B9u) ^ (1u << (i % 32));
  }
  rc |= rom_util_program_flash(buf, TEST_ADDR + FLASH_PAGE_SIZE, sizeof(buf));
  report(rc == 0 && matches(buf, TEST_ADDR + FLASH_PAGE_SIZE, TEST_WORDS),
         "Data written successfully");

  /* 3. Erase the first page again; the second keeps its data. */
  rc = rom_util_page_erase(TEST_ADDR, FLASH_PAGE_SIZE);
  report(rc == 0 && erased(TEST_ADDR, FLASH_PAGE_SIZE) &&
         matches(buf, TEST_ADDR + FLASH_PAGE_SIZE, TEST_WORDS),
         "Data erased successfully");

  /* 4. One call erasing both pages. */
  rc = rom_util_page_erase(TEST_ADDR, 2 * FLASH_PAGE_SIZE);
  report(rc == 0 && erased(TEST_ADDR, 2 * FLASH_PAGE_SIZE),
         "Data erased successfully");

  printf("FLASH TEST DONE\n");

  PROCESS_END();
}
