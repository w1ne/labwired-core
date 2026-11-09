Negative control (2026-11-09), final firmware and engine: sam4s8b with
the EEFC told the part has 1 MiB (flash_bytes 1048576, the SAM4S16C size)
instead of 512 KiB. GETD reports size 0x100000 / 128 lock regions and the
check fails ("eefc getd ... FAIL"); every other check passes, summary
0x5A400FFD. FAIL 1/3.
