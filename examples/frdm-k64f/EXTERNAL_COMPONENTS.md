# External Components — FRDM-K64F

None for the UART-only L1 slice. The smoke exercises the MCU's own SIM + GPIO +
UART0 surface; UART0 is the modelled console and the on-board OpenSDA bridge is
not modelled.

The board carries an **FXOS8700CQ** accelerometer/magnetometer on I²C0, a
**KSZ8081** RMII Ethernet PHY, a microSD slot, and the OpenSDA debug bridge.
**None of these are modelled or attached** in this example — the system
manifest lists only the RGB LED and the two user buttons as `board_io`.

When the Ethernet/TCP milestone lands, the KSZ8081 PHY goes here with its
fidelity tier and descriptor path, and the system manifest grows an
`external_devices:` entry. The FXOS8700 sensor belongs to a later I²C slice.
