# LPC2294 timer smoke attempt 1 (2026-11-09)

This one-target inventory run booted 100,000 instructions, but failed the
UART assertions after printing the first eight passing check marks. The
captured CPU snapshot showed the interrupted wait loop's shared R0/R1 values
had been overwritten by the IRQ vector and handler, so the fixture kept
polling the wrong locations. This was an IRQ-fixture defect; it did not
establish whether the timer model passed.

The next attempt initializes the banked IRQ stack and saves/restores the shared
registers around VIC vector dispatch. The original metrics, UART capture,
JUnit file, simulator result, snapshot and logs are retained in this
directory.
