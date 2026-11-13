# sam_e70 — before the PIO pull-up offset fix (2026-11-09)

Run: new sam_e70 descriptor + firmware-sam-e70-renode-smoke, CLI built before
the fix. Probe pin moved to PA12 for this run (same result on PA11).

Result: FAIL, 13/14 checks. `pioa pa11 change up=0 isr=11`: the firmware
writes PIO_PUER at +0x64 (ASF sam4s and hal_atmel same70 component/pio.h:
PUDR 0x60, PUER 0x64, PUSR 0x68). `sam4s_pio` had the two swapped, so the
"enable" write disabled the pull-up and the pad stayed low. The sam4s and
same70 Xplained firmwares used the same swapped offset, which hid the bug.
