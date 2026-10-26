# External Components — STM32F072B-DISCO

None. The smoke firmware exercises the SoC surface only: CPU, memory, RCC,
GPIO and USART1. The simulator captures the USART1 byte stream as its debug
console; that output sink does not model a board USB serial bridge. Physical
serial output requires an external 3.3 V USB-UART adapter connected to PA9,
PA10 and ground, with USART1 pinctrl configured.

The board carries a joystick and a touch sensor; neither is modelled or
attached in this example:

| Component | Board wiring | State |
| :--- | :--- | :--- |
| 4-direction joystick with selection | PC0–PC4 (GPIO) | not modelled |
| Linear touch sensor | TSC block | not modelled (the TSC is a declared stub) |
| ST-LINK/V2 | SWD | debug/programming only; ST-LINK/V2 on this board is not a VCP |
| USART1 console | PA9/PA10 | simulator output capture only; no physical USB bridge modeled |

When any of these becomes a milestone, it goes here with its fidelity tier and
descriptor path, and the system manifest grows the matching
`external_devices` entry.
