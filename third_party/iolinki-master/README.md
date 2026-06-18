# iolinki-master

`iolinki-master` is a standalone IO-Link master stack. It is intentionally split
from the device-oriented `iolinki` repository and reuses only the narrow shared
pieces needed for CRC, frame handling, PHY contracts, and IO-Link constants.

The master API is built around caller-owned opaque storage. Public users allocate
`iolink_master_port_t` or `iolink_master_controller_t`; private state lives in
`src/` and is not exposed through the public header.

Track implementation status and next work here:

- [`docs/IMPLEMENTATION_STATUS.md`](docs/IMPLEMENTATION_STATUS.md)
- [`docs/ROADMAP.md`](docs/ROADMAP.md)
- [`docs/TESTING.md`](docs/TESTING.md)

The default local dependency path is:

```sh
/home/andrii/projects/labwired/core/third_party/iolinki
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
