# CH32V003 WCH Arduino L0 fixture

`ch32v003f4p6-arduino-l0.elf` is the Arduino matrix L0 sketch
(`validation/arduino-matrix/sketches/L0_serial_boot/src/main.ino`, unchanged)
built for the CH32V003F4P6 with WCH's official Arduino core. It is used by
`crates/core/tests/ch32v003_arduino.rs`.

Build inputs (PlatformIO 6.2.0):

| package | version |
|---|---|
| platform `ch32v` (Community-PIO-CH32V/platform-ch32v) | 1.1.0, `f26b06a375c5a8b4dd58af5265538e88aa2a68c7` |
| `framework-arduino-openwch-ch32` (openwch/arduino_core_ch32) | `3820604346d0a9f0ee9a72cab8b7583e488e6674` |
| `toolchain-riscv` (WCH GCC 12.2.0, `riscv-wch-elf`) | 1.120200.220829, `fde267f` |

```ini
[env:ch32v003]
platform = https://github.com/Community-PIO-CH32V/platform-ch32v.git
board = genericCH32V003F4P6
framework = arduino
```

`pio run` builds with `-march=rv32ecxw -mabi=ilp32e`. sha256 of the ELF:
`8f745f373e9a48ad900b036b727a0dbcb91ffa1e0c565dd2b0cae1c520a817a0`.

What the image exercises before it prints `LW_L0_OK`:

* `startup_ch32v00x.S`: `mstatus = 0x80`, `csrw 0x804, 3` (INTSYSCR: HWSTKEN |
  INESTEN), `mtvec = _start | 3` (vectored, absolute-address table), `mret`
  into `main`.
* `SystemInit`: FLASH `ACTLR` latency and the RCC switch to 48 MHz.
* `main`: STK `CMP = 0xBB7F`, `CTLR = 0xF` (STE | STIE | STCLK | STRE), then
  `IPRIOR[12]` and `IENR[0] = 1 << 12` (`SysTicK_IRQn`).
* `Serial.begin` (USART1 on PD5), then `delay(1)`, which waits for
  `SysTick_Handler` (`interrupt("WCH-Interrupt-fast")`, relies on HPE) to bump
  `msTick`.
* WCH XW compressed loads/stores throughout (the first is `c.lhusp a1, 24(sp)`
  at 0x736).

The stock L2 sketch is not a fixture: the CH32V003F4 variant defines
`LED_BUILTIN` as `PNUM_NOT_DEFINED`, and GCC compiles
`digitalWrite(LED_BUILTIN, ...)` to a trap (`sw zero, 16(zero); ebreak`).
L2 needs a board LED pin passed at build time.

The upstream Arduino core repository ships no LICENSE file; its sources carry
the WCH copyright header.
