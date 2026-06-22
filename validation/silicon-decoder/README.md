# Silicon-vs-emulator validation: wide-instruction decoder fixes

Proves the labwired Cortex-M4/M33 decoder fixes match **real silicon** byte-for-byte, so
they restore fidelity rather than break it. Covers three fixes uncovered by the udslib
dual-H5 UDS gate:

| Fix | Bug |
|-----|-----|
| `LDRB.W`/`LDRH.W` zero-extend | the wide loads were decoded as **signed** |
| `{S,U}XT{B,H}.W` | the wide register-extends were **not decoded** (skipped) |
| `{S,U}XTA{B,H}.W` | the extend-**and-add** variants were **not decoded** (skipped) |

`main.c` runs the exact instructions — with the same operands as the
`cortex_m::tests::test_exec_wide_*` unit tests — and writes each result to
`g_results[]` at `0x20000000`. The Cortex-M4 (L476) shares these Thumb-2 encodings with
the Cortex-M33 (H563), so it validates the same decoder paths.

## Silicon-validated results (a real NUCLEO-L476RG, over SWD)

| `g_results[i]` | instruction | value |
|---:|---|---|
| 0 | `LDRB.W` of `0x85` | `0x00000085` |
| 1 | `LDRSB.W` of `0x85` | `0xFFFFFF85` |
| 2 | `LDRH.W` of `0x8042` | `0x00008042` |
| 3 | `LDRSH.W` of `0x8042` | `0xFFFF8042` |
| 4 | `UXTH.W` of `0x1234FF00` | `0x0000FF00` |
| 5 | `UXTB.W` of `0x85` | `0x00000085` |
| 6 | `SXTB.W` of `0x85` | `0xFFFFFF85` |
| 7 | `UXTH.W … ROR #8` of `0x00850000` | `0x00008500` |
| 8 | `UXTAH 4, 0x12340002` | `0x00000006` |
| 15 | done sentinel | `0xDEADBEEF` |

The labwired `stm32l476` model produces this **same** table (see `lockstep.yaml`), and the
`cortex_m` unit tests assert the same values at the instruction level. All three agree.

## Build

```sh
make           # clang (cortex-m4) + rust-lld -> build/silicon_decoder.elf
```

## Re-validate on hardware (NUCLEO-L476RG via ST-LINK)

```sh
CHIP=STM32L476RGTx
probe-rs download --chip $CHIP --binary-format elf build/silicon_decoder.elf
probe-rs reset    --chip $CHIP
probe-rs read     --chip $CHIP b32 0x20000000 16   # compare to the table above
```

## CI regression (no hardware)

`lockstep.yaml` runs the same ELF on the labwired `stm32l476` model and asserts every
result equals the silicon-captured value — so any future decoder change that diverges
from silicon fails:

```sh
labwired test --script lockstep.yaml   # exit 0 == emulator matches silicon
```
