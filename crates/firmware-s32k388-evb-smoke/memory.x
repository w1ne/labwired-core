/* S32K388EVB-Q289 (S32K388): 8 MiB program flash @ 0x00400000, SRAM_0-2
 * (3 x 256 KiB) @ 0x20400000 (S32DS RTD linker_flash_s32k388.ld).
 *
 * The first 4 KiB page of program flash holds the boot header (IVT, marker
 * 0x5AA55AA5) that the boot ROM reads; its CM7_0 slot points at the vector
 * table at 0x00401000, which is where cortex-m-rt's FLASH region starts. The
 * last 176 KiB of program flash belong to the HSE firmware and are left out.
 */

MEMORY
{
  IVT   : ORIGIN = 0x00400000, LENGTH = 4K
  FLASH : ORIGIN = 0x00401000, LENGTH = 8012K
  RAM   : ORIGIN = 0x20400000, LENGTH = 768K
}

SECTIONS
{
  .ivt ORIGIN(IVT) :
  {
    KEEP(*(.ivt.s32k3));
  } > IVT
} INSERT BEFORE .vector_table;
