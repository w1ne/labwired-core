# External Components — STM32F072B-DISCO

None. The smoke firmware exercises the SoC surface only: CPU, memory, RCC,
GPIO and USART1. The UART is the modelled console and needs no external
device — the engine captures its byte stream directly.

The board carries a joystick and a touch sensor; neither is modelled or
attached in this example:

| Component | Board wiring | State |
| :--- | :--- | :--- |
| 4-direction joystick with selection | PC0–PC4 (GPIO) | not modelled |
| Linear touch sensor | TSC block | not modelled (the TSC is a declared stub) |
| ST-LINK/V2-1 VCP | USART1 PA9/PA10 | modelled as the console sink (no host bridge modelled) |

When any of these becomes a milestone, it goes here with its fidelity tier and
descriptor path, and the system manifest grows the matching
`external_devices` entry.
