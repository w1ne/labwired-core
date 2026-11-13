# renesas-ek_ra2e1 negative control: S1 never pressed (2026-11-08)

Same firmware, chip and system, with the smoke's four S1 `stimuli` removed
(the `renesas-ek_ra2e1-smoke-no-press.yaml` copy here). FAIL 8/10: the press
counter at 0x20004010 stays 0 and LED2 (P914) shows no edges. The MULS word,
SCR readback, PODR readback, S1 idle level (P004 high through the pull-up),
UART banner, LED1 heartbeat and stop reason still hold. So the two
button-dependent checks pass only because the board_io button drives PORT0
PIDR and the firmware reads it back.
