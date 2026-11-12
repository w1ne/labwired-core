# Optional OPAMP supply envelope evidence

The final opt-in VMIN/VMAX candidate passes 83 focused cases: 15 supply-gate cases, 47 unchanged macro cases, and 21 adapter cases. `r7-result.json` binds the exact changed Rust source hashes and its log. Logs are stored as deterministic gzip archives with exact decoded-byte hashes in `log-encoding.json`; the uncompressed originals remain in the work evidence directory. It covers cold ramp, zero-power currents, signed and inclusive bounds, preserved compensation charge, independent instances, no-stable-active-set refusal, generated primitive ownership, bitwise nominal parity, and accepted readback after a refused trial.

## Independent review and corrected failures

The earlier `r3` candidate passed 82 cases. Independent source review then found that a rejected overvoltage trial could leave the working supply gate off while public node voltages still represented the accepted powered state. `r4-red` reproduces its false zero clamp-current readback. The correction reads qualification from the accepted voltage buffer; `r5` passes all 83 cases.

A separate recovery assertion then exposed the retained off gate causing a false singularity on a later valid unloaded powered trial. `r6-red` preserves that failure. The final source also recomputes only the OPAMP working gates from the latest accepted voltage buffer at a terminal advance failure. This retains any accepted substep and leaves other device state and legacy cards unchanged. `r7` passes both the overvoltage and genuine floating-off refusal cases, their unchanged accepted current observations, and the subsequent valid powered recovery.

The first normal commit attempt was stopped to address that review finding. Its record remains in `/data/work-evidence/opamp-power-20261112/commit-result.json`; it did not create a commit. An intervening command-selection error named a nonexistent test target and is preserved in the scratch `r4-green` record; no test ran in that invocation. All subsequent checks use the actual `analog_macros` target.

## Model boundary

This is a functional envelope and disabled-output assumption. It does not establish physical off impedance, leakage, UVLO, startup, input protection or signal-dependent supply current. No output shunt, substitute rail or circuit-specific constant is added. The native circuit requires its separately versioned model declaration, explicit initialization and execution evidence.

The generic application probe was linked against the exact production library. `probe-build-r7-receipt.json` records the final formatted harness, command, library and executable hashes. The older `probe-build-receipt.json` remains the historical r3 link. Linking alone is not native execution evidence. Normal source gates and the eventual commit identify the release candidate; no historical physical capture or human acknowledgment was changed.
