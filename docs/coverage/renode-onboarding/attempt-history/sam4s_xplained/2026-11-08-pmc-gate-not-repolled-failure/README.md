# sam4s_xplained — PMC gate-off not seen by the TC model (2026-11-08)

First run of the deepened smoke (firmware-sam4s-xplained-renode-smoke on the
new `sam4s_tc` model): 6/7 firmware checks passed; FAIL 3/5 junit.

    tc5 pmc gated=1 held=0 tc4_runs=1 FAIL

Cause: the first `sam4s_tc` draft re-read PMC_PCSR0 only after a TC_CCR store
(or while an enabled channel already saw its clock off). A PMC_PCDR0 store
does not notify the TC block, so a running TC5 kept counting after its
peripheral clock was gated off. Fixed by re-reading PCSR0 on every bus tick
while any channel's clock is enabled (tc4s.rs `needs_bus_tick`).
