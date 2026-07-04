# Contributing to iolinki-master

## Getting started

`iolinki-master` builds against a **sibling checkout** of the `iolinki` device
repository for the shared CRC/frame helpers (and, for the real-device test, the
device stack sources). Clone it next to this repo:

```bash
git clone git@github.com:w1ne/iolinki-master.git
git clone -b develop git@github.com:w1ne/iolinki.git   # sibling ../iolinki
cd iolinki-master
```

Point elsewhere with `-DIOLINKI_DEVICE_DIR=/path/to/iolinki` if it is not a sibling.

You need CMake (≥ 3.10), a C99 compiler, and CMocka for the tests.

## Build / test loop

The canonical loop — run it before every commit:

```bash
cmake -S . -B build
cmake --build build
ctest --test-dir build --output-on-failure
git diff --check
```

`ctest` exercises the CMocka unit/protocol suites, the fake-device harness, the
real-device in-memory harness, and the runnable examples (see
[`TESTING.md`](TESTING.md) for the target list). Passing local tests means the
behavior is *locally* verified — not hardware-tested or conformance-validated. Keep
that distinction in [`IMPLEMENTATION_STATUS.md`](IMPLEMENTATION_STATUS.md) honest.

## Quality gate

Run the strict gate before opening a PR:

```bash
./check_quality.sh
```

It builds with warnings-as-errors (`-Wall -Wextra -Werror` and friends) and runs
static analysis on top of the test suite. CI runs the equivalent on every push and
PR: the `cmake-ctest` job (which fetches the `iolinki` device stack and builds +
tests the master) and the `labwired-real-firmware-model` job (which runs this master
against the real device firmware over an on-wire model). Both must be green.

Install the pre-commit hooks so formatting and hygiene are checked locally:

```bash
pre-commit install
```

The hooks cover trailing whitespace / EOF, YAML and large-file checks, and
`clang-format`, and block direct commits to protected branches.

## Style

- **Portable Standard C (C99).** No OS-specific calls in `src/`; no board headers in
  `src/master_*.c` (board code goes through the PHY boundary — see
  [`PHY_BOUNDARY.md`](PHY_BOUNDARY.md)).
- **No dynamic memory.** No `malloc`/`calloc`/`free` anywhere in `src/` or
  `include/`; all state is caller-owned opaque storage.
- **Fixed-width types** from `<stdint.h>`. **No globals.** **Checked inputs.**
- **No compiler warnings.** Formatting is enforced by `.clang-format`; do not
  hand-format against it.
- **Named result codes.** Return the `IOLINK_MASTER_*` constants, never bare magic
  integers, and document the return contract of every public function in the header.

## Branch and PR conventions

- Single long-lived branch: `master`. Do commit to it directly for release-ready
  work; land features through a short-lived feature branch → PR → merge to `master`.
- Conventional-commit subjects (`feat`, `fix`, `docs`, `refactor`, `test`, `chore`,
  `style`), scoped where useful, e.g.:

  ```text
  feat(isdu): add Data Storage restore readback verification
  fix(port): reset rx_retry_count after a good frame
  docs(porting): document t_REN half-duplex settling
  ```

- Update `CHANGELOG.md` under `[Unreleased]` in the same PR as a user-visible change,
  and update `IMPLEMENTATION_STATUS.md` when a feature's status changes.
- Prefer merge over rebase for integrating branches.
- **No AI-authored attribution** in commits, PR descriptions, or code comments.

## Reporting issues

File issues on GitHub with a description, reproduction steps, and expected vs actual
behavior. For suspected vulnerabilities, follow [`SECURITY.md`](../SECURITY.md)
(private reporting) instead of opening a public issue.
