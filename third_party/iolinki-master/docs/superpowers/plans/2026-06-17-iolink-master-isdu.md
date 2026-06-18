# IO-Link Master ISDU Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the current ISDU skeleton into a real nonblocking master-side ISDU client layered over cyclic OD bytes.

**Architecture:** `iolink_master_process()` emits process data plus OD bytes from the active ISDU transaction. `iolink_master_on_rx()` keeps latching PD input and feeds OD response bytes back into that transaction. Public read/write calls start a transaction, poll pending completion, and return copied data or protocol errors.

**Tech Stack:** C99, CMake, cmocka, local `iolinki` frame/protocol helpers.

---

### Task 1: Pin Down ISDU Wire Behavior

**Files:**
- Modify: `tests/test_master_isdu.c`

- [ ] Add tests proving read request OD emission, read response completion, write request payload emission, busy rejection, buffer-too-small handling, and device ISDU error handling.
- [ ] Run `cmake --build build --target test_master_isdu && ./build/tests/test_master_isdu`.
- [ ] Confirm the new tests fail because the skeleton does not emit or complete ISDU transactions.

### Task 2: Add Master Transaction State

**Files:**
- Modify: `include/iolinki_master/master.h`
- Modify: `src/master_internal.h`

- [ ] Add small enums and fields for active operation, phase, request buffer, response buffer, sequence number, caller length, and error code.
- [ ] Keep public APIs unchanged.
- [ ] Run `cmake --build build --target test_master_public_header`.

### Task 3: Integrate ISDU With Cyclic OD

**Files:**
- Modify: `src/master_port.c`
- Modify: `src/master_isdu.c`

- [ ] Let `master_port.c` ask `master_isdu.c` for the next OD bytes before encoding cyclic frames.
- [ ] Let `master_port.c` pass decoded response OD bytes to `master_isdu.c`.
- [ ] Implement nonblocking `read_isdu` and `write_isdu` start/poll semantics.
- [ ] Run `cmake --build build --target test_master_isdu && ./build/tests/test_master_isdu`.

### Task 4: Verify Master Library

**Files:**
- Modify if needed: `README.md`

- [ ] Run `cmake --build build`.
- [ ] Run `./build/tests/test_master_startup`.
- [ ] Run `./build/tests/test_master_pd`.
- [ ] Run `./build/tests/test_master_isdu`.
- [ ] Run `./build/tests/test_master_public_header`.
- [ ] Note that full `ctest` may still include unbuilt dependency tests unless CTest registration is cleaned up separately.
