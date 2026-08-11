# External Components

The base **Arduino Nano 33 BLE** (ABX00030) has no soldered environmental sensors
in the product system. Onboard modelling is limited to:

| Item | How it appears |
|------|----------------|
| User LEDs (builtin + RGB + power) | `board_io` in `configs/systems/arduino-nano-33-ble.yaml` |
| USB serial | nRF52840 USB device (firmware path); not a separate external_device |
| 2.4 GHz antenna | Electrical feature on the manufacturer part (`antenna`) |

## USB + BLE

| Feature | Where |
|---------|--------|
| USB device | Chip peripheral `usbd` (nRF52840 USBD model) |
| BLE radio | Chip peripheral `radio` |
| Antenna / crystal | Manufacturer electrical on ABX00030 |

SoftDevice / NINA-style blobs are firmware packaging, not board_io.

## Not this board

**Arduino Nano 33 BLE Sense** (ABX00031) is product id `arduino-nano-33-ble-sense`
with Wire1 sensor shells. Sense **Rev2** (BMI270 suite) is still a future id.

## Canvas / kit attach

Any I²C/SPI/UART sensor may be placed on the Playground canvas and wired to
the Nano's header pins (Arduino D*/A* aliases or Nordic P0.xx / P1.xx).
