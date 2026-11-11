# Required Source Documents (Arduino Uno R4 Minima)

## MCU Datasheet (authoritative)

1. Renesas RA4M1 Group User's Manual: Hardware — **R01UH0887** (memory map, SYSTEM HOCO/OSCSF, PORT PCNTR, SCI, RIIC/IIC1 registers and PFS selection):
   https://www.renesas.com/en/document/mah/ra4m1-group-users-manual-hardware

## Board Pinout / BSP

1. Arduino Uno R4 Minima product / docs:
   https://docs.arduino.cc/hardware/uno-r4-minima/
2. Arduino UNO R4 Minima full pinout (A4/SDA=P101, A5/SCL=P100):
   https://docs.arduino.cc/5e218c7ed3622cbd09ab25357c345b84/ABX00080-full-pinout.pdf
3. ArduinoCore-renesas MINIMA variant (D13=P111, Serial SCI2 on P301/P302):
   https://github.com/arduino/ArduinoCore-renesas/tree/main/variants/MINIMA
4. Current product listing / buyability check:
   https://store.arduino.cc/products/uno-r4-minima

## Address Cross-Check Only (not a source of truth)

1. FSP / `ra4m1-fsp-pac` / `R7FA4M1AB.h` may be used to **cross-check** peripheral base addresses (SCI2 `0x40070040`, IIC1 `0x40053100`, SYSTEM `0x4001E000`, PORT1 `0x40040020`, USBFS `0x40090000`). Do not treat FSP driver semantics as authoritative for LabWired models.
