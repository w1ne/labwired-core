# Required Source Documents (Pico 2 / RP2350)

## MCU Reference + Register Maps

1. RP2350 Datasheet (memory map, peripherals, dual M33):
   https://datasheets.raspberrypi.com/rp2350/rp2350-datasheet.pdf
2. pico-sdk `hardware_regs` for RP2350 (addressmap.h, intctrl.h, sysinfo.h) —
   authoritative bases for the moved low-APB map (CLOCKS/RESETS/XOSC/PLLs/UART
   etc. differ from RP2040):
   https://github.com/raspberrypi/pico-sdk/tree/master/src/rp2350/hardware_regs
3. CHIP_ID = `0x30004927` (sysinfo.h / RP2350 datasheet SYSINFO).
4. RP2350 SDK IO_BANK0 definitions, including `GPIO_FUNC_SIO = 5`:
   https://github.com/raspberrypi/pico-sdk/blob/master/src/rp2350/hardware_structs/include/hardware/structs/io_bank0.h
5. Raspberry Pi Pico 2 datasheet §§3.1, 5.2, and 5.3 (header pinout, GP26–GP28
   ADC-capable pads, GP29's VSYS monitor use, and the ADC reference/ground):
   https://datasheets.raspberrypi.com/pico/pico-2-datasheet.pdf
6. Raspberry Pi pico-sdk ADC API and RP2350 register definitions (ADC channel
   selection and the RP2350 ADC register map):
   https://github.com/raspberrypi/pico-sdk/tree/master/src/rp2_common/hardware_adc
   https://github.com/raspberrypi/pico-sdk/blob/master/src/rp2350/hardware_regs/include/hardware/regs/adc.h

## Board

1. Raspberry Pi Pico 2 product brief / pinout (GP25 user LED):
   https://www.raspberrypi.com/products/raspberry-pi-pico-2/
