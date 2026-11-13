# S32K388EVB-Q289 (S32K388) — LPUART+LED L1 example

Boots a bare-register Cortex-M7 smoke on CM7_0 of the NXP S32K388EVB-Q289 map
and proves the LPUART2 console, the SIUL2 output path on the six RGB LED pads
and the PTA1 input pulls.

## What the firmware does

1. Carries the S32K3 boot header (IVT) at `0x0040_0000`: marker `0x5AA55AA5`,
   `BOOT_CONFIG` = CM7_0 enabled, CM7_0 start address `0x0040_1000`, where
   `cortex-m-rt` puts the vector table.
2. Unlocks `SWT_0` (`SR` = `0xC520`, `0xD928`) and clears `CR.WEN`.
3. Gates SIUL2 (`PRTN1_COFB1` REQ42) and LPUART2 (`PRTN1_COFB2` REQ76) in
   MC_ME partition 1, then `PCONF.PCE`, `PUPD.PCUD` and the `CTL_KEY`
   `0x5AF0` / `0xA50F` sequence.
4. Sets LPUART2 to 8N1 at 115384 baud (`OSR` 16, `SBR` 13 on the 24 MHz
   reset `AIPS_SLOW_CLK`).
5. Makes PTF17–19 and PTF22–24 GPIO outputs with the input buffer on (`MSCR`
   `SSS` = 0, `OBE`, `IBE`), then turns them on one at a time through the
   `GPDO` bytes, reading each pad back through `GPDI` and the whole port
   through `PGPDO11`/`PGPDI11`.
6. Clears PTF17–19 with one masked `MPGPDO11` write, then the rest with a
   `PGPDO11` write.
7. Reads PTA1 (SW10) with the internal pull-up, then the pull-down.
8. Prints a heartbeat that toggles the green LED (PTF23) through `PGPDO11`.

Expected console (asserted by `io-smoke.yaml`):

```text
S32K388 EVB SMOKE
cpu=cortex-m7 core=cm7_0 uart=lpuart2
ivt marker=5aa55aa5 cm7_0=00401000 vtor=00401000
clk aips_slow=24000000 osr=16 sbr=13 baud=115384
leds=off pgpdo11=0000 pgpdi11=0000
on ptf17 gpdi=1 pgpdo11=4000 pgpdi11=4000
on ptf18 gpdi=1 pgpdo11=6000 pgpdi11=6000
on ptf19 gpdi=1 pgpdo11=7000 pgpdi11=7000
on ptf22 gpdi=1 pgpdo11=7200 pgpdi11=7200
on ptf23 gpdi=1 pgpdo11=7300 pgpdi11=7300
on ptf24 gpdi=1 pgpdo11=7380 pgpdi11=7380
mpgpdo pgpdo11=0380 pgpdi11=0380
pgpdo pgpdo11=0000 pgpdi11=0000
pta1 pu=1 pd=0
hb=0 ptf23=1
hb=1 ptf23=0
```

## Run it

```bash
# from the repository root
cargo build -p firmware-s32k388-evb-smoke --release --target thumbv7em-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/s32k388-evb/io-smoke.yaml --no-uart-stdout
```

`ci/test.sh` runs the same io-smoke and then the unsupported-instruction audit
with `--fail-on-unsupported`; `configs/ci/boards.yml` runs both on every PR
that touches configs/ or examples/.

## Files

| File | Purpose |
| :--- | :--- |
| `system.yaml` | Board manifest (same as `configs/systems/s32k388-evb.yaml`) |
| `io-smoke.yaml` | Ordered UART assertions over 4,000,000 steps |
| `ci/build.sh`, `ci/test.sh` | Board gate build and proof |
| `VALIDATION.md` | Claims, evidence and non-claims |
| `REQUIRED_DOCS.md` | Sources |
| `EXTERNAL_COMPONENTS.md` | Board parts not modelled |

The committed fixture `tests/fixtures/s32k388-evb-smoke.elf` is the same
build; `firmware_survival::test_s32k388_evb_smoke_survival` boots it.
SIM-DERIVED: there is no S32K388 bench part.
