# Required Documents — LPC1114FN28/102

Authoritative sources used by this example (retrieved 2026-10-31):

| Doc | Title | Used for |
| :--- | :--- | :--- |
| NXP CMSIS-SVD `LPC11xx_v6a.svd` ([cmsis-svd-data](https://github.com/cmsis-svd/cmsis-svd-data/blob/main/data/NXP/LPC11xx_v6a.svd)) | LPC111x / LPC11Cxx device description, version 6 | Peripheral bases, IRQ numbers, every SYSCON/IOCON/UART/GPIO register offset, access and reset value, the `SYSAHBCLKCTRL`/`SYSPLLSTAT`/`PDRUNCFG` bit fields |
| NXP CMSIS `system_LPC11xx.c` (as vendored in mbed-os 5.15 `targets/TARGET_NXP/TARGET_LPC11XX_11CXX/TARGET_LPC11XX/device/`) | Cortex-M0 device system source | The `SystemInit` clock sequence (UEN handshakes, PLL power-up, `SYSPLLSTAT.LOCK` wait) and the mbed values (`SYSPLLCTRL` `0x23`, `MAINCLKSEL` 3) |
| mbed-os 5.15 `TARGET_LPC11XX_11CXX/serial_api.c`, `TARGET_LPC11XX/PinNames.h`, `LPC1114.ld` | mbed LPC1114FN28 target | UART bring-up order (`DLAB`, `DLM`/`DLL`, `FDR`, `FCR`), console on PIO1_7/PIO1_6, LED1 on PIO0_7, 32 KiB flash / 4 KiB SRAM |
| lpc21isp `lpcprog.c` ([capiman/lpc21isp](https://github.com/capiman/lpc21isp)) | NXP ISP programmer part table | `DEVICE_ID` `0x1A40902B` for LPC1114…/102 (`0x0A40902B` on the earlier silicon revision); memory sizes per part |

The v6 SVD predates the DIP-28 /102 parts, so `DEVICE_ID` comes from lpc21isp
alone. UM10398 (the LPC111x user manual) and the LPC1110/11/12/13/14/15
datasheet were not retrievable from nxp.com during this work; their values are
taken from the SVD instead.

Open items:

- The `SYSAHBCLKCTRL` reset value is the SVD's `0x85F`. It was not checked
  against UM10398; the smoke only ORs bits into it.
- The PLL lock time is not modelled (lock follows the power-up write by one
  cycle).
