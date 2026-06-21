# ESP32-C3 MLX90640 thermal-fingerprint

An ESP32-C3 reads a **Melexis MLX90640** 32×24 far-IR thermal-camera array over
the simulated C3 **I²C0** controller, decodes per-pixel °C **on-target** with the
**real, unmodified Melexis driver** (`third_party/mlx90640-library`, compiled for
riscv32), computes a spatial **thermal-fingerprint + fault classification**, and
prints a per-frame verdict over UART. This is the device's intellectual core —
the algorithm that turns a thermal video into a health verdict.

The whole pipeline runs in the LabWired simulator: the firmware does **real I²C
transactions** (status poll → frame read → decode) against the MLX90640 device
model through the C3 I²C0 command-list engine — nothing is bypassed.

## The fault-fingerprint algorithm (blind to the scene)

Per frame, from the 24×32 °C field the firmware computes: the **hotspot** (max °C
+ its row/col), an **ambient** estimate (edge/min), **ΔT** = hotspot − ambient,
the field **mean**, and the hotspot **heating rate** (°C/s) from consecutive
frames. A small state machine runs `IDLE → WARMUP → STABLE → FAULT`, and the
classifier emits one of:

| Fault                | Trigger                                                          |
|----------------------|------------------------------------------------------------------|
| `OVERTEMP`           | hotspot ≥ `TFS_CRITICAL_C`                                        |
| `COOLING_FAILURE`    | past ~3τ the heating rate fails to decay (runaway — cooling lost) |
| `HOTSPOT_EMERGENCE`  | a localized ΔT spike vs the surrounding field                    |

It also computes a **health score 0–100** (margin to critical, penalised by rate)
and a **time-to-limit** estimate.

**Crucially, the algorithm never reads the simulation's scene config.** Every
threshold (`warn_c`, `critical_c`, expected τ, rate floor) is a firmware `#define`
in [`firmware/fingerprint.h`](firmware/fingerprint.h). The `COOLING_FAILURE`
verdict is inferred purely from the observed heating-rate behaviour — **not** from
the scene's `cooling_fault_at_s`, which the firmware cannot see.

## Process-data frame

Each verdict is packed into a **9-byte process-data frame** and emitted as hex on
the human line **and published as IO-Link process data** (see below):

```
[int16 temp_x100][int16 heatrate_x100][u8 state][u8 health][u16 time_to_limit_s][u8 fault<<4 | event_flags]
```

The last byte carries the device's fault classification (`tfs_fault_t`) in the
high nibble and the 5-bit event flags in the low nibble, so a reader gets the
exact verdict the device computed.

Human line per frame:

```
TFS state=<S> hot=<°C>@(r,c) dT=<°C> rate=<°C/s> health=<n> fault=<NAME> PD=<18 hex>
```

## True IO-Link device

The firmware is a **real IO-Link device**: it runs the portable **iolinki**
device stack (`third_party/iolinki`) compiled on-target for rv32imc and publishes
the 9-byte verdict frame as IO-Link **process data** on **UART1** (`0x60010000`,
the C/Q line), while UART0 stays the debug console. A C3 UART PHY shim
([`firmware/phy_c3_iolink.c`](firmware/phy_c3_iolink.c)) bridges UART1 to the
`iolink_phy_api_t` (no timing enforcement — driven by byte arrival, mirroring the
iolink-dido example).

A native IO-Link **master** model is attached to UART1 in the system manifest
(`type: iolink-master`, `pd_in_len: 9`). It drives the wake-up → startup →
OPERATE handshake and cyclically reads the device's process data. The master
**decodes the verdict it received** and logs it to the captured UART channel:

```
MASTER PD=<18 hex>
MASTER VERDICT state=<S> health=<n> fault=<NAME>
MASTER EVENT pending (device diagnostic event)
```

When the fingerprint trips, the firmware raises an IO-Link **TEMPERATURE
diagnostic event** (`iolink_event_trigger`); the device DLL sets the
operate-status EVENT bit, which the master observes on its next cyclic read.

This is the money shot: the **master side** reads the correct verdict over
IO-Link — `STABLE / fault=NONE` in NORMAL, `FAULT / COOLING_FAILURE` then
`OVERTEMP` plus a diagnostic event in FAULT.

## Build

Needs the Espressif RISC-V GCC (`riscv32-esp-elf-gcc`, from PlatformIO or
ESP-IDF). The Makefile auto-discovers it on `PATH` or under `~/.platformio`.

```sh
make -C firmware     # → firmware/thermal_fingerprint.elf (rv32imc, bare ELF)
```

The firmware links a small [`fast_math.c`](firmware/fast_math.c) — a faster
soft-float `pow`/`sqrt` libm shim (the C3 has no FPU). It only supplies faster
libm symbols; **the Melexis driver itself is unmodified**.

## Run

The C3 path boots the bare ELF directly (no boot ROM); UART0 (`0x60000000`) is
captured into the test runner's log.

```sh
# NORMAL: warms up to a bounded plateau, never faults.
cargo run --release -p labwired-cli -- test --script examples/esp32c3-mlx90640-thermal/test.yaml

# FAULT: cooling collapses mid-run; the fingerprint detects the runaway.
cargo run --release -p labwired-cli -- test --script examples/esp32c3-mlx90640-thermal/test-fault.yaml

# As a TRUE IO-Link device: the firmware publishes the verdict as IO-Link
# process data and the MASTER model reads it back (UART1 = IO-Link, UART0 =
# console). Assertions key on the master-observed verdict + diagnostic event.
cargo run --release -p labwired-cli -- test --script examples/esp32c3-mlx90640-thermal/test-iolink.yaml
cargo run --release -p labwired-cli -- test --script examples/esp32c3-mlx90640-thermal/test-iolink-fault.yaml
```

## The two scenarios (money shot)

Both runs share the **same firmware and the same thresholds** — only the scene
differs (`system.yaml` vs `system-fault.yaml`).

**NORMAL** — fan-cooled hotspot warms to a bounded ~46 °C plateau:

```
TFS state=IDLE   hot=43.35@(11,15) dT=43.35 rate=0.00 health=100 fault=NONE
TFS state=WARMUP hot=45.16@(11,16) dT=20.14 rate=0.45 health=96  fault=NONE
TFS state=STABLE hot=45.82@(11,15) dT=20.80 rate=0.16 health=99  fault=NONE
TFS state=STABLE hot=46.07@(11,16) dT=21.05 rate=0.06 health=100 fault=NONE
```

**FAULT** — starts identical, then cooling fails at t=18 s and the hotspot runs
away; the fingerprint flags `COOLING_FAILURE` from the rate, then `OVERTEMP`:

```
TFS state=IDLE   hot=43.35@(11,15) dT=43.35 rate=0.00 health=100 fault=NONE
TFS state=WARMUP hot=45.16@(11,16) dT=20.14 rate=0.45 health=96  fault=NONE
TFS state=STABLE hot=45.82@(11,15) dT=20.80 rate=0.16 health=99  fault=NONE
TFS state=FAULT  hot=66.17@(11,16) dT=41.14 rate=5.08 health=0   fault=COOLING_FAILURE
TFS state=FAULT  hot=73.65@(11,15) dT=48.63 rate=1.87 health=0   fault=OVERTEMP
TFS state=FAULT  hot=76.40@(11,16) dT=51.38 rate=0.68 health=0   fault=OVERTEMP
```

## Files

- `firmware/` — startup (`startup.S`), linker (`c3.ld`), UART (`c3_uart.*`),
  the on-target I²C shim driving the C3 I²C0 engine (`mlx90640_i2c_c3.c`), the
  fast soft-float libm shim (`fast_math.c`), the fingerprint algorithm
  (`fingerprint.*`), and `main.c`. The Melexis driver is compiled from
  `third_party/mlx90640-library` directly by the Makefile.
- `system.yaml` / `system-fault.yaml` — the two thermal scenes.
- `test.yaml` / `test-fault.yaml` — headless runs + assertions.
