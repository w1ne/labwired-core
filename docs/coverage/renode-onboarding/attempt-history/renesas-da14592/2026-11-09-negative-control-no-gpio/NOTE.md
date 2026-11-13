# renesas-da14592 — negative control: no-gpio (2026-11-09)

Run: this round's firmware and smoke against this round's descriptor without the `gpio` block (UART windows 256 B). Result: FAIL 1/6 checks. HardFault from a precise BusFault at 0x50020618 (P0_00_MODE, check 2) at PC 0x00000888. The two UART LSR checks pass first.
