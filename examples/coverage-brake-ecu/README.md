# Coverage demo: brake-light ECU

A small safety function and six tests, to show what branch coverage finds
that line coverage hides. See `docs/howto/firmware-coverage.md`.

The ECU reads 5-byte pedal frames (`[0xA5][alive counter][pedal %][flags]
[CRC-8]`) over UART, checks them like AUTOSAR E2E Profile 1 (CRC-8 SAE J1850
plus a 4-bit alive counter), and switches the brake light. Three bad frames
in a row latch a safe state.

| Test | What it sends |
|---|---|
| `pedal-press` | Press and release. |
| `emergency-flag` | Emergency request with the pedal up. |
| `crc-error` | One frame with a broken CRC. |
| `e2e-sequence` | Repeated and skipped counters, then the safe-state latch. |
| `pedal-implausible` | Pedal value over 100 %. |
| `line-noise` | Junk bytes before a frame. |

The first three reach 81.5 % of lines but only 76.3 % of branches; the
missing branches are exactly the alive-counter checks, the safe state and
the plausibility check. All six reach 38/38 branches.

```bash
for t in tests/*.yaml; do
  labwired test --script "$t" --coverage --output-dir "out/$(basename "$t" .yaml)"
done
labwired coverage merge out -o out/merged.info --cobertura out/coverage.xml
```

The ELFs in `tests/fixtures/coverage-brake-ecu-{cm3,rv32}.elf` are built from
`src/` with `make` (clang + lld, `-O0 -g`); the RISC-V build
(`system-rv32.yaml`) gives the same branch results on the same lines.
