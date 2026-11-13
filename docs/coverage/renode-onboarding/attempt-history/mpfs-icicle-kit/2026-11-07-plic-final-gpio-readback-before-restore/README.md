# PLIC smoke fixture failure: stale final GPIO readback

This retained run is a reproduced fixture assertion failure, not a platform boot failure. The pre-fix firmware configures GPIO_0 port 0 to raise the PLIC source, then checks the old all-high input value after it has cleared the GPIO output latch. At that point only port 0 is high, so the result correctly reports `0x1` instead of the stale expected `0x3fff`.

The final smoke disables interrupt generation, restores `CFG_ALL = 0x7` and `GPIO_OUT = 0xffffffff`, then checks the 14-port input value. The current per-target metrics pass 9/9 assertions for all three imported systems. This archive is kept for the failed attempt history and is not part of active scoreboard metrics.
