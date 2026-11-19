# Four-item circular DMA half-transfer first attempt

The exact-import `stm32f103` target booted, but the newly added firmware check
reported `FAIL`: `HALF1=1`, `LAP=1`, `HT_REPEAT=1`, `IRQ=1`, and
`EOC_CLEAR=1`, while its combined `SEQ` assertion was false. The inventory
smoke still reported pass because its YAML assertions did not yet include this
new UART line. This is therefore a firmware behavior failure hidden by an
incomplete smoke oracle, not evidence for the new check.

The original per-target build/run logs, UART output, JUnit, result, snapshot,
and metrics are preserved beside this note. The result is superseded only
after the data mismatch is diagnosed and the exact imported-target assertion
passes.
