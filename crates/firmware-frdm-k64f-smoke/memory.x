/* FRDM-K64F (MK64FN1M0VLL12): 1 MiB program flash @ 0x00000000,
 * 192 KiB SRAM_U @ 0x20000000 (SRAM_L at 0x1FFF0000 is not used by the smoke;
 * the vector table and stack live in SRAM_U). */

MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 1024K
  RAM   : ORIGIN = 0x20000000, LENGTH = 192K
}

/* Kinetis Flash Configuration Field (K64 RM §29.3.1): 16 bytes at 0x400 that
 * the part loads into FSEC/FOPT/FPROT at reset. The 16 + 240-entry vector
 * table ends exactly at 0x400, so without this section `.text` would start
 * there and Reset-handler opcodes would land in FSEC/FOPT — which can secure
 * (and, with MEEN=0b10, permanently lock) a real board. Pin the FCF at 0x400
 * and start `.text` after it. */
SECTIONS
{
  .flash_config 0x400 :
  {
    KEEP(*(.flash_config));
  } > FLASH
} INSERT AFTER .vector_table;

_stext = 0x410;

ASSERT(SIZEOF(.flash_config) == 0x10, "the Kinetis FCF at 0x400 must be exactly 16 bytes");
