# NUCLEO-F767ZI Arduino matrix row — evidence notes (2026-11-21)

Vendor status, read live on st.com in Chrome on 2026-11-21:

- https://www.st.com/en/evaluation-tools/nucleo-f767zi.html: ACTIVE, "Order Direct"
- https://www.st.com/en/microcontrollers-microprocessors/stm32f767zi.html: ACTIVE

Result: 9/9 cells pass (`scoreboard-2026-11-21.md`), stock stm32duino 2.12.0
(ststm32@19.7.1), `labwired test` from this branch.

Exact values (not only markers):

- L3: INA219 CONFIG 0x399F and BUS 0x19CA read back; no `LW_L3_PARTIAL_NO_RX`.
- L5: `LW_L5_VAL 511 511` = A0 (PA3, ADC1_IN3) held at 1650 mV, 10-bit.
- L6: 61 logic edges on PA0 (TIM2_CH1, AF1), oracle needs 20.
- L8: bxCAN1 register-level loopback, ID 0x123, data 0xA5.

Negative controls:

- L6 without `input_capture: "stm32f4"` on tim2 (main before this change):
  `logic edges 0 < required min_logic_edges 20 (watch_gpio=['gpioa:0'])`.
  No F7 timer channel reached a pad.
- L8 with the same firmware on a chip copy without `bxcan1`: FAIL 0/1,
  `HardFault escalated from a precise BusFault: data access at 0x4000_6400`.
