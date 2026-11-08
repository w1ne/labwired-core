# Hosted LabWired verification attempt

The hosted `labwired_verify` request used the generic `nrf54l15` board, a GPIO
oracle for P2.00 low, and the XIAO smoke's GPIO firmware. The tool returned
`status: inconclusive`: its generic board did not provide state evidence for
P2.00. It also reported `unpowered_rail_assumed` because the diagram did not
route a power net; the verifier assumes ideal VDD and does not simulate
power-on or brown-out reset.

This is not a hosted `model_verified` result. The board-specific repository
smoke is separate evidence: [`onboarding-summary.md`](onboarding-summary.md)
records `status: pass`, including at least 16 GPIO2:0 edges and the expected
max-step stop. The local result also records the ideal-rail limitation.
