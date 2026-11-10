# Validation — ESP32-C6-DevKitC-1 MCPWM servo

Target: ESP32-C6-DevKitC-1-N8 (v1.2) with an external SG90 servo connected to
J1 GPIO4. The firmware configures MCPWM0 timer 0 and operator 0 for an up-count
50 Hz frame, routes PWM0A to matrix signal 87, and leaves the waveform running.

**Local onboarding smoke: PASS (4/4 assertions).** The script built the CLI and
firmware, then confirmed the UART marker, a 1,500-count comparator value,
multiple GPIO4 transitions, and the expected max-step stop. The committed
result is [`evidence/simulation-output/result.json`](evidence/simulation-output/result.json);
stage logs are in [`evidence/logs/`](evidence/logs/). This is a local Core CLI
simulation result, not a hosted `model_verified` result or a physical board
capture.
