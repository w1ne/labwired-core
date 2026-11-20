First ITIM model counted on the legacy walk (`tick_elapsed(cycles)`). The CLI
hosted fast path widens the bus tick to 1024 cycles while no model walks; the
ITIM joined the walk later and was credited the wide window per bus tick, so
ITCNT = 99 at 1 MHz timed out every ~50 cycles instead of 1500. The core
stayed in the IRQ 27 handler (xPSR exception 43, exception 43 re-pended) and
never printed the result line: FAIL 2/3, UART 19 bytes. Trace run with
--max-steps 100000. Fixed by deriving the count from the bus CycleClock
(re-anchored on each write) with scheduler events for the timeout.
