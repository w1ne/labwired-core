# S140-disabled test fixture

`HalloweenGhostGame.elf` is the author's Spooky Keychain Arcade application,
debug-stripped, with the symbol table kept so tests can resolve `setup` and
`loop`.

- Source: Gokux, "Spooky Keychain Arcade", https://www.hackster.io/Gokux/spooky-keychain-arcade-51c1bf (CC BY).
  Code: https://github.com/gokuxmaker/-Keychain-Arcade commit f2854c5.
- Built with Seeed nRF52 Arduino core 1.1.13, FQBN `Seeeduino:nrf52:xiaonRF52840Plus`.
- This file is the application image only. It is not a Nordic SoftDevice, MBR,
  or bootloader binary. The ELF PT_LOAD is 64 KiB-aligned at 0x20000 and
  includes the ELF header in that padding; the loader used by the tests keeps
  only bytes at and above the S140 application base 0x27000.

Do not add Nordic SoftDevice or MBR images to this directory.
