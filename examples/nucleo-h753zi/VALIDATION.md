# Validation — NUCLEO-H753ZI

The smoke is local simulator evidence for the committed narrow profile. It is
not a physical-board test and does not validate every STM32H753 peripheral.
The connected hosted verification catalog has no H753 target, so this package
does not claim a hosted `labwired_verify` verdict.

| Capability | Evidence / limit |
| :--- | :--- |
| Cortex-M7 firmware entry from the H753 flash address | Captured by the committed smoke run |
| H7 RCC enables for GPIOB, GPIOD, GPIOE, and USART3 | Written by firmware; these reset-clock gate paths are modeled |
| LD1 PB0, LD2 PE1, and LD3 PB14 | Smoke requires at least 16 GPIO edges on each pad |
| USART3 transmitter; PD8 AF7 declared for the default ST-LINK VCP route | Smoke requires the banner on the modeled USART3 console; the external ST-LINK USB bridge and electrical AF route are not simulated |
| Other RAM banks | Not mapped in this narrow profile; only DTCM is provided for the image |
| Ethernet, USB, FDCAN, ADC, DMA, timers, SPI, I2C, and remaining board signals | Not declared by this profile and not claimed |
| Physical H753 reset values and peripheral fidelity | Not checked against a physical board |

ST's UM2407 Rev 6 identifies the LED pads and default USART3 VCP wiring. The
committed UART/GPIO logs and machine-generated onboarding metrics are in
[`evidence/`](evidence/).
