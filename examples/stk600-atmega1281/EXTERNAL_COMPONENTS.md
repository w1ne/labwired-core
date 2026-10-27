# External Components — ATmega1281 on STK600

The simulator attaches logical LED0 and SW0 board-I/O contacts to PB5 and PB0, respectively.
These describe the explicit Playground demo harness; they do not simulate analog LED drive,
button bounce, jumper-card electrical behavior, or the STK600 programmer.

USART0 is captured at the simulated MCU boundary. There is no simulated USB or RS-232 bridge
and no assertion that a physical STK600 serial connector is routed to the Playground output.
