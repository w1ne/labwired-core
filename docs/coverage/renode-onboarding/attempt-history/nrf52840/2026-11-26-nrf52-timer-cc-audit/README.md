# nRF52 TIMER compare-channel audit

This is a descriptor and MMIO-surface check, not a new onboarding firmware
run. The gate assembles the shipped nRF52832, nRF52833 and nRF52840 profiles,
plus the separately imported nRF52840 profile, then checks each declared
TIMER's highest implemented CC register and first absent CC register.

Nordic's nRF52832, nRF52833 and nRF52840 Product Specifications agree:
TIMER0..2 have four CC registers; TIMER3..4 have six. The tested descriptors
match that estate, and the assembled model retains the last supported CC while
ignoring the first unsupported one.

Evidence:

- `gate.log` — targeted integration-test result (1/1 passed).
- Sources: [nRF52832 TIMER](https://docs.nordicsemi.com/r/bundle/ps_nrf52832/page/timer.html),
  [nRF52833 TIMER](https://docs.nordicsemi.com/r/bundle/ps_nrf52833/page/timer.html),
  [nRF52840 TIMER](https://docs.nordicsemi.com/r/bundle/ps_nrf52840/page/timer.html).

No onboarding metric changed. The committed scoreboard remains at 245
descriptors, 231 booted, 231 booted and smoke-passing, 14 `not_run`, and zero
failures among attempted runs. No inventory refresh or scoreboard regeneration
was needed for this register-width gate.
