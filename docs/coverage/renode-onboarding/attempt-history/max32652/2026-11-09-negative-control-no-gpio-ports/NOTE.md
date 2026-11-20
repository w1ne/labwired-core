# max32652 — negative control: no-gpio-ports (2026-11-09)

Run: this round's firmware and smoke against this round's descriptor without port0..port3. Result: FAIL 1/7 checks. HardFault from a precise BusFault at 0x40008000 (GPIO0 EN, check 3) at PC 0x100004A4. The three UART STAT checks pass first.
