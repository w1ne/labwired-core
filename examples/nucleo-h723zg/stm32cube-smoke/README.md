# NUCLEO-H723ZG STM32Cube startup smoke

`main.c` uses the standard PlatformIO STM32Cube/CMSIS startup and configures
USART3 on PD8/PD9 and LD1 on PB0 at reset HSI clock. `SystemInit` runs before
main and accesses FLASH_ACR and disables FMC bank 1. The committed fixture was
built with PlatformIO `ststm32@19.7.1`, `nucleo_h723zg`, STM32Cube; the
compiler was GCC Arm Embedded 7.2.1 and framework-stm32cubeh7 was 1.12.1.

```sh
mkdir -p /tmp/h723-pio/src
cp examples/nucleo-h723zg/stm32cube-smoke/platformio.ini /tmp/h723-pio/
cp examples/nucleo-h723zg/stm32cube-smoke/main.c /tmp/h723-pio/src/
pio run -d /tmp/h723-pio
sha256sum /tmp/h723-pio/.pio/build/nucleo_h723zg/firmware.elf
```

The fixture SHA-256 is `33906d31f99f66a95339fbb7ecdeedcf5ab7f5c8bbc7b5887f0a1419f102d79d`.
The smoke checks the marker, PB0 edges and normal stop in the simulator. It
does not validate H7 flash programming, FMC operation or physical silicon.

The linked `SystemInit` and startup code are STMicroelectronics CMSIS device
code. PlatformIO's `framework-stm32cubeh7` 1.12.1 matches the ST CMSIS device
repository revision `e8d40ae6e2fa06afe5b46d24756f141b363342a7` referenced by
STM32CubeH7 v1.12.1: normalized source SHA-256 is
`2ea5c2bd1c55e454fcbe26612c0b2a57f321fffb857ccd92844682288dd62518` for
`system_stm32h7xx.c` and
`8460ce955e1ab8ccd9bbf22b62351058035ed0f2c8020b3c91c09d84cc48f416` for
`startup_stm32h723xx.s`. ST's component is Apache-2.0; its exact revision
license is included as `STM32-CMSIS-APACHE-2.0.txt`. The smoke `main.c` is
project-authored under the repository MIT license. A separate PlatformIO project
rebuilt the fixture with the same local package cache and reproduced its hash;
no hardware capture is implied.
The ELF also links GNU Arm Embedded's newlib `__libc_init_array` and CRT
objects. The toolchain's full bundled notices are reproduced with whitespace normalized in
`GNU-ARM-TOOLCHAIN-LICENSES.txt`; its SHA-256 is recorded below.
`NEWLIB-INIT-NOTICE.txt` preserves the copyright and permission notice for
newlib's `__libc_init_array` source. Original bundled license file SHA-256: `2da9093efc52295c36b56db7a2c58d08d73b3406fa7c4db8e310d07604f3b75a`; normalized copy SHA-256: `5eb61ab852735772bbb73faf515c0a79701a8f3969481a1e4d9516962fd9d8df`.
The separate notice matches the `newlib-2_5_0` source copy of
[`newlib/libc/misc/init.c`](https://github.com/mirror/newlib-cygwin/blob/newlib-2_5_0/newlib/libc/misc/init.c);
the PlatformIO toolchain package does not state the exact newlib source revision.
