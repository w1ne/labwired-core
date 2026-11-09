# Vendor status and availability — ESP8684-DevKitM-1-H4X

Checked live on 2026-11-09 for the exact board SKU.

- Espressif's current [ESP8684-DevKitM-1 v1.1 guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c2/esp8684-devkitm-1/user_guide.html)
  lists **ESP8684-DevKitM-1-H4X** as the 4 MB variant and separately marks
  **ESP8684-DevKitM-1-H4** as **NRND**. It documents the discrete RGB LED pin
  mapping: GPIO0 red, GPIO1 green, GPIO8 blue, and GPIO2 as ADC1_CH2. The official [v1.1
  schematic](https://dl.espressif.com/dl/schematics/esp8684-devkitm-1-schematics_V1.1.pdf)
  shows a common-anode LED, so the three channels are active-low.
- Espressif's live [board page](https://www.espressif.com/en/dev-board/esp8684-devkitm-1-en)
  links the H4X SKU to DigiKey, Mouser, AliExpress, and Amazon.
- Mouser's live [ESP8684-DevKitM-1-H4X listing](https://www.mouser.ca/en/ProductDetail/Espressif-Systems/ESP8684-DevKitM-1-H4X)
  showed **48 in stock** and **48 can ship immediately**. This is the exact
  H4X orderable SKU, not the NRND H4 revision or another board variant.

This round extends the existing board smoke with ADC1_CH2 on GPIO2. The profile
does not claim the board's radio, USB, UART bridge, boot buttons, or other
peripherals.
