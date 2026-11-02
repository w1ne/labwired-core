# AMD Kria KV260 Vision AI Starter Kit — RPU (Cortex-R5F)

The KV260 (`SK-KV260-G`) carries the K26 SOM: an XCK26 Zynq UltraScale+ MPSoC.
This example runs its Real-time Processing Unit core 0, an Arm Cortex-R5F.
It is the first Cortex-R (ARMv7-R) part in LabWired.

- Chip: [`configs/chips/xck26-rpu.yaml`](../../configs/chips/xck26-rpu.yaml)
- System: [`system.yaml`](system.yaml) (console `uart1`)
- Firmware: unmodified Zephyr v4.1.0 for board `kv260_r5`, rebuilt by
  [`crates/firmware-kv260-r5-zephyr/build.sh`](../../crates/firmware-kv260-r5-zephyr/build.sh)

## What runs

| Script | Proves |
|--------|--------|
| `zephyr-hello.yaml` | PMSAv7 MPU setup, VFP enable, RPU GIC + TTC0 init, polled PS UART1 console |
| `zephyr-sync.yaml` | TTC0 match IRQs through the GIC, WFI idle, thread switches with VFP state |
| `zephyr-kernel-common.yaml` | `tests/kernel/common`: 12 suites, 61 pass / 2 skip / 0 fail, userspace syscalls through the MPU |

The core is the pinned tlib `cortex-r5f` model (PMSAv7 with 16 MPU regions,
VFPv3-D16) behind `--features qemu-arm-system`. That build is native-only and
GPL, so these scripts need a native CLI:

```sh
examples/kv260-r5/ci/test.sh
```

CI runs the same commands in `.github/workflows/core-qemu-arm.yml`. The core
crate's `crates/core/tests/kv260_r5_zephyr.rs` covers hello and sync too.

## Modelled / not modelled

Installed from the chip's `native_rpu_peripherals`:
- the RPU GIC (PL390, GICv1 subset), served by the GICv2 model;
- PS UART0/UART1 (Cadence);
- TTC0 (three 32-bit counters: match, interval and overflow).

Not modelled:
- the APU (Cortex-A53) cluster;
- the PMU/CSU/FSBL boot flow: the ELF is loaded as the FSBL would place it;
- TCM vs DDR timing, caches and lockstep;
- IPI mailboxes, the other TTCs, GPIO, I2C, SPI/QSPI and the PL.

## Availability

The KV260 is a current AMD Kria product, sold as `SK-KV260-G` at DigiKey
(13985269, factory lead time) and Farnell (4690049, stock to reserve). Checked
2026-11-01.
