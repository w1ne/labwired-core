# GDB Integration (RSP)

LabWired implements the GDB Remote Serial Protocol (RSP), allowing standard GDB clients (`arm-none-eabi-gdb`, `gdb-multiarch`) to connect for interactive debugging.

## 1. Server Configuration

The GDB server is embedded within the `labwired-cli`. It listens on TCP port `3333` by default.

### Starting the Server manually
```bash
labwired --gdb 3333 --firmware firmware.elf --system system.yaml
```

## 2. Connecting with GDB

Launch your cross-architecture GDB client and connect to the remote target.

```bash
arm-none-eabi-gdb target/firmware.elf
```

Inside GDB:
```gdb
(gdb) target remote localhost:3333
(gdb) load            # Optional: Reloads sections if modified
(gdb) break main
(gdb) continue
```

## 3. Supported Commands

The RSP implementation supports the following subsets:

| Command | Description | Notes |
| :--- | :--- | :--- |
| `g` / `G` | Read/Write All Registers | Full context switch support |
| `p` / `P` | Read/Write Single Register | One 32-bit, little-endian register per packet; `P` uses the `reg=val` form and really writes now, so `set $pc = ...` / `reset $pc` work. |
| `m` / `M` | Read/Write Memory | Used for variable inspection |
| `Z0` / `z0` | Software Breakpoints | Uses BKPT instruction injection |
| `Z2` / `Z3` / `Z4` | Hardware Watchpoints (write / read / access) | Software-tracked, `1..=4096` bytes per range, up to 16 concurrent ranges; CPU accesses only — see [Watchpoints](#5-watchpoints-z2--z3--z4). |
| `vCont` | Continue / Step | Supports single-stepping |
| `qSupported` | Feature Negotiation | XML target description |

## 4. IDE Integration (Cortex-Debug)

For VS Code users preferring the GDB workflow (e.g., for extensive peripheral viewing via SVD), configured `launch.json` as follows:

```json
{
    "type": "cortex-debug",
    "request": "launch",
    "servertype": "external",
    "gdbTarget": "localhost:3333",
    "executable": "${workspaceFolder}/target/firmware.elf",
    "runToEntryPoint": "main",
    "svdFile": "${workspaceFolder}/STM32F103.svd"
}
```

**Note**: You must start the LabWired GDB server manually or via a pre-launch task before starting the debug session.

## 5. Watchpoints (`Z2` / `Z3` / `Z4`)

- `Z2`/`Z3`/`Z4` arm a write / read / access watch on a byte range; `z2`/`z3`/`z4` disarm the exact same request.
- The `kind` field is the watched length in bytes and accepts `1` through `4096` — a single 2 KiB-object request (`Z2,20000000,800`) fits; an empty or >4 KiB range answers `E16` (NonFatal) and does not disarm what was armed before it.
- Up to 16 concurrent ranges; a 17th request is refused the same way.
- Ranges are software-tracked by the bus accessors (no hardware DWT comparators consumed) and fire on CPU accesses (DWT-style comparators: DMA and debugger transfers do not trip them): memory the debugger itself reads or writes (`m`/`M`/`x`) is bypassed, so inspecting a watched object cannot trip its own watchpoint.
- Matching uses the access's base address: an access that begins outside a range and ends inside it does not latch.
- A hit stops the target with a bare SIGTRAP (`T05`/`S05`): there is no `watch:` annotation in the stop reply, and the hit is located through `g` (read PC back; for a Thumb-16 store the stop is post-retirement, so `PC = store + 2` and the stored value is already visible in memory).

**Known `gdbstub_arch` CPSR raw-id quirk**: the `g` reply ordering puts `cpsr` after 25 FPU placeholder words (the `xx` bytes between PC and CPSR), so a client that addresses CPSR by the raw id its own `g` layout implies (decimal raw id 41, sent as `P 29`) gets NonFatal — raw ids outside the stub's `0..=25` space do not map to a register. PC (15) and r0–r15 are unaffected; the `reset $pc` path (`P f=...`) works.

## 6. Conformance matrix

| Scenario (issue) | RSP (CI) — `rsp_conformance.rs` | gdb-multiarch | arm-none-eabi-gdb | CLion | VS Code cortex-debug | LLDB |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| WFI reset-PC stays responsive (#51) | `reset_pc_while_stopped_in_wfi_stays_responsive` | smoke: `scripts/gdb-conformance.sh` (skips without the tool) | untested | untested | untested | untested |
| Continue while halted (#52) | `continue_while_halted_without_prior_start` | smoke: `scripts/gdb-conformance.sh` (skips without the tool) | untested | untested | untested | untested |
| Step into fault survives (#74) | `step_into_fault_reports_fault_and_target_survives`, `step_into_fault_with_faults_disabled_reports_segv` | smoke: `scripts/gdb-conformance.sh` (skips without the tool) | untested | untested | untested | untested |
| 2 KiB watchpoint (#70) | `watchpoint_on_a_2kib_object_stops_the_core` | smoke: `scripts/gdb-conformance.sh` (skips without the tool) | untested | untested | untested | untested |
| Exactly one stop reply (#978) | `exactly_one_stop_reply_per_stop` | smoke: `scripts/gdb-conformance.sh` (skips without the tool) | untested | untested | untested | untested |
| Attach/detach stress (#57) | `attach_detach_stress_under_breakpoint_traffic` | smoke: `scripts/gdb-conformance.sh` (skips without the tool) | untested | untested | untested | untested |

Every cell is either backed by CI or marked `untested` — no client claims without a run. The `gdb-multiarch` column is one batch session (connect, read PC, step, memory read, detach), not a per-scenario client answer; the script skips with exit 0 when the tool is absent, so those cells only carry a pass after a machine with the tool actually ran it.

**Receipts**: `cargo test -p labwired-gdbstub --test rsp_conformance` (the six CI scenarios; the optional client smoke is `#[ignore]`d under `gdb_multiarch_batch_smoke`); `bash scripts/gdb-conformance.sh` (the optional client smoke itself).
