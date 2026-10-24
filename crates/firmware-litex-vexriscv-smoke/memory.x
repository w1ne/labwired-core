/* memory.x for firmware-litex-vexriscv-smoke
 *
 * The LiteX VexRiscv SoC's integrated SRAM is 256 KB at 0x10000000 (see
 * configs/chips/litex-vexriscv.yaml). Code/rodata take the first 192 KB and
 * the stack/data region the last 64 KB, so the stack top is the SRAM top —
 * the same value the RISC-V builder seeds SP with before riscv-rt's _start
 * installs its own stack pointer.
 */
MEMORY
{
  FLASH : ORIGIN = 0x10000000, LENGTH = 192K
  RAM   : ORIGIN = 0x10030000, LENGTH = 64K
}

REGION_ALIAS("REGION_TEXT", FLASH);
REGION_ALIAS("REGION_RODATA", FLASH);
REGION_ALIAS("REGION_DATA", RAM);
REGION_ALIAS("REGION_BSS", RAM);
REGION_ALIAS("REGION_HEAP", RAM);
REGION_ALIAS("REGION_STACK", RAM);
