# Required Documents — ATmega1281

| Source | Used for |
| :--- | :--- |
| [Microchip DS40002211A](https://ww1.microchip.com/downloads/aemDocuments/documents/OTH/ProductDocuments/DataSheets/ATmega640-1280-1281-2560-2561-Datasheet-DS40002211A.pdf) | Figure 1-3 package pinout; Table 2-1 memory/peripheral geometry; §8 memory map; §31.1 I/O voltage thresholds; instruction availability for the family. |
| [Microchip ATmega1281 product page](https://www.microchip.com/en-us/product/ATmega1281) | 128 KB flash, 8 KB SRAM, 64-pin part metadata and package options. |
| `crates/core/src/cpu/avr.rs` | `AvrModel::Atmega1281`, 16-bit PC/return address, and the public instruction/peripheral behavior. |
| `configs/chips/atmega1281.yaml` | Exact 64-pin descriptor, bonded GPIO/ADC map, and declared peripherals. |

The datasheet defines the silicon facts. The simulator boundary is limited to
behavior exercised by the integration test and example CLI smoke.
