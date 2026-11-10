# ESP32-DevKitC-32E PCNT smoke

- Demand rank: ESP32 rank 1 (38% MCU use).
- Work type: low-cost board-profile integration and smoke for the existing
  PCNT model, not a new board/MCU demand pick.
- Target: exact ESP32-DevKitC-32E / ESP32-WROOM-32E board profile.
- Behavior: Arduino firmware routes GPIO23 pulses to classic PCNT unit 0,
  reads a count of 8, then raises GPIO16 as the pass output.
- Local smoke: `PASS 3/3 checks`; UART count marker, 16 GPIO23 transitions,
  and GPIO16 pass edge are required.
- Hosted verification: `status: model_verified`; GPIO16 high is the explicit
  behavioral oracle. The hosted profile assumes an ideal 3.3 V rail and stops
  at the five-million-step budget in the sketch's empty `loop()`.
- Limits: PCNT glitch filtering, external pulse sources, encoder decoding, and
  other PCNT units/channels are outside this smoke.
- Live board/module status: [`vendor-status.md`](vendor-status.md).
- Detailed local logs and simulator outputs are in this directory.
