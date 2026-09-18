# MemBrowse + LabWired

Two tools, one ELF, one verdict.

[MemBrowse](https://membrowse.com) reads the ELF and the linker script. It knows every
byte the linker placed, which symbol owns it, which source file it came from, and how
that has moved commit over commit.

[LabWired](https://labwired.com) runs that same ELF on the modeled chip. It knows what
the firmware *did*: UART output, bus traffic, register state, and — the part no static
analysis can reach — how much stack the run actually burned.

Neither tool alone answers the question a firmware team is really asking.

> Static analysis says the firmware fits. It says nothing about the stack, so it is a
> lower bound. A simulator measures the stack but has no idea which symbol to blame when
> the number moves. Put them together and you get a worst case you can gate on, plus the
> symbol to go fix when the gate trips.

## Run it

Both halves run locally with no account. MemBrowse's local mode writes JSON instead of
uploading; `labwired test` runs free-tier.

```sh
pip install membrowse pyyaml
curl -fsSL https://labwired.com/install.sh | sh

./examples/membrowse/run-demo.sh
```

That runs two committed examples, one per architecture, and merges each pair:

| Target | Core | Firmware | LabWired script |
| --- | --- | --- | --- |
| `nrf54l15-dk` | Cortex-M33 | [`examples/nrf54l15-dk/`](../nrf54l15-dk/) | `io-smoke.yaml` |
| `esp32c3-blinky` | RISC-V | [`examples/esp32c3-blinky/`](../esp32c3-blinky/) | `test-blink.yaml` |

## What comes out

Real output from `run-demo.sh` on the nRF54L15-DK example:

```markdown
### Memory & behaviour — `nrf54l15-dk` · ✅ pass

arch `ARM` · toolchain `gcc-16.1.0` · ELF `82ee70c659a9`

#### RAM: what the linker placed + what the run actually used

| Contributor | Bytes | Source |
| --- | ---: | --- |
| `.data` | 0 | MemBrowse (static) |
| `.bss` | 128 | MemBrowse (static) |
| peak main stack | 24 | LabWired (measured) |
| peak heap | 0 | LabWired (measured) |
| **worst case** | **152** | **0.06% of 262,144 B** |
```

The last row is the point. `.bss` is 128 bytes and the linker is happy; the run adds 24
bytes of measured stack on top. On a 256 KB part that is noise — on a part where static
RAM is already at 90% it is the difference between shipping and a field stack overflow.

The report also carries the largest RAM symbols with source attribution (so a failed gate
names the thing to shrink), the LabWired assertion results, and a memory-map cross-check.

### The cross-check

MemBrowse gets the memory map from the linker script. LabWired gets it from its chip
catalog — the modeled silicon. Those are independent sources for the same facts, so
comparing them is free:

| Quantity | Linker script | Silicon model | |
| --- | ---: | ---: | --- |
| RAM size | 262,144 | 262,144 | ✅ |
| code region size | 1,560,576 | 1,560,576 | ✅ |
| static RAM | 128 | 128 | ✅ |
| code bytes | 428 | 428 | ✅ |

A mismatch on the first two rows means the linker script and the part disagree about the
device — a linker script copied from a sibling SKU, a wrong memory origin, a bootloader
offset nobody subtracted. That is normally found by flashing a board and watching it not
boot. Here it is a warning line in a PR comment.

## Gating

[`budgets.yaml`](budgets.yaml) holds per-target budgets. `combined-report.py` exits
non-zero when one is exceeded or when the LabWired run did not pass:

```yaml
targets:
  nrf54l15-dk:
    flash_used_bytes: 4096
    ram_static_bytes: 2048
    main_stack_high_water_bytes: 1024
    ram_combined_bytes: 3072       # static + measured runtime
```

`ram_combined_bytes` is the gate that needs both tools. Tighten it to 140 against this
example and:

```
| static RAM                 | 128 | 2,048 | ✅ |
| combined RAM (static+peak) | 152 |   140 | ❌ |

> ❌ combined RAM (static+peak) 152 B exceeds budget 140 B (over by 12 B)
```

Static RAM passes its own budget comfortably. The combined figure is what fails — which
is exactly the regression neither tool catches on its own.

## Honest degradation

LabWired paints the stack on ARM. On the RISC-V example it reports
`main_stack_method: unsupported`, and the report says so rather than quietly treating
static RAM as the answer:

```
| peak main stack | not measured | LabWired (arch_not_implemented) |

> Combined worst case is unavailable: the runtime half was not measured on this
> target, so static RAM is a floor, not the answer.
```

A budget set for an unmeasured dimension raises a warning instead of silently passing.
Where the model is thin, it is written down — see [FIDELITY.md](../../FIDELITY.md).

## In CI

[`github-actions.yml`](github-actions.yml) and [`gitlab-ci.yml`](gitlab-ci.yml) are
copy-paste templates. One build, then:

1. **MemBrowse** analyses the ELF. With `MEMBROWSE_API_KEY` set it also uploads, so the
   MemBrowse dashboard keeps the per-commit history and diffs.
2. **LabWired** runs the ELF on the modeled chip and writes `result.json`.
3. **`combined-report.py`** merges the two, gates the sum, and posts one PR comment.

The two tools stay independently useful — MemBrowse's own PR comment and trend history,
LabWired's own JUnit and HTML report — and the combined comment is the single line a
reviewer reads: *does it fit, and does it work.*

## Files

| File | What it is |
| --- | --- |
| [`run-demo.sh`](run-demo.sh) | Runs both tools over both targets locally |
| [`combined-report.py`](combined-report.py) | Merges the two JSON reports, renders markdown, applies the gate |
| [`budgets.yaml`](budgets.yaml) | Per-target static + runtime budgets |
| [`github-actions.yml`](github-actions.yml) | GitHub Actions template |
| [`gitlab-ci.yml`](gitlab-ci.yml) | GitLab CI template |

`combined-report.py` reads only documented fields — MemBrowse's `--json` report and
LabWired's `result.json` ([schema](../../docs/resource_metrics.md)) — so neither tool
needs to know about the other.
