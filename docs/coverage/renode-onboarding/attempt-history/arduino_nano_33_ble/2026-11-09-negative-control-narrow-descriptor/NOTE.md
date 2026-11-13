# Arduino Nano 33 BLE — negative control: the old narrowed descriptor

The new smoke firmware run against the row's previous chip profile (UARTE0 + P0
only), via a /tmp copy of the system pointing at
`git show HEAD:configs/chips/onboarding/arduino_nano_33_ble.yaml`.

Result: FAIL 1/6. The first silicon block reads FICR INFO.PART (0x10000100),
which the narrow profile does not map: precise BusFault → HardFault at PC
0x05BC, summary word never written (0xA5A5A5A5), no LED edges. The smoke
therefore depends on the included nrf52840 surface, not just UARTE0/P0.
