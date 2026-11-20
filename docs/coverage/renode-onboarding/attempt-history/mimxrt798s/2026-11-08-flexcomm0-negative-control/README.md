# mimxrt798s negative control: flexcomm0 removed (2026-11-08)

Same firmware and smoke script, with `flexcomm0` removed from the chip (the
`mimxrt798s.yaml` copy here, loaded through a one-line system manifest
`chip: mimxrt798s.yaml` with `labwired test --system`). FAIL 2/5: no UART
bytes and the pass word at 0x20000004 is never written. Only the MULS word
(written before any peripheral access) and the max_steps stop reason still
hold. The first LP_FLEXCOMM access (the PSELID read at 0x40110FF8, PC
0x08000866) is unmapped and escalates to a HardFault (CFSR 0x8200 PRECISERR,
BFAR 0x40110FF8). The UART and register checks therefore depend on the
modelled flexcomm0.
