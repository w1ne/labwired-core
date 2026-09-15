# Required Source Documents (Arduino Uno R3)

## MCU

1. ATmega48A/PA/88A/PA/168A/PA/328/P datasheet (memory map, I/O registers, ADC,
   USART0, SPI, TWI status codes):
   https://ww1.microchip.com/downloads/aemDocuments/documents/MCU08/ProductDocuments/DataSheets/ATmega48A-PA-88A-PA-168A-PA-328-P-DS-DS40002061B.pdf
2. ATmega328P automotive datasheet (the document the parts catalog cites):
   https://ww1.microchip.com/downloads/en/DeviceDoc/Atmel-7810-Automotive-Microcontrollers-ATmega328P_Datasheet.pdf

Register facts the twin depends on: PINB/DDRB/PORTB at 0x23-0x25,
PINC/DDRC/PORTC at 0x26-0x28, PIND/DDRD/PORTD at 0x29-0x2B; ADCL/ADCH/ADCSRA/ADMUX
at 0x78/0x79/0x7A/0x7C (ADLAR = ADMUX bit 5, REFS = bits 7:6, MUX 0x0E = 1.1 V
bandgap); UDR0 at 0xC6; SPCR/SPSR/SPDR at 0x4C-0x4E; TWBR..TWCR at 0xB8-0xBC.

## Board

1. Arduino UNO R3 product documentation:
   https://docs.arduino.cc/hardware/uno-rev3/
2. A000066 datasheet (component list: SPX1117M3-L-5 regulator, ATmega16U2 USB
   bridge, LMV358 op-amp, two 47 µF 25 V capacitors, two 6-pin ICSP headers):
   https://docs.arduino.cc/resources/datasheets/A000066-datasheet.pdf
3. Schematic:
   https://docs.arduino.cc/resources/schematics/A000066-schematics.pdf
4. Full pinout:
   https://docs.arduino.cc/resources/pinouts/A000066-full-pinout.pdf
5. Tech specs (68.6 × 53.4 mm, 25 g, 7-12 V input, 20 mA per I/O pin, 16 MHz on
   both processors), from Arduino's docs-content repository:
   https://github.com/arduino/docs-content/blob/main/content/hardware/02.uno/boards/uno-rev3/tech-specs.yml
6. Arduino core pin table `variants/standard/pins_arduino.h` (PWM on 3/5/6/9/10/11,
   SPI SS/MOSI/MISO/SCK = 10/11/12/13, SDA/SCL = A4/A5, `LED_BUILTIN` = 13, six
   analog inputs):
   https://github.com/arduino/ArduinoCore-avr/blob/master/variants/standard/pins_arduino.h
7. PlatformIO board `uno`:
   https://docs.platformio.org/en/latest/boards/atmelavr/uno.html
8. KiCad footprint `Module:Arduino_UNO_R3` (pad 1-14 POWER and ANALOG IN, 15-32
   D0 back to SCL; confirms header positions and the 0.16 in D7-D8 gap):
   https://gitlab.com/kicad/libraries/kicad-footprints/-/blob/master/Module.pretty/Arduino_UNO_R3.kicad_mod

9. Board design files (Eagle, CC BY-SA 4.0), source of the layout in `images/board.svg`:
   https://github.com/arduino/docs-content/blob/main/content/hardware/02.uno/boards/uno-rev3/downloads/A000066-cad-files.zip

## Reference photograph

Arduino's A000066 front photograph, used to place parts in the drawings and in
the Playground part (reference only, not redistributed):
https://store.arduino.cc/products/arduino-uno-rev3
