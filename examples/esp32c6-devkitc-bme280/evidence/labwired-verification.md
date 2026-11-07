# Hosted verifier status

The LabWired component catalog identifies `esp32-c6-devkitc-1` as runnable
with a supplied ELF, but reports that it has no hosted compile path. The hosted
verifier's supported compile targets also do not include the C6. No hosted
`model_verified` verdict is claimed for this example.

The committed evidence is the repository's local deterministic Core CLI smoke
under `logs/` and `simulation-output/`. Its three assertions passed, including
the BME280 ID and expected default raw temperature. This is not physical
hardware confirmation.
