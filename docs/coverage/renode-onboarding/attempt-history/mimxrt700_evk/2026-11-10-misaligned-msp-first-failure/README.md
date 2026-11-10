# MIMXRT700-EVK misaligned MSP first attempt

This target-only run booted the imported Cortex-M33 system but failed the smoke.
Adding a separate static MSP stack shifted the existing PSP stack address and
invalidated its fixed-address assertions. The new MSP check also computed the
frame pointer four bytes too low; its captured R0 was `0xFFFFFFF9` and the
stacked xPSR was read from the wrong word. The firmware now reuses the existing
PSP buffer after its PSP cases complete and locates the frame at the eight-word
frame plus the optional alignment word.

The full failed metrics, result, JUnit, UART, snapshot, and build/run logs are
preserved beside this note. This failed attempt is not counted as a passing
measurement.
