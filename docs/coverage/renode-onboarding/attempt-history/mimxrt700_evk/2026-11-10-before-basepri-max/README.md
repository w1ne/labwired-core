# MIMXRT700-EVK baseline before BASEPRI_MAX smoke

This archive preserves the passing target evidence from commit `65c2ba8c9` before the BASEPRI_MAX update. It includes the per-target metrics, simulator logs and results, plus the smoke YAML that produced them.

The baseline executed 24,000,000 instructions and passed all 35 smoke assertions. The UART smoke checked plain BASEPRI, PRIMASK, FAULTMASK, nested IRQ entry, priority deferral, and the existing LPUART behavior. It did not check BASEPRI_MAX write semantics.
