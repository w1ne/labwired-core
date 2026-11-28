# NUCLEO-F103RB Arduino matrix row — evidence notes (2026-11-21)

Vendor status, read live on st.com in Chrome on 2026-11-21:

- https://www.st.com/en/evaluation-tools/nucleo-f103rb.html: ACTIVE, "Order Direct"
- https://www.st.com/en/microcontrollers-microprocessors/stm32f103rb.html: ACTIVE, "Order Direct"

Result: 9/9 cells pass (`scoreboard-2026-11-21.md`), stock stm32duino 2.12.0
(ststm32@19.7.1, board `nucleo_f103rb`), `labwired test` from this branch.
The row uses the existing `stm32f103rb` descriptor with no model change.
The existing `stm32f103` row covers the Blue Pill (F103C8) only.

Exact values (not only markers):

- L3: INA219 CONFIG 0x399F and BUS 0x19CA read back (exact tier; F1 is not
  on the F4 ACK-only branch).
- L4: MAX31855 frame 0x01901600 on SPI1, CS on D10 = PB6.
- L5: `LW_L5_VAL 511 511` = A0 (PA0, ADC12_IN0) held at 1650 mV, 10-bit.
- L2: 3 logic edges on LD2 = PA5.
- L6: 61 logic edges on PA0 (AF, TIM2_CH1), oracle needs 20.
- L8: bxCAN register-level loopback, ID 0x123, data 0xA5.

Negative controls (same firmware, system with no external devices and no
`input_mv`, `stop_when_assertions_pass: false`):

- L3: `LW_L3_FAIL nack`.
- L4: `LW_L4_FAIL frame=0x00000000`.
- L5: `LW_L5_VAL 0 0`. The generic L5 oracle also accepts this (equal
  samples), so 511 is the L5 proof, not the marker.
- L6 with `--watch-gpio gpioa:5` (LD2): 0 edges; `gpioa:0`: 61 edges.
