# L6 negative control: watch the pad next to the PWM pin

Same `firmware.elf` and system as the passing L6 cell. The only change is the
watched pad: `--watch-gpio portb:1` (D10, TCA0 WO1) in place of `portb:0` (D9,
TCA0 WO0).

Result: 0 edges on PB1 (`result.json`, `logic_edges`), below the cell's
`pwm_min_edges: 20`, so the matrix oracle fails the cell. The sketch calls
`analogWrite(9, 128)` only, which sets `TCA0.CTRLB.CMP0EN` and leaves
`CMP1EN` clear. The passing cell records 60 edges on PB0. Its edge count
therefore comes from the compare output that firmware enabled, not from the
timer running.
