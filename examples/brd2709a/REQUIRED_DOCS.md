# Required Source Documents (BRD2709A / EFR32MG26)

## MCU (CMSIS Device Headers — simplicity_sdk, tag sisdk-2025.6)

Silicon Labs publishes **no SVD** for the EFR32MG26 family; the CMSIS headers
in the Simplicity SDK are the authoritative register source.

1. Device header (memory map, base addresses, IRQ numbers):
   https://raw.githubusercontent.com/SiliconLabs/simplicity_sdk/sisdk-2025.6/platform/Device/SiliconLabs/EFR32MG26/Include/efr32mg26b510f3200im48.h
2. USART register map (`USART_TypeDef`: EN@0x04, STATUS@0x18, TXDATA@0x38, RXDATA@0x24):
   https://raw.githubusercontent.com/SiliconLabs/simplicity_sdk/sisdk-2025.6/platform/Device/SiliconLabs/EFR32MG26/Include/efr32mg26_usart.h
3. GPIO block + port structs (`GPIO_TypeDef` P[4] at +0x30, port stride 0x30):
   https://raw.githubusercontent.com/SiliconLabs/simplicity_sdk/sisdk-2025.6/platform/Device/SiliconLabs/EFR32MG26/Include/efr32mg26_gpio.h
   https://raw.githubusercontent.com/SiliconLabs/simplicity_sdk/sisdk-2025.6/platform/Device/SiliconLabs/EFR32MG26/Include/efr32mg26_gpio_port.h
4. Product page (datasheet, reference manual):
   https://www.silabs.com/wireless/zigbee/efr32mg26-series-2-socs

## Board

1. UG594 — BRD2709A User's Guide (VCOM = USART1 on PB02/PB03, LED0/1 =
   PC08/PC09, BTN0/1 = PB00/PB01):
   https://www.silabs.com/documents/public/user-guides/ug594-brd2709a-user-guide.pdf

## Silicon identification (physical board)

1. probe-rs SWD against the on-board J-Link OB (1366:0105): Energy Micro DP
   part 0x1013, CPUID PARTNO = Cortex-M33.
