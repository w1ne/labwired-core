# BASEPRI smoke attempt 2

The target booted and exercised the BASEPRI case. The high-priority IRQ ran
with BASEPRI set to `0x80`; IRQs 7 and 9 stayed pending, then ran in priority
order after the mask cleared. The only new context bit that failed was the
MSP restoration check (`stack=1,0`). This fixture had switched Thread mode to
PSP across separate inline-assembly blocks, allowing compiler-generated stack
use between them to invalidate the assumption that MSP stayed unchanged.

The next attempt keeps Thread mode on MSP and checks exception return plus MSP
restoration. Full per-target run artifacts are preserved here; the runner had
recorded one failed target before the final replay.
