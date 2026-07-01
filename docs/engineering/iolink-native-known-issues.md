# IO-Link Native Path — Known Issues / Tech Debt

Tracked during implementation of the native `iolink-native` stack
(plan 2026-06-18). These are not blockers for the proof-of-concept (all native
tests pass), but should be fixed before the native path is relied on or shipped.

## 1. Bridge routing globals — FIXED (was a real parallel-test failure)

The bridges route PHY callbacks via a "currently active context" pointer
(`g_active` in the master bridge, `g_device_active` in the device bridge). These
were plain process globals; under cargo's default parallel test execution the
writes raced and a callback could read the wrong context → the master-tick tests
failed nondeterministically (observed reproducibly once the branch sat on
`origin/main`).

**Fixed:** both routing pointers are now `__thread` (thread-local), and the
device bridge also passes the owning context through PHY `user` pointers. The
native tests now run multiple real master/device pairs in one process.

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

## 4. Device-stack version coupling — FIXED

The LabWired submodule now points at the merged `iolinki` mainline reentrant
device-stack release, and the vendored `iolinki-master` snapshot is refreshed
from merged upstream `master`. The native build no longer depends on a local,
unpushed device-stack commit.

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

## 7. Multi-port stack backing

The old singleton blocker is gone. The native unit gate exercises multiple real
`iolinki-master` ports and multiple real `iolinki` device contexts in one
process. The firmware-level station gate also runs a real 4-port
`iolinki-master` firmware against four real device-firmware sensor nodes when
the ARM ELFs are built.

## 8. `components/` is a flat junk drawer; IO-Link modules should be grouped

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

## 9. Multiport demo is not browser-runnable — catalog entry skipped

Plan Task 7 calls for a `packages/playground` catalog entry. The real
`BoardConfig` interface requires a prebuilt `demoFirmwarePath` ELF — every
catalog lab is a browser-runnable demo (labs with no firmware surface
"Cannot run: no firmware"). The native multiport demo cannot run in the
browser: it links the `iolinki-master` + GPL `iolinki` device stack, which is
gated out of `labwired-wasm`, and there is no demo firmware crate/ELF for it.

Adding a catalog entry now would ship a broken, unrunnable lab. The parent
`bundled-configs.ts` was therefore left untouched, and the parent catalog test
(plan Task 8 Step 4) is N/A. The core demo assets
(`configs/systems/iolink-multiport-demo.yaml`, `examples/iolink-multiport-demo`)
are committed as scaffolding. A real catalog entry needs a master-side demo
firmware crate compiled to an ARM ELF (the way `al2205-iolink-dido` runs the
device stack as firmware) — out of scope for this plan.

## 10. Limited host-side sensor profiles

`NativeIolinkMasterPort::new_type2_com3` hard-codes M-sequence type 2_1, COM3,
`min_cycle_time=20`, `response_timeout=3`. The device bridge hard-codes a
matching config and a 1-byte proximity PD. Pressure/distance station profiles
are still product metadata unless/until they get real C-stack profile adapters.
