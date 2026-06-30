# Opaque Master API Design

## Goal

Make the IO-Link master public API better architected by hiding port and controller internals while keeping embedded-friendly caller-owned storage.

## Decision

Use caller-owned opaque storage, not heap allocation. Public consumers keep declaring `iolink_master_port_t` and `iolink_master_controller_t`, but those types become fixed-size aligned storage unions. The real field layouts move to internal state structs visible only through `master_internal.h`.

## Public API Shape

The existing public functions remain stable. Callers still pass `iolink_master_port_t*` and `iolink_master_controller_t*` to init, tick, process, ISDU, PD, diagnostics, SIO, and controller APIs.

Public code may no longer access fields such as `port.state`, `port.diagnostics`, `port.startup`, or `port.isdu`. Legitimate state is read through existing getters or through new narrow getters only when required by public behavior.

## Internal Architecture

`include/iolinki_master/master.h` owns public enums, public result structs, config structs, API prototypes, and opaque storage types.

`src/master_internal.h` owns the private state structs and conversion helpers from public storage to internal state. Implementation files use internal state pointers for all field access.

`src/master_port.c` remains responsible for lifecycle, startup, RX/TX, ticking, PD, diagnostics, DQ, parameters, and controller behavior in this slice. Later behavior splits can move parameters or SIO to separate files once the public/private boundary is clean.

## Compatibility

This is an API source break only for consumers that directly access public fields. It preserves stack/static allocation and avoids `malloc`, which keeps the API appropriate for MCU firmware.

## Verification

The public-header test must prove the public header no longer exposes struct fields. Existing behavior tests must keep passing after being updated to use public getters or private test access where they intentionally inspect internals.
