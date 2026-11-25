# Negative control: HEAD descriptor, new firmware (2026-11-15)

Chip descriptor `git show HEAD:configs/chips/onboarding/focaltech_ft9001.yaml`
(before the CPM, reset and TRNG entries), the same system yaml and the same
smoke script, with the firmware built for this round
(SHA-256 13c25e4c056e23bc585b816dd78ad52be75fce9275c48ff60c7ef33f71473dcf).

Result: FAIL 1/3. The first firmware access is the RSR read at 0x40002007;
with no reset controller mapped it is a precise BusFault
(CFSR=0x00008200, BFAR=0x40002007), so no UART line is printed.
