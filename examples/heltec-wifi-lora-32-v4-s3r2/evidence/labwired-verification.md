# LabWired verification

Checked 2026-11-06 UTC with `labwired_verify`.

- Result: `status: model_verified`, `run_status: pass`.
- Oracle: `GPIO35` ended high after the Arduino sketch configured it as an output and wrote `HIGH`.
- Diagram: generic ESP32-S3 model with GPIO35 wired through a series resistor to an LED and back to GND. The exact Heltec board isn't currently a LabWired catalog board, so this hosted result verifies the S3 GPIO behavior, not every peripheral on the Heltec system.
- Model gap: the Playground assumes an ideal MCU supply because the diagram does not route a power rail. That approximation does not block the GPIO result; power-on and brown-out behavior were not tested.

The exact Heltec V4 S3R2 configuration separately passes the repository's
`scripts/onboarding_smoke.sh` check. Its committed run log asserts at least
eight GPIO35 edges: [`logs/run_smoke.log`](logs/run_smoke.log).

The SX1262 radio smoke is a separate repository-local run, not covered by this
hosted oracle result. The hosted component catalog returned no `sx1262` entry
when checked on 2026-11-06, so no hosted radio model-verification claim is made.
