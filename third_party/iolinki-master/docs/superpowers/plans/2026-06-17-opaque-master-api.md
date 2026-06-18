# Opaque Master API Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Hide IO-Link master port/controller internals behind caller-owned opaque storage.

**Architecture:** Public `master.h` keeps embedded-friendly storage objects and public APIs. Private `master_internal.h` owns the real state structs plus cast helpers. Implementation files access state through helpers; behavior tests either use public accessors or include the private header when they intentionally inspect state-machine internals.

**Tech Stack:** C99, CMake, CMocka, local `iolinki` CRC/frame helpers.

---

### Task 1: Introduce Opaque Storage Types

**Files:**
- Modify: `include/iolinki_master/master.h`
- Modify: `src/master_internal.h`
- Modify: `src/master_port.c`
- Modify: `src/master_isdu.c`
- Modify: `tests/test_master_public_header.c`

- [ ] Add failing public-header checks that `iolink_master_port_t` has no visible fields and remains stack allocatable.
- [ ] Move the current port/controller field layouts into private state structs in `src/master_internal.h`.
- [ ] Replace public structs with fixed-size aligned storage unions.
- [ ] Add compile-time checks that private state fits public storage.
- [ ] Update implementation files to convert public pointers to private state pointers before field access.
- [ ] Run `cmake --build build --target test_master_public_header`.

### Task 2: Update Behavior Tests For The Private Boundary

**Files:**
- Modify: `tests/test_master_startup.c`
- Modify: `tests/test_master_pd.c`
- Modify: `tests/test_master_isdu.c`
- Modify: `tests/test_master_controller.c`
- Modify: `tests/test_master_tick.c`
- Modify: `tests/test_master_parameters.c`

- [ ] Replace public field reads with public getters where practical.
- [ ] Include `master_internal.h` only in tests that need deliberate internal state inspection.
- [ ] Keep existing behavior expectations unchanged.
- [ ] Run each focused test executable.

### Task 3: Verify And Commit

**Files:**
- Modify only files required by Tasks 1 and 2.

- [ ] Run `cmake --build build`.
- [ ] Run `ctest --test-dir build --output-on-failure`.
- [ ] Run `git diff --check`.
- [ ] Commit with `refactor: hide master internals behind opaque storage`.
