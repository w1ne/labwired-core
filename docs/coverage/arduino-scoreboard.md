# Arduino × LabWired board matrix

_Generated 2026-07-23 02:54:36 +0200 by `validation/arduino-matrix/run_matrix.py`._

Legend: ✅ pass · 🔧 compile fail · 📦 toolchain/platform missing · 🔴 boot/sim fail · 🟠 UART ran but marker missing · 🟣 unmodeled/unsupported · ⏱️ timeout

| chip | L2_blink_serial | notes |
|------|------|-------|
| `esp32s3` | 🟣 | L2_blink_serial:unmodeled |

## Summary

- Cells: **1**
- `unmodeled`: 1

## Failures (detail)

### `esp32s3` × `L2_blink_serial` → **unmodeled**
```
seeded factory MMU (12 entries, app0 @ flash 0x10000) for cache2phys
labwired-cli test: seeded S3 factory MMU for cache2phys (app0 @ 0x10000)
labwired-cli test: S3 APP handshake flags @ [
    0x3fc9c733,
    0x3fc9c734,
    0x3fc9c731,
    0x3fc9c732,
]
labwired-cli test: pxCurrentTCBs @0x3fc9cc98 (hybrid preserve key)
labwired-cli test: installed xthal_window_spill_nw CPU spill workaround @0x4038839c
labwired-cli test: ESP32-S3 fast-boot entry=0x40379b90 (dual-core APP_CPU)
2026-07-23T00:54:35.477146Z  INFO esp32s3::rom::ets_printf: 
2026-07-23T00:54:36.387093Z ERROR labwired: Simulation error at step 1085861: Memory access violation at 0x20406a
2026-07-23T00:54:36.387110Z ERROR labwired: Assertion failed: UartContains(UartContainsAssertion { uart_contains: "LW_L2_OK" }) (captured len=8)

```
UART tail:
```
LW_L2_BO
```

