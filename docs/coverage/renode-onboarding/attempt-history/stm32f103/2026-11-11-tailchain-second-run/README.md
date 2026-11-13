# Second tail-chain smoke attempt

This run booted but failed the existing nested-preemption assertion. While correcting the new tail-chain fixture's priority bytes, the prior nested-preemption fixture was changed instead. The captured active-bit result identified the reversed priorities; the final source restores the existing fixture and applies the new ordering only to the tail-chain test.
