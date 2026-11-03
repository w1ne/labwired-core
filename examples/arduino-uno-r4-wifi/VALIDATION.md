# Arduino UNO R4 WiFi validation

## Board and model scope

The target is Arduino UNO R4 WiFi, SKU ABX00087, with Renesas
R7FA4M1AB3CFM#AA0. The simulator captures SCI2, which the board routes to its
D0/D1 header pins (P301/P302); the active-high D13 LED maps to P102
(RA4M1 port 1 bit 2). It reuses the existing `ra_sysc`, `ra_port`, `ra_pfs`,
and SCI models. The chip descriptor contains only those required
windows; unmodeled blocks are omitted so firmware that accesses them faults
instead of receiving fabricated zero answers.

The smoke firmware waits for HOCO readiness, configures P102 as GPIO output,
prints `UNO_R4_WIFI_OK` through SCI2, and produces at least eight P102 output
edges. The local LabWired CLI test asserts both UART output and GPIO transitions.
The console is captured at the SCI2 model; this does not test the USB serial
bridge or the board's physical D0/D1 electrical routing.

## Evidence

- `evidence/logs/run_smoke.log`: local engine smoke result.
- `evidence/simulation-output/`: UART capture, GPIO observations, and run result.
- `evidence/vendor-status.md`: current Arduino/Renesas product status.

This is simulation evidence only. The hosted `labwired_verify` catalog does not
currently expose the UNO R4 WiFi target, so no hosted `model_verified` or
silicon-confirmed claim is made.

## Sources

- [Arduino UNO R4 WiFi official product page](https://store.arduino.cc/products/uno-r4-wifi) and [full pinout](https://docs.arduino.cc/resources/pinouts/ABX00087-full-pinout.pdf): board composition and D0/D1/D13 assignments.
- [Renesas RA4M1 hardware manual](https://www.renesas.com/en/document/mah/ra4m1-group-users-manual-hardware): register behavior and memory map.
- The focused descriptor reuses the repo's existing RA4M1 register profile in `configs/chips/ra4m1.yaml`; its modeled boundary is not a claim that the remaining RA4M1 blocks work.
