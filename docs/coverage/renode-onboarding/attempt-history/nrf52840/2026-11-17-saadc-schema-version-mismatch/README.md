# nRF52840 SAADC smoke schema mismatch

Attempted 2026-11-17 before boot. The simulator rejected the new `stimuli` block because the smoke declared schema 1.0; the feature requires schema 1.2. No firmware instruction executed. The main smoke script was corrected to schema 1.2 before the follow-up run. Logs, result, JUnit, snapshot, and metrics from this failed attempt are retained here.
