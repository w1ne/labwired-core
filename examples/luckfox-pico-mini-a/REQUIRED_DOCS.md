# Required documents and provenance

References checked on **2026-10-11**. Preserve revision/hash receipts when implementing hardware or register models; current web links can change.

| Reference | Purpose / status |
|---|---|
| [Exact Mini A product](https://www.luckfox.com/Luckfox-Pico-Mini-A?ci=536) | User-selected board identity; scripted access returned 403 during this pass |
| [Official variant overview](https://wiki.luckfox.com/Luckfox-Pico/Luckfox-Pico-RV1103/) | RV1103/Cortex-A7, memory and board-population comparison |
| [Mini A MicroSD flashing guide](https://wiki.luckfox.com/Luckfox-Pico-Plus-Mini/Flash-image/) | Boot medium and correct image selection; not executed |
| [UART guide](https://wiki.luckfox.com/Luckfox-Pico-Plus-Mini/UART/) / [I2C guide](https://wiki.luckfox.com/Luckfox-Pico-Plus-Mini/I2C/) | Pinmux and physical Linux peripheral setup |
| [Device-tree guide](https://wiki.luckfox.com/Luckfox-Pico-Pro-Max/Device-Tree/) | Mini A-specific `rv1103g-luckfox-pico-mini-a.dts` |
| [Official SDK](https://github.com/LuckfoxTECH/luckfox-pico) | Board configurations, drivers and reproducible vendor firmware; not built here |
| [SDK DDR base](https://github.com/LuckfoxTECH/luckfox-pico/blob/824b817f889c2cbff1d48fcdb18ab494a68f69d1/sysdrv/source/uboot/u-boot/include/configs/rv1106_common.h) / [shared DTS](https://github.com/LuckfoxTECH/luckfox-pico/blob/824b817f889c2cbff1d48fcdb18ab494a68f69d1/sysdrv/source/kernel/arch/arm/boot/dts/rv1106.dtsi) | Declared DDR physical base 0 and shared SRAM 0xff6c0000/256 KiB; zero-filled memory is not DDR initialization |
| [Official hardware/documentation repository](https://github.com/LuckfoxTECH/Luckfox-Pico-docs) | Pinned source of schematic, datasheet and CAD; revision in manifest |
| [Cached Mini schematic](../../docs/assets/luckfox-pico-mini-a/schematic.pdf) | Power nets, connector mapping and Mini population; shared design needs Mini A population check |
| [Cached RV1103 datasheet](../../docs/assets/luckfox-pico-mini-a/rv1103-datasheet.pdf) | Silicon characteristics; not a substitute for register-level reference manual |
| [Official CAD archive](../../docs/assets/luckfox-pico-mini-a/vendor-cad.zip) | Contains `Luckfox-Pico-CAD/Luckfox Pico Mini.DWG`; geometry and scale unverified |
| [ESP32-C3-MINI-1/1U datasheet](https://www.espressif.com/sites/default/files/documentation/esp32-c3-mini-1_datasheet_en.pdf) | External antenna, module pads, power and boot straps |
| [ESP-AT wiring](https://docs.espressif.com/projects/esp-at/en/latest/esp32c3/Get_Started/Hardware_connection.html) / [release compatibility](https://docs.espressif.com/projects/esp-at/en/latest/esp32c3/AT_Binary_Lists/esp_at_binaries.html) | Command UART mapping and actual firmware selection |
| [ADXL345 datasheet](https://www.analog.com/media/en/technical-documentation/data-sheets/adxl345.pdf) | Supply, I2C address, identity, registers and sensor settings |
| [BQ24074](https://www.ti.com/product/BQ24074), [TPS61023](https://www.ti.com/product/TPS61023), [TPS62160](https://www.ti.com/product/TPS62160) | Current datasheets/reference circuits for selected power devices |
| Actual pack datasheet | **Missing:** exact battery part, charge/discharge limits and protection thresholds |
| RV1103 register-level TRM | **Missing:** audited reset/boot, memory map, clocks, IRQs, UART/I2C/SD controller descriptions for simulation |
| Carrier schematic / BOM / footprint / enclosure | **Missing:** manufactured design and physical review evidence |

Official product, pin and dimension images are retained as original bytes with alt text on the [board page](../../docs/boards/luckfox-pico-mini-a.md). See the [machine-readable source manifest](../../docs/assets/luckfox-pico-mini-a/sources.json) and [asset provenance notes](../../docs/assets/luckfox-pico-mini-a/README.md).

The [community KiCad repository](https://github.com/lightningspore/seedsigner-luckfox-pico/tree/main/hardware-kicad) is a possible comparison reference. No explicit redistribution license was established; no community symbol, footprint or 3D file is imported into this package or treated as official vendor data.

CPU/memory declaration source revision: `824b817f889c2cbff1d48fcdb18ab494a68f69d1`. The pinned U-Boot header sets DDR base zero; the pinned shared DTS declares SRAM `0xff6c0000`, length `0x40000`. This does not specify board reset or initialize DDR.
