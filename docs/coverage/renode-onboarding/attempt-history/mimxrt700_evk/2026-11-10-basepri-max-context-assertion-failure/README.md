# BASEPRI_MAX smoke attempt: stale context assertion

This run booted the imported MIMXRT700-EVK system. It passed the UART assertion for the BASEPRI_MAX threshold and the firmware pass bitmap, but the YAML memory assertion at `0x20000068` still expected the previous BASEPRI context `0x3FF` instead of `0x7FF`.

The runner recorded 34/35 checks. Only the stale BASEPRI context memory assertion failed. The full run evidence is preserved here.
