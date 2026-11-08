# L6 PWM on the Arduino Nano Every

Stock ArduinoCore-megaavr `analogWrite(9, 128)`. D9 is PB0 = TCA0 WO0: the
core's `setup_timers()` sets PORTMUX.TCAROUTEA = PORTB, TCA0 single-slope
with PER = 0xFF at CLK_PER/64, and `analogWrite` writes CMP0BUF = 128 and
sets CTRLB.CMP0EN. (`LED_BUILTIN` = D13 = PE2 has no timer output, so the
sketch uses D9 on this board.)

`test.yaml` / `result.json` / `uart.log` / `junit.xml`: the matrix cell as
`run_matrix.py` ran it, with `pwm_watch: portb:0` and `pwm_min_edges: 20`.
`result.json` `logic_edges` holds 60 transitions on PB0. After the first,
partial period (CMP0EN is set mid-period), every period is 16,384 cycles
(976.6 Hz at 16 MHz) and every high phase is 8,192 cycles (50.00 %),
which is 128 of 256 counts of 64 cycles.

`controls/`: the same firmware and the same CLI binary, with the chip's
PORTMUX put back to the plain ATDF register bank
(`atmega4809-portmux-register-bank.yaml`). Without the TCAROUTEA remap
cell, the bus binds no TCA0 route, so PB0 shows the OUT latch. The run
prints `LW_L6_OK` all the same, but captures **0 edges** on PB0. That is
why L6 counts only with `pwm_watch`: the UART marker alone passes either
way.

What this does not show: split mode (WO3..WO5), the dual-slope and
frequency modes, TCA interrupts, and the TCB waveform outputs (D3 and D6
on this board) are still refused. A buffered PER reaches the pads at the
first access or wake after the wrap, not at the wrap itself.

Paths in the yaml files are repo-relative records. The control was run
from a /dev/shm copy of `configs/` and the system yaml. The CLI resolves
paths relative to the script, so run them from a copy with absolute paths.
