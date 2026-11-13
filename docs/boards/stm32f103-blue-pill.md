# STM32F103C8T6 Blue Pill

**Demand position:** STM32F103 ranks #3 at 17% of reported MCU use. The Blue
Pill is a board-level profile over the existing STM32F103C8 model.

The profile includes its reference active-low user LED on PC13 and the USART1
header path on PA9/PA10. The local smoke prints `BLUEPILL_F103C8_OK` and checks
PC13 output edges. It does not claim USB device behavior, a USB bootloader, an
onboard debugger, or physical board validation. Blue Pill clones can differ.

- [Pin map, Arduino example, and smoke command](../../examples/stm32f103-blue-pill/README.md)
- [Live ST lifecycle and retailer availability receipt](../../examples/stm32f103-blue-pill/vendor-status.md)
- [Committed local simulator evidence](../../examples/stm32f103-blue-pill/evidence/)

The STM32F103 chip descriptor also declares unexercised peripherals. Its USB
device and BKP windows are stubs, and WWDG is inert; this board profile does
not claim those paths. The exercised GPIOC and USART1 models are separate from
those gaps.
