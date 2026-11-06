# Hosted verifier availability

The hosted LabWired component catalog was queried on 2026-11-06. It lists no
`gc9a01` component, so it cannot issue an exact hosted `model_verified` verdict
for this board's display path. No hosted verdict is claimed.

The exact `gc9a01` device model is present in this repository. The local
`scripts/onboarding_smoke.sh` run used that model and passed all three checks;
the recorded result is `simulation-output/result.json` and the run log is
`logs/run_smoke.log`. This local evidence is not a hosted verifier verdict or a
physical-board test.
