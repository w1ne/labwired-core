# External components — NUCLEO-F103RB

The onboarding smoke uses no external simulated components. It exercises the
on-chip GPIOA and USART2 paths, with board-level connections to LD2 and the
ST-LINK virtual COM port. B1 is described in `board_io` but the smoke does not
read it.

`external_devices: []` in the system manifest is intentional. Add sensors,
displays, or other bus devices only after their pin wiring and the associated
F103 peripheral path are documented and validated.
