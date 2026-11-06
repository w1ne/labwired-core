# Grounding sources

- ST [STM32G0B1RE product/datasheet](https://www.st.com/en/microcontrollers-microprocessors/stm32g0b1re.html): Cortex-M0+, 64 MHz maximum, 512 KiB flash, 144 KiB SRAM.
- ST [RM0444](https://www.st.com/resource/en/reference_manual/rm0444-stm32g0x1-advanced-armbased-32bit-mcus-stmicroelectronics.pdf): reset HSI16, RCC PLL/AHB/APB, TIM2, USART, GPIO and EXTI.
- ST [CMSIS G0B1 header](https://github.com/STMicroelectronics/cmsis-device-g0/blob/master/Include/stm32g0b1xx.h), audited blob `517d8b39dcbc9d87fe12b249607251d0ba35bf7a`: exact peripheral addresses, masks and IRQ numbers.
- ST [UM2324](https://www.st.com/resource/en/user_manual/um2324-stm32-nucleo64-boards-mb1360-stmicroelectronics.pdf): NUCLEO-G0B1RE/MB1360 board; PA2/3 VCP, PA5 LED, PC13 button. This rig uses USART1 PA9/10 for the PHY.
- TI [TIOL112 SLLSFJ1D](https://www.ti.com/lit/ds/symlink/tiol112.pdf), §§5, 6.7, 8.3.6: pinout, EN/CQ truth table, inverted receiver, 1.2 µs/0.3 µs propagation maxima and 4 µs typical enable/release times.
- [IO-Link Interface and System Specification v1.1.4, June 2024](https://io-link.com/fileadmin/user_upload/Downloads/Package_2024/IOL-Interface-Spec_10002_V114_Jun24.pdf): M-sequence layouts, checksum, A.3.4/A.3.5 timing and Tables A.7/A.8 startup recovery. Capability code 4 and internal enum TYPE_2_V=6 are distinct encodings. Table 102 and Table 53 T4/T6 define the ISDU acknowledgment interval.

The original 16 MHz reference ELF and the separate 64 MHz protocol ELF must be
identified by source commit, compiler and SHA256 in any external evidence report.
