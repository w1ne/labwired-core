/* MIMXRT595-EVK (MIMXRT595S): XIP from the FlexSPI0 NOR at 0x08000000, data
 * in the on-chip SRAM (SDK evkmimxrt595 MIMXRT595Sxxxx_cm33_flash.ld).
 *
 * The boot ROM reads the FlexSPI configuration block (FCB, tag "FCFB") at
 * 0x08000400 and the vector table at 0x08001000, which is where
 * cortex-m-rt's FLASH region starts. RAM follows the SDK's m_data region
 * (0x20280000, 2.5 MiB); the lower SRAM partitions are left free as in the
 * SDK image.
 */

MEMORY
{
  FCB   : ORIGIN = 0x08000400, LENGTH = 0x200
  FLASH : ORIGIN = 0x08001000, LENGTH = 8188K
  RAM   : ORIGIN = 0x20280000, LENGTH = 2560K
}

SECTIONS
{
  .fcb ORIGIN(FCB) :
  {
    KEEP(*(.fcb.rt500));
  } > FCB
} INSERT BEFORE .vector_table;
