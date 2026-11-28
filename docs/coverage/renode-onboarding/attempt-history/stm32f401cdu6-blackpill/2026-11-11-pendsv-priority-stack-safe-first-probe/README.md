# F401 PendSV priority probe: stack-safe first run

After moving test scratch below the active stack, the full firmware completed
and the existing checks passed. The new priority probe still failed; this
attempt had only a generic failure marker, so the next run added individual
probe flags to identify the condition.

This failed local CLI run is retained as diagnostic history, not the current
scoreboard measurement.
