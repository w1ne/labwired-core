/* Direct NPCX9 execution profile: mapped ROM at zero and data RAM. */
MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 64K
  RAM   : ORIGIN = 0x200C0000, LENGTH = 0x17800
}
