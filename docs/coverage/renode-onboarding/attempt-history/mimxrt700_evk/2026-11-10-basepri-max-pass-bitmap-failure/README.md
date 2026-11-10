# BASEPRI_MAX smoke attempt: stale pass bitmap

This first run booted the imported MIMXRT700-EVK system and passed the new BASEPRI_MAX UART assertion. The recorded threshold was `0x80` after writing `0xC0`, then `0x80`, then the less-masking `0xA0`, and finally zero through BASEPRI_MAX.

The smoke failed because the firmware returned the expanded BASEPRI context `0x7FF` but the pass bitmap still compared that context against the old value `0x3FF`. As a result, the firmware printed `pass=0000dfff`; the YAML expected `pass=0000ffff`. The runner recorded 32/35 checks, with three failures tied to the stale pass bit and its dependent scoreboard assertions. No BASEPRI_MAX behavior assertion failed.
