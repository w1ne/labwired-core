# SAMD51 SERCOM SPI split-IRQ slice

Status: implementation candidate; hosted correctness and actual guest
qualification are pending. This is not an admitted CODAL runtime or a new
performance result. Continue through the ordered
[runtime plan](samd-runtime-replacement-plan.md).

## Primary register boundary

Microchip [SAM D5x/E5x DS60001507F](https://ww1.microchip.com/downloads/en/DeviceDoc/SAM_D5xE5x_Family_Data_Sheet_DS60001507F.pdf),
section 10.2.2, printed page 68, assigns SERCOM4 interrupt sources 0/1/2 to
NVIC IRQs 62/63/64 and sources 3/4/5/7 to IRQ65. In the SPI master map,
INTFLAG/INTEN DRE, TXC, RXC and ERROR occupy bits 0, 1, 2 and 7. Thus this
slice routes those four sources to IRQs 62, 63, 64 and 65 respectively,
not to one shared IRQ62. CPU exception numbers are 78 through 81.

The [interrupt operation description](https://onlinedocs.microchip.com/oxy/GUID-F5813793-E016-46F5-A9E2-718D8BCED496-en-US-15/GUID-54B4A908-6742-4D23-AB66-0D67113A6FB5.html)
defines a held interrupt from its flag and enable. DATA writes remove DRE
until the holding register empties; DATA reads remove RXC when the receive
queue empties. TXC/ERROR support write-one-to-clear. INTENCLR masks a source
without acknowledging its flag; unmasking an already-set flag requests it.
This does not add SPI client/SSL support or claim wire-edge timing.

## Implementation and verification contract

- `configs/chips/atsamd51-pybadge.yaml` supplies IRQ62 as the start of four
  exclusively owned consecutive lines. `SamSercomSpi` reports all four levels
  and delivers active sources through the existing explicit-IRQ choke.
- Walk and scheduler re-emit held sources until acknowledged or masked.
  An IRQ-only event must not select a slave. An idle controller with INTEN=0
  still schedules nothing. Existing frame countdown, mux, MCLK/GCLK pause,
  cancellation and forced-walk controls remain mandatory.
- The shared DATA-read cleanup drops deasserted, marked multi-line pends only.
  It never raises an IRQ and must preserve unrelated lines, non-consuming
  peeks/upper DATA lanes, and a software pend of a low source.
- `crates/core/tests/sam_sercom_spi.rs` checks simultaneous source routing,
  NVIC bank crossing, per-source W1C/masking, FIFO consumption, software pend
  preservation and reset. Authored Thumb guests configure SPI/NVIC themselves,
  exercise all four handlers, leave time for a held-source re-pend, acknowledge
  inside the handler and return to thread exactly once. PRIMASK-blocked controls
  must run without handler entry. Scheduler intervals 1/64/1024 and the
  one-cycle forced reference are checked separately. Additional guests reset
  already-pending sources and gate MCLK before unmasking: no stale handler or
  pending bit may survive, and an IRQ-only cancelled wake must not select CS.
- Feature-off and feature-on PR checks run those integration tests nonvacuously
  and print actual guest rows. Require every enabled exact-head check and inspect
  the original hosted guest output before merging; a test declaration is not
  a passing receipt.

No new dynamic-clock-rate, asynchronous IRQ delivery while an in-flight
controller is clock-gated, DMAC, real fiber/event services, store admission,
hardware capture, or active Arcade RTx claim follows from this slice. Clocked
service continues to use the existing shared gate and nominal-cycle boundary.
Dependency/application pins, capture results and drift acknowledgements are
unchanged.
