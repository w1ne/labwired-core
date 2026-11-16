# L7 negative control: TCB3 raises the wrong vector

Same `firmware.elf` as the passing L7 cell. The only change is in the chip
descriptor (`atmega4809-tcb3-on-vector-35.yaml`): TCB3's interrupt is routed
to vector 35 (PORTE) instead of 36 (TCB3_INT).

Result: FAIL, nothing on the UART in 500000 steps (`uart.log` is empty). The
first TCB3 wrap enters vector 35, which ArduinoCore-megaavr leaves as
`__bad_interrupt` (a jump to 0), so the sketch resets before it prints. The
passing cell therefore depends on the TCB3 CAPT interrupt reaching the
`millis()` handler at vector 36.
