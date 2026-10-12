# Native ARM system backend implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox syntax for tracking. Root owns commits and merges.

**Goal:** Provide an isolated, callback-backed Cortex-A7 system CPU with architectural MMU, exceptions and level interrupts for the RV1103 Linux machine.

**Architecture:** Vendor unmodified QEMU-derived tlib at `167decf9129758829762e582939128c0694e90d6`. A feature-gated small Rust crate builds and embeds a shared library; each CPU loads its own temporary inode, isolating upstream globals. A C guard contains fatal errors and checks callback failures only after Rust has returned. The core adapter keeps LabWired's Bus authoritative.

**Tech Stack:** Rust, C11, CMake, tlib, libloading 0.8, tempfile 3, existing Cpu/Bus interfaces. Initial host: x86_64 Linux; no default/browser native dependency.

## Task 1: Native library and safe scoped execution

Files: create `crates/arm-system/{Cargo.toml,build.rs,src/lib.rs,native/CMakeLists.txt,native/bridge.c,native/bridge.h,tests/architecture.rs}`; add workspace member in `Cargo.toml`; update `Cargo.lock`; vendor `third_party/tlib/` and `third_party/tlib-PROVENANCE.md`.

- [ ] Write architectural tests using the public API below before the implementation. Begin with `MOV r0,#42; STR r0,[r1]; LDR r2,[r1]`, assert exactly three retired instructions and a single width-4 physical read/write at `0x100000`. Retain red output from the missing backend.
- [ ] Run `cargo test -p labwired-arm-system --features native --test architecture -- --nocapture`; expected initial failure because the API is absent.
- [ ] Implement this API (all errors are owned strings):

```rust
pub trait PhysicalBus {
    fn read(&mut self, address: u64, width: u32, state: u64) -> Result<u64, String>;
    fn write(&mut self, address: u64, width: u32, value: u64, state: u64) -> Result<(), String>;
    fn cp15_read(&mut self, instruction: u32, wide: bool) -> Result<u64, String>;
    fn cp15_write(&mut self, instruction: u32, wide: bool, value: u64) -> Result<(), String>;
}
pub struct Execution { pub retired: u64, pub waiting: bool }
// ArmCpu owns its library and file. CPSR uses public index 16, mapped to tlib 25.
// Methods: new() -> Result<Self,String>; register(u8) -> Result<u32,String>;
// set_register(u8,u32) -> Result<(),String>; executable_range(u64,u64)
// -> Result<(),String>; interrupt_levels(bool,bool) -> Result<(),String>;
// execute(&mut dyn PhysicalBus,u32) -> Result<Execution,String>;
// invalidate_code() -> Result<(),String>; reset() -> Result<(),String>.
```

Use `tlib_register_access_flags_for_range(start,length,1)` for executable callback memory; do not call `tlib_map_range` or expose Rust RAM pointers. Reads/writes preserve widths 1/2/4/8 and native security state. CP15 callbacks forward to PhysicalBus rather than returning zero. Reject null/unbound callbacks and unexpected host RAM pointer requests. `execute` installs a stack callback context, catches Rust panic, stores the first error, and clears all native callback pointers on every return. The C thunk checks that error after Rust returns and unwinds inside C. Native init/reset/register/range/invalidate/dispose operations also run inside a C fatal-error guard. Never longjmp across a Rust callback. A failed native call poisons the CPU; refuse execution until reset/recreation. LPAE is outside the supported capability.

The crate defaults to no native feature. Enabling `native` on an unsupported host produces an explicit build error. Build the vendored library with CMake, HOST_ARCH=i386, HOST_WORD_SIZE=64, TARGET_ARCH=arm, TARGET_WORD_SIZE=32, `-Bsymbolic`; embed bytes from OUT_DIR. Retain all licenses and the verified source archive SHA `8db4d1fe0a316eec5dcae83bd4d2757b5172332493d6fdecfd9d4b81cb7a77a3`. No build-time network access.

- [ ] Add and execute separate tests for SVC vector 8/SVC mode, unmapped short-descriptor translation to guest data abort, real VA `0x80000300` to PA `0x300` page-table walk, IRQ vector 24/IRQ mode with deassertion, Thumb-2, WFI, callback errors, callback panic, self-modifying code, explicit host-write invalidation and swapped buses.
- [ ] Interleave two CPUs with different memory/register values, drop one and continue the other; repeat concurrently on two threads. Move an idle CPU to another thread and resume it. All tests use genuine guest instructions and physical memory callbacks.
- [ ] Verify `cargo test -p labwired-arm-system --features native --test architecture`, `cargo test -p labwired-arm-system`, formatting and diff checks. Keep test builds single-job and bounded because disk is constrained.
- [ ] Root reviews and commits `feat: embed isolated QEMU-derived ARM system backend` after spec and quality review.

## Task 2: LabWired Cpu adapter

Files: modify `crates/core/{Cargo.toml,src/cpu/mod.rs}`; create `crates/core/src/cpu/cortex_a_system.rs`, `crates/core/tests/qemu_arm_system.rs`; modify `crates/core/src/lib.rs` only for additive IRQ/FIQ level capability with default behavior preserving existing CPUs.

- [ ] Add a feature-required integration test that loads the architectural instruction fixture through an actual Bus and calls Cpu::step_batch. Require translated physical accesses, real retired counts, observers only for retired instructions and explicit snapshot refusal.
- [ ] Run `cargo test -p labwired-core --features qemu-arm-system --test qemu_arm_system`; retain the missing-adapter failure.
- [ ] Add optional dependency `labwired-arm-system` and feature `qemu-arm-system = ["dep:labwired-arm-system", "labwired-arm-system/native"]`. Gate exports on native feature and keep the existing Unicorn backend unchanged.
- [ ] Implement `CortexA7System` with scoped PhysicalBus forwarding and exact-width Bus reads/writes. CP15 timer callbacks must reach the RV1103 timer owner, never a second private timer. Forward IRQ/FIQ levels, including deassertion. Record errors from infallible Cpu register setters and return them on the next fallible execution call. Refuse incomplete banked/CP15 runtime snapshot restoration.
- [ ] Verify feature-on architecture tests plus existing feature-off/default/browser compilation and the original qemu-arm fixture. Root commits only after both reviews.

## Subsequent integration plans

The platform plan must add the RV1103 physical map, GIC-400, architectural timer and DW UART2 first, then pinned Linux artifacts and direct-boot loader. Acceptance requires genuine kernel UART log, PID1 identity, syscall and a guest sleep whose uptime advances through timer IRQs. An initramfs and fixed clock are disclosed reductions; SD/ROM/vendor boot are separate gates.

The basket plan follows with RK3x I2C3, UART4 and existing ADXL345/ESP-AT component attachment. Linux guest read/write operations must cross those controllers into the shared component state. The stock Alpine first-boot kernel has I2C_RK3X disabled; basket acceptance therefore needs a kernel enabling the real driver. No claimed Linux circuit acceptance may substitute host-only register stimulation.
