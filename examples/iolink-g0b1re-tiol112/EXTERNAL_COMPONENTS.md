# External components

| Part | Core type | Modeled behavior | Boundary |
|---|---|---|---|
| TIOL112 | `tiol112` | EN, inverted CQ, RX echo, GPIO status, timed TX/RX propagation and enable/release scenarios | Digital, ideal pull-ups; no analog or guaranteed EN settling bound |
| Virtual IO-Link master | TimedUartPort test driver | Bit-timed 8E1 requests, real startup/OPERATE, reply validation | Scripted master; no Beckhoff firmware/emulation |
| 24 V and 3.3 V rails | Diagram power nets | Explicit connectivity | Analog supply/load/protection not simulated |

TIOL112 must not be substituted with TIOL1123 without changing the supply-pin
contract. TIOL112 pin 1 is VCC_IN; TIOL1123 pin 1 provides 3.3 V output.
