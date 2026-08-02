# LabWired Core Roadmap

Where LabWired Core is heading as a production-ready environment for
professional firmware simulation. Shipped work lives in
[CHANGELOG.md](CHANGELOG.md); current model accuracy and known gaps live in
[FIDELITY.md](FIDELITY.md).

## Now — v0.19.x

- **Multi-node CI runner**: run a full `inputs.env` world from YAML in GitHub
  Actions or an OCI image and publish a self-contained report bundle without
  rebuilding LabWired.
- **Environment result contract**: a stable, machine-readable result schema
  with per-node provenance and declared fidelity gaps.
- **Declarative modelling**: bus-agnostic component IR and one-step SVD
  ingestion, so a new chip or peripheral is a data path rather than a code
  path.

## Next

- **Timing accuracy**: cycle models for pipeline stalls and bus contention.
- **Fault injection API**: programmatic induction of hardware faults for
  safety-critical testing.
- **RTOS awareness**: task-list inspection for FreeRTOS and Zephyr.
- **Analyzer depth**: richer trace export and inspection across every modelled
  bus.

## Later — v1.0

- **ISO 26262 readiness**: tool qualification kits and traceability reporting.
- **Cloud fleet execution**: scalable, multi-tenant simulation orchestration.
- **AI-accelerated modelling**: automated extraction of behavioural models from
  datasheets.

---

*Features and ordering are subject to change based on community feedback and
project evolution.*
