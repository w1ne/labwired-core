# Required Source Documents (Arduino Nano 33 IoT)

## MCU Datasheet (authoritative)

1. Microchip SAM D21 / DA1 Family Datasheet — **DS40001882** (memory map, PM/GCLK, SERCOM USART, PORT):
   https://ww1.microchip.com/downloads/aemDocuments/documents/OTH/ProductDocuments/DataSheets/SAM_D21_DA1_Family_DataSheet_DS40001882.pdf

## Board Pinout / BSP

1. Arduino Nano 33 IoT pinout / product page:
   https://docs.arduino.cc/hardware/nano-33-iot/
2. ArduinoCore-samd Nano 33 IoT variant (D13=PA17, Serial1=SERCOM5 on PB22/PB23):
   https://github.com/arduino/ArduinoCore-samd/blob/master/variants/nano_33_iot/variant.cpp

## Address Cross-Check Only (not a source of truth)

1. CMSIS / SAM D21 device headers may be used to **cross-check** peripheral base addresses against DS40001882. Do not treat third-party simulator platform files as authoritative for LabWired models.
