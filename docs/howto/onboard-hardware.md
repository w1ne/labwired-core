# Onboard hardware

Add something new to the LabWired twin: a **part** (sensor, SPI chip, actuator), a **board/MCU**, or an **on-chip peripheral**.

---

## Pick a track

| Track | You are adding… | Start here |
|-------|-----------------|------------|
| **A. Part** | I²C sensor, SPI device, servo, motor, buzzer, display module | **[Onboard a part](onboard-part.md)** (default) |
| **B. Board / MCU** | A new chip or development board | [Board playbook](../board_onboarding_playbook.md) |
| **C. On-chip peripheral** | Behavior inside the MCU (timer, UART, …) | [Peripheral modeling](../peripherals.md) |

Most product work is **track A**: the MCU already exists; you need the sensor or actuator on the bus.

---

## When is it “done”?

| Level | Meaning (short) |
|-------|------------------|
| **Modeled** | Config loads; firmware can talk on the bus |
| **Smoke (L1+)** | A deterministic test or verify script passes in CI |
| **Supported** | Do not call a target “supported” below smoke — see [Target support rubric](../target_support_rubric.md) |

---

## Related

- [Parts catalog](../parts/index.md)
- [Run firmware (CLI)](../getting_started_firmware.md)
- [Agent tools](../agent/tools.md)
