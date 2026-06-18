# IO-Link Native Path — Known Issues / Tech Debt

Tracked during implementation of the native `iolink-native` stack
(plan 2026-06-18). These are not blockers for the proof-of-concept (all native
tests pass), but should be fixed before the native path is relied on or shipped.

## 1. Bridge globals are not thread-safe (latent test flakiness)

Both bridges use process-global mutable state:

- `g_active` in `native/iolink_master_bridge.c`
- `g_device_active` / `g_device_in_use` in `native/iolink_device_bridge.c`
- the device stack itself (`g_dll_ctx`) is a true global (see issue #6).

The master bridge sets `g_active = c` around each `iolink_master_*` call so the
PHY `send`/`recv_byte` callbacks can find the right queue. If two
`NativeIolinkMasterPort`s are driven from different threads (cargo runs tests in
parallel by default), the `g_active` writes race and a callback can read the
wrong context → corruption or SIGSEGV.

It has NOT reproduced yet (the per-call C critical sections are short), but it is
real. **Fix:** make `g_active` `thread_local`. The device stack is inherently
singleton, so its instance must stay serialized regardless.

## 2. C stack stores PHY/port by pointer — ownership is fragile

`iolink_master_init` does `state->phy = phy` (pointer, not copy) and copies the
config by value; `iolink_init` behaves the same. The first bridge draft passed a
stack-local `iolink_phy_api_t` and crashed on the next tick (dangling pointer).

Fixed by owning the `phy` struct inside the bridge context. Two fragilities
remain:
- The context lives in a Rust `Vec<u64>`; the C stack holds `&ctx->phy` pointing
  into that heap buffer. It is only safe because the Vec is never reallocated.
  A `Box<[u64]>` (or an explicitly pinned allocation) would document the intent
  and remove the foot-gun.
- Any future bridge field the C stack retains by pointer must follow the same
  rule. Worth a comment in the header.

## 3. Wake-up `0x55` is filtered at the wire boundary

The master models the C/Q wake-up pulse as a single `0x55` UART byte
(`bridge_wake_up`). The device bridge drops a standalone `0x55` in
`lw_iold_feed_master`, mirroring the reference `master_loopback_demo` PHY. This
is safe (real frames are always ≥2 bytes) but is an artifact of modeling a
current pulse as a byte. A cleaner model would represent wake-up out-of-band
rather than as in-band data.

## 4. Device-stack version coupling is implicit and fragile

The vendored master (`iolinki-master` @ `5d3ebe9`) needs the device stack's
`frame.c`, which only exists at device-stack commit `aec4803` — a **local,
unpushed** commit. A fresh `git submodule update` checks out the older recorded
pin (`4b94a35`, no `frame.c`) and the native build breaks. The required
device-stack commit must be pushed and the submodule pin bumped, or the coupling
documented in the build. Right now it is tribal knowledge.

## 5. `iolinki-master` is vendored as a flat copy, not a submodule

Task 0 rsync-copies `iolinki-master` into `third_party/iolinki-master` with a
`SOURCE_COMMIT` marker. It will not track upstream; drift is manual. Acceptable
for the proof but should become a submodule (consistent with
`third_party/iolinki`) before it is maintained long-term.

## 6. GPL linkage

The native build links the GPL-3.0-or-later `iolinki` device stack (and the
shared `frame.c`/`crc.c`/headers). It is correctly gated behind the non-default
`iolink-native` feature and must never be enabled for `labwired-wasm` or any
distributable default build. See iolink-device-stack-isolation.md.

## 7. Multi-port is modeled, not stack-backed

`IolinkStation` (Task 6) is a pure-Rust product/profile model. Because the
device stack is singleton, it does NOT instantiate one real device stack per
port. The "without sharing state" guarantee in its test is true of the Rust
wrapper only — it is not evidence that four real device stacks coexist. Real
multi-port stack-backed behavior needs the device-stack reentrancy work.

## 9. `components/` is a flat junk drawer; IO-Link modules should be grouped

`peripherals/components/mod.rs` is a flat list of ~30 `pub mod` declarations
spanning unrelated device families. This implementation added two more
top-level siblings (`iolink_native`, `iolink_station`) next to the existing
`iolink_master`. A flat `pub mod` list is idiomatic Rust, but the IO-Link
modules are one cohesive cluster and should live under a single `iolink/`
submodule:

```
components/iolink/{mod.rs, master.rs, native.rs, station.rs}
```

so `components/mod.rs` carries one `pub mod iolink;` line. Done flat here to
match the plan and the pre-existing `iolink_master.rs` placement. Regrouping
also touches the kit registry and the public re-export path
`peripherals::components::IolinkMaster`.

## 8. Single hard-coded master configuration

`NativeIolinkMasterPort::new_type2_com3` hard-codes M-sequence type 2_1, COM3,
`min_cycle_time=20`, `response_timeout=3`. The device bridge hard-codes a
matching config and a 1-byte proximity PD. Any other device profile
(pressure/distance from the station model, different PD lengths, ISDU) is not
wired through the bridge yet — those station "profiles" have no native backing.
