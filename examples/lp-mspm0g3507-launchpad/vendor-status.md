# Vendor and buyability check — LP-MSPM0G3507

Checked 2026-11-17.

- TI's live [MSPM0G3507 product page](https://www.ti.com/product/MSPM0G3507)
  marks the MCU **ACTIVE** and lists 80-MHz Arm Cortex-M0+, 128 KB flash,
  and 32 KB SRAM.
- TI's [LP-MSPM0G3507 product page](https://www.ti.com/tool/LP-MSPM0G3507)
  identifies the board and its onboard XDS110 probe. TI's order/inventory text
  is inconsistent in the page rendering (the page includes both “In stock” and
  “Out of stock on TI.com”), so this evidence does not rely on TI inventory.
- [LionCircuits' LP-MSPM0G3507 listing](https://www.lioncircuits.com/parts/LP-MSPM0G3507)
  showed **In Stock**, “ships in 7–10 days,” and ACTIVE lifecycle status in the
  page checked today. DigiKey's exact product page showed 0 in stock and one
  expected 24-Nov-2026, so current distributor availability is mixed.
- TI's [Rev. E LaunchPad User's Guide](https://www.ti.com/lit/ug/slau873e/slau873e.pdf)
  says J21 connects PA10 to the XDS UART and J22 connects PA11 to the XDS UART;
  both default to the XDS_UART function. Its schematic names PA10 UART0-TX and
  PA11 UART0-RX. The guide was revised September 2026.

The MCU is active and the board has a current buyable distributor listing. The
conflicting DigiKey/TI inventory results are preserved here rather than hidden.
