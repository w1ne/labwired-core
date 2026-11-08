# Hosted LabWired oracle — STM32F411CEU6 Black Pill PC13

The hosted `labwired_verify` oracle ran on board `stm32f411ceu6-blackpill` with
the clause `gpio_edges: PC13 >= 16`.

- Result: `status: model_verified`, `run_status: pass`.
- Observation: PC13 transitioned **48 times**; stop reason was `max_steps` at
  100,000 steps.
- The hosted compiler could not link the repository's Rust source because its
  generated scaffold lacked the Cortex-M interrupt vector table. The oracle
  therefore ran a C translation of the same RCC/GPIOC register sequence and
  toggle loop. The exact Rust firmware is exercised by the committed local
  onboarding smoke under `led-smoke/`.
- Non-blocking model gap: no power net is routed in the hosted board diagram,
  so the twin assumes an ideal 3.3 V rail and does not model power-on or
  brown-out reset. The GPIO edge clause passed despite the verifier's separate
  empty-serial diagnostic; this smoke has no serial clause or UART firmware.

This is deterministic digital-twin evidence, not physical hardware evidence.
