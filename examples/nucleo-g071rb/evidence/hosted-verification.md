# Hosted LabWired oracle verification

The companion sketch [`../hosted-oracle-smoke.ino`](../hosted-oracle-smoke.ino)
was run with `labwired_verify` on board `nucleo-g071rb` using this oracle:

- serial contains `NUCLEO_G071RB_OK`
- PA5 is high

Result: **`model_verified`**; both oracle clauses passed, run status `pass`,
stop reason `max_steps`. This is a hosted Arduino sketch result. The repository's
Rust firmware and its local CLI smoke are recorded separately in
`logs/run_smoke.log` and `simulation-output/`.

The verifier reported one non-blocking model approximation: no power net was
connected, so VDD was assumed to be an ideal rail; power-on and brown-out reset
are not simulated. This result is not hardware confirmation.
