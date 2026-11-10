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
high nibble and the four low event-flag bits in the low nibble. The fifth
internal flag (`FAULT_LATCH`) is represented by `state=FAULT`, not a separate
wire bit.

Human line per frame:

```
TFS t_ms=<time> state=<S> hot=<°C>@(r,c) dT=<°C> rate=<°C/s> health=<n> fault=<NAME> PD=<18 hex>
```

## IO-Link device and master integration

The firmware is a **real IO-Link device**: it runs the portable **iolinki**
device stack (`third_party/iolinki`) compiled on-target for rv32imc and publishes
the 9-byte verdict frame as IO-Link **process data** on **UART1** (`0x60010000`,
the C/Q line), while UART0 stays the debug console. A C3 UART PHY shim
([`firmware/phy_c3_iolink.c`](firmware/phy_c3_iolink.c)) bridges UART1 to the
`iolink_phy_api_t` through the native ESP32-C3 FIFO and STATUS registers. UART1
uses fixed COM2 configuration. The device DLL's timing enforcement is disabled;
this fixture tests byte-stream protocol and process-data integration, not the
physical C/Q wake-up pulse, transceiver, parity handling, or IO-Link timing
conformance. The native UART currently models ten bit times per byte.

A native IO-Link **master** model is attached to UART1 in the system manifest
(`type: iolink-master`, `pd_in_len: 9`, `m_seq_type: 6`, `od_len: 1`). Both
endpoints use TYPE_2_V, so event diagnosis keeps the typed message format
required in OPERATE. It drives the wake-up → startup →
OPERATE handshake and cyclically reads the device's process data. The master
**decodes the verdict it received** and logs it to the captured UART channel:

```
MASTER PD=<18 hex>
MASTER VERDICT state=<S> health=<n> fault=<NAME>
MASTER EVENT pending (device diagnostic event)
```

On the first fault, the firmware raises an IO-Link diagnostic Error event
(`iolink_event_trigger`). `OVERTEMP` maps to temperature overload (`0x4000`);
rate and shape diagnoses map to application fault (`0x8C00`). The exact
classification remains in the process data. The device DLL sets the
operate-status EVENT bit, which the master observes on its next cyclic read.

The assertions check the **master-observed** verdict: `STABLE / fault=NONE` in
NORMAL, and `FAULT / COOLING_FAILURE` followed by `OVERTEMP` plus a diagnostic
event in FAULT.

## Device time and completion

CONTROL1 selects **2 Hz** sampling. Each sensor subpage becomes ready after
500 ms of simulated time; acknowledging STATUS does not advance the scene.
The firmware reads SYSTIMER UNIT0 for the sample timestamp used by the
unchanged fingerprint algorithm. It waits between polls with SYSTIMER TARGET0
and `WFI`, so the simulator can skip idle CPU cycles while advancing the sensor,
UART, and timer together. The CPU remains at 160 MHz.

Each run acquires **52 subpages over 26 seconds** and emits
`TFS DONE frames=52 faults=<count>`. Every script requires that completion
marker. NORMAL additionally requires `faults=0`; FAULT requires an ordered
healthy → cooling-failure → overtemperature sequence before completion.
The IO-Link variants require the corresponding verdicts from the receiving
master, not just the firmware console.

The calibration EEPROM is readable before the first conversion. Its PTAT
reference is initialized at construction and remains unchanged while samples
arrive, matching the firmware's boot-time calibration cache.

## Build

Needs the Espressif RISC-V GCC (`riscv32-esp-elf-gcc`, from PlatformIO or
ESP-IDF). The Makefile auto-discovers it on `PATH` or under `~/.platformio`.
It selects `rv32imc_zicsr_zifencei/ilp32` explicitly so GCC 15 uses the C3
soft-float libraries rather than its default multilib with floating-point CSR
instructions.

From the repository root:

```sh
git submodule update --init third_party/iolinki
make -C examples/esp32c3-mlx90640-thermal/firmware
```

The firmware links a small [`fast_math.c`](firmware/fast_math.c) — a faster
soft-float `pow`/`sqrt` libm shim (the C3 has no FPU). It only supplies faster
libm symbols; **the Melexis driver itself is unmodified**.

## Run

The C3 path boots the bare ELF directly (no boot ROM); UART0 (`0x60000000`) is
captured into the test runner's log.

Build and run the CLI from the same source checkout. These scripts use the
current CLI's explicit wall-bounded step budget; released CLI v0.25.0 still
rejects budgets above 50 million steps. Each run is bounded by 1 billion
interpreted instructions, 5 billion device cycles, and 180 seconds of host time.
The cycle budget includes timer-backed idle time. A timeout, engine fault,
missing verdict, or incomplete frame count fails the run.

```sh
# NORMAL: warms up to a bounded plateau, never faults.
cargo run --release -p labwired-cli -- test --script examples/esp32c3-mlx90640-thermal/test.yaml

# FAULT: cooling collapses mid-run; the fingerprint detects the runaway.
cargo run --release -p labwired-cli -- test --script examples/esp32c3-mlx90640-thermal/test-fault.yaml

# IO-Link integration: the firmware publishes the verdict as IO-Link
# process data and the MASTER model reads it back (UART1 = IO-Link, UART0 =
# console). Assertions key on the master-observed verdict + diagnostic event.
cargo run --release -p labwired-cli -- test --script examples/esp32c3-mlx90640-thermal/test-iolink.yaml
cargo run --release -p labwired-cli -- test --script examples/esp32c3-mlx90640-thermal/test-iolink-fault.yaml
```

## The two scenarios

Both runs share the **same firmware and the same thresholds** — only the scene
differs (`system.yaml` vs `system-fault.yaml`).

The excerpts below are from completed runs of the same source-built ELF. The
process-data hex column is omitted here; it remains in each captured UART line.
The first chess subpage updates half the pixel field; the next subpage fills the
other half. The initial IDLE sample therefore does not yet have a complete
ambient-field estimate.

**NORMAL** — the hotspot settles near 46.2 °C. All 52 updates report no fault:

```
TFS t_ms=501 state=IDLE hot=27.51@(11,15) dT=27.51 rate=0.00 health=100 fault=NONE
TFS t_ms=1001 state=WARMUP hot=29.71@(11,16) dT=4.68 rate=4.39 health=65 fault=NONE
TFS t_ms=12001 state=STABLE hot=45.16@(11,16) dT=20.14 rate=0.28 health=98 fault=NONE
TFS t_ms=20001 state=STABLE hot=46.07@(11,16) dT=21.05 rate=0.03 health=100 fault=NONE
TFS t_ms=26001 state=STABLE hot=46.18@(11,15) dT=21.16 rate=0.00 health=100 fault=NONE
TFS DONE frames=52 faults=0
```

**FAULT** — cooling fails at 18 s. The unchanged classifier reports
`COOLING_FAILURE` at 18.501 s, then `OVERTEMP` at 24.001 s:

```
TFS t_ms=12001 state=STABLE hot=45.16@(11,16) dT=20.14 rate=0.28 health=98 fault=NONE
TFS t_ms=18001 state=STABLE hot=45.98@(11,16) dT=20.96 rate=0.06 health=99 fault=NONE
TFS t_ms=18501 state=FAULT hot=49.74@(11,15) dT=24.72 rate=7.51 health=40 fault=COOLING_FAILURE
TFS t_ms=22501 state=FAULT hot=67.61@(11,15) dT=42.58 rate=2.75 health=0 fault=COOLING_FAILURE
TFS t_ms=24001 state=FAULT hot=70.86@(11,16) dT=45.84 rate=1.90 health=0 fault=OVERTEMP
TFS t_ms=26001 state=FAULT hot=73.67@(11,16) dT=48.65 rate=1.14 health=0 fault=OVERTEMP
TFS DONE frames=52 faults=16
```

The completed verification runs pass all **34 script checks**: 7 NORMAL,
10 FAULT, 7 IO-Link NORMAL, and 10 IO-Link FAULT. Both IO-Link runs retain
OPERATE through completion. The fault run includes the master-observed healthy,
cooling-failure, and overtemperature verdicts plus the pending diagnostic event.
The final UART1 reply is drained before `TFS DONE` so a master diagnostic cannot
split its console completion record.

## Files

- `firmware/` — startup (`startup.S`), linker (`c3.ld`), UART (`c3_uart.*`),
  SYSTIMER timestamps and waits (`c3_time.*`),
  the on-target I²C shim driving the C3 I²C0 engine (`mlx90640_i2c_c3.c`), the
  fast soft-float libm shim (`fast_math.c`), the fingerprint algorithm
  (`fingerprint.*`), and `main.c`. The Melexis driver is compiled from
  `third_party/mlx90640-library` directly by the Makefile.
- `system.yaml` / `system-fault.yaml` — the two thermal scenes.
- `test.yaml` / `test-fault.yaml` — headless runs + assertions.
