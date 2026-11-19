/* FocalTech FT9001 as Renode describes it (platforms/cpus/focaltech_ft9001.repl):
 * flash0 2 MiB at 0x1000_0000, sram0 0x38000 bytes at 0x2000_0000. */

MEMORY
{
  FLASH : ORIGIN = 0x10000000, LENGTH = 2048K
  RAM   : ORIGIN = 0x20000000, LENGTH = 224K
}
