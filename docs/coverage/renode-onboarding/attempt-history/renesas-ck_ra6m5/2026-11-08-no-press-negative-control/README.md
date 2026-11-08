# renesas-ck_ra6m5 negative control: button never pressed (2026-11-08)

Same firmware, chip and system, with the smoke's four button `stimuli`
removed (the `renesas-ck_ra6m5-smoke-no-press.yaml` copy here). FAIL 9/11:
the press counter at 0x20000010 stays 0 and the green LED (P609) shows no
edges. The MULS word, SCR readback, PODR readback, idle button level (P804
low), MSTPCRA/B reset values, UART banner, red LED heartbeat and stop reason
still hold. So the two button-dependent checks pass only because the
board_io button drives PORT8 PIDR and the firmware reads it back.
