/* LabWired - Firmware Simulation Platform
 * SPDX-License-Identifier: MIT
 * R7FA4M1AB: 256 KB flash @ 0x00000000, 32 KB SRAM @ 0x20000000.
 */

MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 256K
  RAM   : ORIGIN = 0x20000000, LENGTH = 32K
}
