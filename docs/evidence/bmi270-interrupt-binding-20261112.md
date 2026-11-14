# BMI270 SPI interrupt delivery

The native SPI kit now binds `INT1` and `INT2` output roles through the same
`AttachCtx` pad-binding path as declarative devices. The model queues role
names; the bus resolves them to the placement's `int1_pin` / `int2_pin`.
Previously the kit queued GPIO labels without registering any pad bindings,
so its protocol-level DRDY tests did not establish delivery to an MCU.

The optional interrupt pads remain optional. A supplied pin must be a
nonempty string and resolve to a real input; null, booleans, integers, arrays,
empty/whitespace strings and unknown pads fail attachment.

On 12 November 2026, 35 focused cases passed: 17 BMI270 SPI protocol/factory
cases, 15 existing generic device-pin cases and three I2C wrapper cases.
The new positive fixture initializes the exact 328-byte Bosch image, maps
DRDY first to INT1 then INT2, and reads the actual STM32L476 GPIO input words
through the bus. It also proves status-read clear and reset parking. SPI
frames are driven directly into the attached device to isolate the factory,
controller-drain and GPIO delivery path from MCU SPI register behavior.

This base lacks the pending RP bank-aware input-resolution fix. Its existing
RP factory case therefore tests explicit CS and unpowered bus behavior with
no interrupt declared. Native RP GPIO13 delivery still requires the separate
combined-runtime controller test. Dynamic VDD/VDDIO qualification is a
separate pending gate; this receipt does not establish powered native-board
startup, FIFO/auxiliary behavior or physical sensor fidelity.
