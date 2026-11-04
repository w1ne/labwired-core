# Verification scope — STM32F429I-DISC1

The repository-local LabWired CLI onboarding smoke is the executable check for
this target. The hosted `labwired_verify` catalog does not currently list an
STM32F429 target, so no hosted oracle verdict is claimed. See
[`logs/run_smoke.log`](logs/run_smoke.log) and the asserted simulator result in
[`simulation-output/result.json`](simulation-output/result.json).

The result records `unpowered_rail_assumed`: VDD is treated as an ideal rail
because this smoke profile does not route the board power net. Power-on and
brown-out reset behavior are therefore outside the verified scope.
