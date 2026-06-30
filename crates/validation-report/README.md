# validation-report — provenanced model-fidelity reports

proto.cat's differentiator is *real working hardware*: "the firmware verifiably runs."
That claim is only as good as the **fidelity of the silicon models** it runs on — so
"validated model" has to be an **audit trail**, not an assertion.

labwired already validates models several ways, but the evidence is scattered:

| Authority | Where it lives | What it proves |
|---|---|---|
| tier-1 raw-register vs TRM | `docs/coverage/tier1-matrix.json` | each peripheral's register sequence matches the vendor TRM |
| silicon reset-conformance | `crates/hw-oracle/` (committed silicon captures) | reset-state registers match real silicon, no board needed at check time |
| SVD register coverage | `crates/svd-ingestor/` + `configs/peripherals/` | register map is vendor-authoritative (CMSIS-SVD) |
| real vendor-stack boot | `examples/*/VALIDATION.md` | unmodified ESP-IDF/Zephyr/HAL/UDSLib runs correctly |

This crate consolidates that into one `ModelValidationReport` per chip: per peripheral,
**what** was checked and against **which authority**, with a link/path to the backing
run. So a reviewer (or proto.cat's verdict) can cite validation, not just claim it.

## Use

```sh
# tier-1 + SVD (auto-finds configs/peripherals/<chip>)
cargo run -p validation-report -- docs/coverage/tier1-matrix.json esp32c3        # markdown
cargo run -p validation-report -- docs/coverage/tier1-matrix.json esp32c3 --json # json
cargo run -p validation-report -- docs/coverage/tier1-matrix.json esp32c3 path/to/descriptors
```

A peripheral can carry checks from several authorities; the summary derives ONE status
per distinct peripheral (Fail > Pass > Unrecorded > n/a) and reports coverage over
peripherals, so being validated twice never inflates the score. Real `esp32c3` run:
**51 distinct peripherals, 2 authorities, 97.7% coverage.**

## Status / roadmap

- **Wired (4 authorities):** (1) tier-1 raw-register-vs-TRM matrix; (2) SVD register-layout
  descriptors (`configs/peripherals/<chip>/*.yaml`); (3) silicon reset-conformance from
  committed hw-oracle OpenOCD captures (`scripts/hw-oracle/captures/<chip>/.../reg_oracle.json`)
  — real-hardware ground truth, no board at check time; (4) vendor-stack / integration
  example boots (`examples/*/` targeting the chip, counted by acceptance assertions) —
  device-level behavioral validation, reported separately from per-peripheral coverage.
  Coverage = pass / applicable (excludes `n/a`); `n/a`/`unrecorded` are shown, never dropped.
- **Later (needs infra):** QEMU (Espressif fork) / Renode differential, as another column —
  deferred until a runner exists (same posture as on-silicon HIL).
- **Later (needs infra):** QEMU (Espressif fork) / Renode **differential** — run the same
  firmware on labwired and the reference emulator, diff traces, attach as another column.
  No hardware required; deferred until a runner exists (same posture as on-silicon HIL).
