# EFR32MG26 TIMER compare phase — frozen-counter sweep

BRD2709A on a SEGGER J-Link OB, SWD, VTarget 3.301 V, Cortex-M33 r1p0,
openocd 0.12.0. 2026-08-24.

## The instrument

The question — does `IF.CCn` latch when the counter TAKES `OC`, or one clock
later? — cannot be answered by polling over SWD: a round trip is many counter
ticks. This rig removes the race instead of racing it.

`CFG.DEBUGRUN` is left **clear**, so the counter FREEZES the moment the
debugger halts the core. `halt` therefore becomes an atomic hardware snapshot,
and `CNT` and `IF` can be read afterwards at any speed. `FROZEN_CHECK` in
`sweep.tcl` proves the freeze: `CNT` reads identically three times in a row.

The core is parked in a two-byte spin loop (`0xE7FE`) written to RAM at
`0x20000000`, with `PRIMASK = 1`, so the flash firmware never runs and no
interrupt perturbs the measurement.

Each trial: write `CNT`, clear `IF` through the `+0x2000` alias, `resume`,
`halt` (14–18 counter ticks at `PRESC = 1023`), read `CNT` and `IF`.
Every trial re-reads `IF` after the clear and records it as `ifpre`, so a
failed clear is visible in the data rather than silently contaminating it.

## Result

264 trials in the first configuration, **zero mixed verdicts** — every landed
`CNT` gave a unanimous answer.

| `TOP`    | `OC`     | last `CNT` with CC0 clear | first with CC0 set |
|----------|----------|---------------------------|--------------------|
| `0xFFFF` | `0x8000` | `0x8000` (6/6)            | `0x8001` (8/8)     |
| `0xFFFF` | `0x1234` | `0x1234`                  | `0x1235`           |
| `0x00FF` | `0x0080` | `0x0080` (4/4)            | `0x0081` (4/4)     |
| `0x00FF` | `0x00FF` | `0x00FF` (3/3)            | `0x0000`, wrapped  |

**Every counter clock samples the value `CNT` held BEFORE it.** If that value
equals `OC`, the flag latches — and the same clock moves the counter on. So the
flag is visible with `CNT` reading `OC + 1`, never `OC`.

Two more facts from the same rig:

* `OC` **above** `TOP` never latches. `TOP = 0xFF`, `OC = 0x180`, ~2.5 periods:
  `IF = 0x01`, overflow alone. Control (`OC = 0x80`) in the same run: `0x11`.
* **`IF` is not write-1-to-clear** (`if-clear.tcl`). With the counter frozen and
  `IF` reading `0x10`, a direct `IF = 0xFFFFFFFF` left it at `0x10`; the
  `+0x2000` CLR alias then took it to `0`. Series 2 dropped `IFC` and put the
  clear in the alias window.

⚠️ The FIRST run of `sweep.tcl` used a direct `IF` write to clear between
trials and produced garbage — `IF.CC0` apparently set at `CNT` values below
`OC`. That was not a hardware surprise; it was the clear silently failing, and
`ifpre` is what exposed it. `sweep-top-ffff-oc-8000.txt` is the corrected run,
where `ifpre` is 0 on all 264 trials.

## Reproducing

    openocd -s /opt/homebrew/share/openocd/scripts \
      -f interface/jlink.cfg -c "transport select swd" \
      -f target/efm32.cfg -f sweep.tcl

⚠️ Series-2 register discipline, and it is easy to get wrong: `CFG` **and**
`CC_CFG` are config registers and must be written while `EN = 0`; `TOP`, `CNT`
and `CC_OC` are runtime registers and must be written AFTER `EN = 1`. A
`CC_CFG` written after `EN = 1` reads back 0, leaving the channel switched off
— which is a run that measures nothing while looking fine.
