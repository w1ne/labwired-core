# Read-only flash: naming the region in a BusFault verdict

A tiny firmware stores to `0x0800_1000`, inside flash, on a chip that declares
`flash.cpu_writes: fault`. The store must not commit: the simulator raises a
precise BusFault and the verdict names the read-only region, address and
source line.

## Files

| File | What it does |
| --- | --- |
| `system.yaml` | Points at the STM32L476 fixture chip with `cpu_writes: fault` on flash. |
| `script.yaml` | Runs `fault-readonly-flash.elf` for 5000 steps, expecting `max_steps`. |

```bash
cd <repo root>
cargo run -p labwired-cli -- test \
  --system examples/readonly-flash/system.yaml \
  --script examples/readonly-flash/script.yaml \
  --output-dir /tmp/labwired-readonly-flash
```

## What you should see

```text
PASS  1/1 checks · script · 5000 steps · 0.00s
FAULT  Precise BusFault: data access at 0x0800_1000 (BFAR valid; store to read-only flash), at PC 0x0800_006C in `sensor_read` (fault_fixture.c:61), called from `main`.
       HFSR=0x00000000 CFSR=0x00008200 [PRECISERR] BFAR=0x08001000
```

`/tmp/labwired-readonly-flash/result.json` carries the `memory_policy` block:

```json
  "memory_policy": {
    "flash": {
      "mode": "fault",
      "violations": 1,
      "first": {
        "addr": 134221824,
        "cycle": 7,
        "pc": 134217870
      }
    },
    "ram": {
      "mode": "allow",
      "violations": 0
    }
  }
```

The firmware stores to `0x0800_1000` (inside flash, `cpu_writes: fault`), so the
simulator raises a precise BusFault and names the read-only region; with `allow`
the same store would silently commit.
