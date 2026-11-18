# L6360 digital control model

Source: ST [DS8900 Rev 9, September 2023](https://www.st.com/resource/en/datasheet/l6360.pdf), sections 6.8–7.2 and 8.1. This model has no physical-silicon capture. Unit and firmware evidence must be labelled simulation evidence.

`L6360Control` owns shared register and protection state. Its single `L6360I2c` endpoint implements the existing I2C device interface. The integration kit supplies sampled pins, explicit threshold scenarios and monotonic simulation nanoseconds. The shared UART implementation owns serialization and wire traffic.

## Bus contract

- Seven-bit hardware addresses are 0x60–0x67.
- Current writes contain DATA followed by the parity/address byte. Sequential writes contain seven writable-register bytes followed by their parity byte. Register changes commit at STOP; failed checks preserve the previous register values.
- A one-byte pointer write requires STOP before a random read. Without a pointer, the response is STATUS then PARITY.
- PARITY is stored received-data parity. Its documented reset is zero. The model does not fabricate configuration parity on the first read. STATUS parity follows STATUS. Unassigned readback bits are deterministic zero and are not a silicon reset-value claim.
- Only a complete current read clears eligible historical status. Fault epochs prevent a read from clearing a newer event. Random and partial reads do not acknowledge status.
- Unsupported register/length exchanges fail closed as explicit model protocol errors. They do not invent silicon PE bits. Reset restores the endpoint. The existing slave interface cannot NACK an individual data byte.

## Protection and integration

The snapshot distinguishes requested drive from output shutdown and exposes mode, receiver enable, IRQ assertion, inversion and decoded timing. The kit must apply the actual INC/Q truth table; forced-on modes are not UART transmit modes.

Cut-off and restart use the configured typical delays and caller time. Overload inputs mean a threshold has already been exceeded; no voltage/current/temperature solver is present. Latched protection releases on a valid internal-register access while historical status remains independently observable. Snapshot reads have no I2C side effects.

Supply loss invalidates active bus selection and disables outputs. Exact brownout register retention is unqualified; explicit RST establishes documented defaults. Pin propagation, debounce application, analog rise/fall, current tolerance, inductive decay and thermal behavior remain outside this control module.

## Evidence

`l6360_control/tests.rs` exercises reset parity, independent bus vectors, write rejection, all bridge configurations, two-port isolation, STOP selection, fault/read races, output shutdown, protection deadlines and long-interval restart phase. An integration result is complete only when the actual physical firmware passes through the existing controller, GPIO and UART paths. Host fixtures alone are insufficient.
