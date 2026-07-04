# iolinki-master

`iolinki-master` is a standalone IO-Link master stack. It is intentionally split
from the device-oriented [`iolinki`](https://github.com/w1ne/iolinki) repository
and reuses only the narrow shared pieces needed for CRC, frame handling, PHY
contracts, and IO-Link constants.

The master API is built around caller-owned opaque storage. Public users allocate
`iolink_master_port_t` or `iolink_master_controller_t`; private state lives in
`src/` and is not exposed through the public header.

> **Status: `v0.1.0` — protocol-core, simulation-validated.** The stack is
> exercised against a co-designed simulated device and an on-wire firmware model,
> not yet against real IO-Link silicon. It is **not** a conformant hardware master
> yet (open: wake-response baud detection, physical wake-pulse timing, official
> conformance). See [`docs/IMPLEMENTATION_STATUS.md`](docs/IMPLEMENTATION_STATUS.md)
> and [`CHANGELOG.md`](CHANGELOG.md).

Documentation:

- [`docs/API.md`](docs/API.md) — public API tour and a compiling example
- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — layered design
- [`docs/PORTING.md`](docs/PORTING.md) — implement the PHY contract for a board
- [`docs/CONTRIBUTING.md`](docs/CONTRIBUTING.md) — build/test loop and quality gate
- [`docs/IMPLEMENTATION_STATUS.md`](docs/IMPLEMENTATION_STATUS.md) — honest status ledger
- [`docs/ROADMAP.md`](docs/ROADMAP.md), [`docs/TESTING.md`](docs/TESTING.md),
  [`docs/RELEASE_STRATEGY.md`](docs/RELEASE_STRATEGY.md)

The build needs a local checkout of the `iolinki` device repository for the
shared CRC/frame helpers (and, for tests, the real device stack). By default it
is expected as a sibling of this repository:

```sh
git clone -b develop git@github.com:w1ne/iolinki.git ../iolinki
```

Build and test:

```sh
cmake -S . -B build
cmake --build build
ctest --test-dir build --output-on-failure
```

The build compiles only the narrow shared helper sources from the local `iolinki`
checkout into the master build. It should not link or expose the full device
stack.

Public APIs return named integer-compatible result constants such as
`IOLINK_MASTER_STATUS_OK`, `IOLINK_MASTER_STATUS_PENDING`, and
`IOLINK_MASTER_ERR_INVALID_ARG`.

Runnable examples are built by default:

- `master_loopback_demo`: one IO-Link port startup and cyclic process-data flow.
- `master_4port_controller_demo`: mixed 4-port controller setup with IO-Link,
  DI, DQ, and deactivated ports.

To point at another local `iolinki` checkout:

```sh
cmake -S . -B build -DIOLINKI_DEVICE_DIR=/path/to/iolinki
```

## Related Projects

- **[iolinki](https://github.com/w1ne/iolinki)** — the companion IO-Link
  **device** stack. This repository builds against it for the shared
  CRC/frame/PHY helpers, and CI runs both stacks against each other over a
  simulated wire (real firmware, multi-port station) in LabWired.

## Security

This repository has its own coordinated-disclosure policy in
[`SECURITY.md`](SECURITY.md) — use GitHub private vulnerability reporting. It
ships a master-specific STRIDE [threat model](docs/security/THREAT_MODEL.md), a
[CRA overview](docs/security/CRA.md), and per-release SBOMs (CycloneDX 1.6 +
SPDX 2.3) attached to each tagged release.

## License

`iolinki-master` follows the same licensing model as
[`iolinki`](https://github.com/w1ne/iolinki): dual-licensed under the **GPLv3**
(free, for open-source/GPLv3 use) and a **commercial license** (for
closed-source / proprietary products that cannot accept the GPLv3 copyleft).
Shipping a proprietary product? Email **andrii@shylenko.com**. See
[`LICENSE`](LICENSE) and [`LICENSE.COMMERCIAL`](LICENSE.COMMERCIAL).
