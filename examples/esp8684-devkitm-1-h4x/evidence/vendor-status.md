# Vendor status and availability — ESP8684-DevKitM-1-H4X

Checked live on 2026-11-05 for the exact board SKU.

- Espressif's current [ESP8684-DevKitM-1 guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c2/esp8684-devkitm-1/user_guide.html)
  lists **ESP8684-DevKitM-1-H4X** as the 4 MB variant and separately marks
  **ESP8684-DevKitM-1-H4** as **NRND**. It documents the discrete RGB LED pin
  mapping: GPIO0 red, GPIO1 green, GPIO8 blue. The official [v1.1
  schematic](https://dl.espressif.com/dl/schematics/esp8684-devkitm-1-schematics_V1.1.pdf)
  shows a common-anode LED, so the three channels are active-low.
- Mouser's live [ESP8684-DevKitM-1-H4X listing](https://nz.mouser.com/en/ProductDetail/Espressif-Systems/ESP8684-DevKitM-1-H4X)
  labels the item a **New Product** and showed **35 in stock** with immediate
  dispatch when checked. This is the exact H4X orderable SKU, not the NRND H4
  revision or another board variant.

This round adds a board-level GPIO smoke only. The profile does not claim the
board's radio, USB, UART bridge, or other peripherals.
