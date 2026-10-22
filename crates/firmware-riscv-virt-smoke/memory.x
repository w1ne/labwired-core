/* memory.x for firmware-riscv-virt-smoke
 *
 * The virt machine has one flat RAM at 0x80000000. The descriptor splits it
 * into a 1 MiB code window (flash) and a 7 MiB data/stack window (ram), so
 * the loader places text and data where the machine maps them.
 */
MEMORY
{
  FLASH : ORIGIN = 0x80000000, LENGTH = 1M
  RAM   : ORIGIN = 0x80100000, LENGTH = 7M
}

REGION_ALIAS("REGION_TEXT", FLASH);
REGION_ALIAS("REGION_RODATA", FLASH);
REGION_ALIAS("REGION_DATA", RAM);
REGION_ALIAS("REGION_BSS", RAM);
REGION_ALIAS("REGION_HEAP", RAM);
REGION_ALIAS("REGION_STACK", RAM);
