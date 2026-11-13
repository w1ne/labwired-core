# Initial imported-profile attempt

This diagnostic run used the unmodified Renode-imported memory/peripheral descriptor. It did not boot: reset fetched the erased default flash alias at address 0 and attempted to execute at `0xfffffffe` (0 instructions). The profile was then narrowed and its i.MX RT reset mapping corrected before the measured run. This failed attempt is retained for audit; the scoreboard reports the later run against the corrected descriptor.
