# Hosted LabWired verification attempts

The hosted verifier did not return `model_verified`, so no hosted model-
verification pass is claimed for this board profile.

- A Rust compile-then-run request using the nRF54L15 GPIO firmware failed at
  compile time because the hosted build had no Cortex-M interrupt vector table.
- A C compile-then-run request failed before firmware compilation because the
  hosted nRF54L15 Zephyr setup requires Python 3.12 or newer but provides 3.11.2.

The required repository onboarding smoke is separate evidence. Its committed
`simulation-output/result.json` records `status: pass`, 32 observed GPIO2:0
edges, both passing assertions, and the ideal-VDD assumption. It does not
establish hosted `model_verified` status or behavior on a physical board.
