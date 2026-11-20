# ut32m0r500 — negative control: no-reset-altfunc (2026-11-09)

Run: this round's descriptor without `reset_altfunc: 0xFFFF` on gpio_bank2. Result: FAIL 5/7 assertions; summary `PFPPPPPPPP` (0x5E3203FD). Check 2 (GPIO2 ALTFUNC reset 0xFFFF, Renode alternateFunctionResetValue) fails; everything else passes.
