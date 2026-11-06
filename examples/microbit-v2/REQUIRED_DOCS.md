# Required Source Documents (micro:bit v2 / nRF52833)

## MCU Datasheet (authoritative)

1. Nordic nRF52833 Product Specification — memory map, GPIO, UARTE, clock, IRQ
   numbering:
   https://docs.nordicsemi.com/bundle/nRF52833-PS/resource/nRF52833_PS_v1.7.pdf
2. Nordic nrfx MDK CMSIS-SVD `nrf52833.svd` — peripheral base addresses and NVIC
   IRQ numbers (cross-check only; the Product Specification is authoritative):
   https://github.com/NordicSemiconductor/nrfx/blob/master/mdk/nrf52833.svd

## Board Pinout / Documentation

1. micro:bit v2 revision hardware page (target MCU, 512 KB flash / 128 KB RAM,
   interface MCU, 5x5 charlieplexed matrix):
   https://tech.microbit.org/hardware/2-0-revision/
2. micro:bit nRF52833 pin map (UART_INT_RX = P0.06, UART_INT_TX = P1.08,
   BTN_A = P0.14, BTN_B = P0.23):
   https://tech.microbit.org/hardware/schematic/
3. micro:bit V2 edge connector pinout (P0/AIN0 = P0.02, P1/AIN1 = P0.03,
   P2/AIN2 = P0.04; the three rings have fixed 10 MΩ pull-ups):
   https://tech.microbit.org/hardware/edge-connector/
4. micro:bit interface documentation (UART is bridged through the interface MCU
   to USB CDC; labels are from the interface chip's perspective):
   https://github.com/microbit-foundation/dev-docs/blob/master/software/interface.md
5. codal-microbit-v2 pin definitions (`MICROBIT_PIN_UART_TX` = P0.06):
   https://github.com/lancaster-university/codal-microbit-v2

## Address Cross-Check Only (not a source of truth)

1. Third-party simulator platform files (Renode, Zephyr DTS stubs) may be used
   to cross-check peripheral offsets. Do not treat them as authoritative for
   LabWired models.
