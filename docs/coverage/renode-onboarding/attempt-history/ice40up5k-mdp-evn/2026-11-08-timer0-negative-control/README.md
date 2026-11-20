# ice40up5k-mdp-evn negative control: timer0 removed (2026-11-08)

Same firmware (`firmware-fomu-smoke --features timer0`) and smoke script, with
`timer0` removed from the chip (the `ice40up5k-mdp-evn.yaml` copy here, loaded
through a one-line system manifest `chip: <abs path>/ice40up5k-mdp-evn.yaml`
with `labwired test --script`).
FAIL 1/5: the first timer0 access (the EN write at 0x60002820) is a memory
access violation at step 0 of the run, so the load readback, check-bit and pass
words are never written and the run does not reach max_steps. Only the MUL word
at 0x10000000 (written before any peripheral access) holds. The timer checks
therefore depend on the modelled timer0.
