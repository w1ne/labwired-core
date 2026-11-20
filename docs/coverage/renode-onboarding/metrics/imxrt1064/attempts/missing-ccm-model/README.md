# Attempt after correcting the reset alias

This diagnostic run executed 800,000 instructions but faulted on the first clock-gate write at `0x400fc068`; the CCM model was mapped at the wrong base (`0x40100000`). UART and GPIO assertions did not pass. The descriptor's CCM base was corrected to `0x400fc000` before the final measured run.
