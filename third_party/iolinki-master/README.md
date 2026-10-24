# iolinki-master

The companion device stack is [iolinki](https://github.com/w1ne/iolinki).

`iolinki-master` is a standalone IO-Link master stack. It is intentionally split
from the device-oriented [`iolinki`](https://github.com/w1ne/iolinki) repository
and reuses only the narrow shared pieces needed for CRC, frame handling, PHY
contracts, and IO-Link constants.

The master API is built around caller-owned opaque storage. Public users allocate
`iolink_master_port_t` or `iolink_master_controller_t`; private state lives in
`src/` and is not exposed through the public header.

> **Status: protocol-core, simulation-validated.** The stack speaks the IO-Link
> V1.1.5 wire (checksum A.1.6, ISDU transport 7.3.6, diagnosis-channel events
> 7.3.8, Table A.10 codes and the C6 timing rules) and is exercised against the
> `iolinki` device stack and an on-wire firmware model — not yet against real
> IO-Link silicon (open: physical wake-pulse timing, official conformance). See
> [`docs/IMPLEMENTATION_STATUS.md`](docs/IMPLEMENTATION_STATUS.md) and
> [`CHANGELOG.md`](CHANGELOG.md).

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

For new quotes using **2026-10-02-indie-company-v4**:

| Package | Holder and product rights | Included assistance |
| --- | --- | --- |
| **Indie — €1,399** | Named individual developing their own independent product family, including as a sole trader. Employer or separate-company rights require Company licensing. | Two onboarding hours; 48-hour email response target. |
| **Company — €4,699** | Named legal company; unlimited authorized employees and contractors working on one agreed product family. | Eight total scoped engineering hours, including quarterly technical reviews during the first year; 24-hour priority email response target. |

Both packages include the same master source, documentation and reference
release, perpetual royalty-free rights for unlimited units and customers within
the agreed family, and one year of updates and bug fixes. Optional annual
updates/support renewal is 20% of the original fee, with coverage agreed in the
quote. The named family includes variants, successors and hardware revisions
within that commercial range; an unrelated line needs separate scope.

Agree the target, compiler, PHY and tasks before booking. Assistance covers
setup/build configuration, MCU/PHY callbacks and wiring review, an integration
checklist and written findings within the allowance. Larger ports and validation
work are quoted separately; reproducible stack bug corrections do not consume
custom engineering hours. Broader families, subsidiaries and SDK redistribution
use an Enterprise quote.

These packages license **iolinki-master only**; the companion device stack is
separately licensed. Previously accepted agreements retain their existing rights,
including older unlimited-deployment and developer-seat grants. See the
[commercial terms](LICENSE.COMMERCIAL) and [prior published offer](docs/licenses/commercial-pre-v4.md).
