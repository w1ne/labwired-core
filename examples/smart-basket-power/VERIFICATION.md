# Local verification receipt

Performed in the isolated `labwired-core-qemu-arm` checkout, initially at
`7db68aea7`, on 2026-10-12. The stored results use documented default
assumptions, not an authoritative application-generated circuit fixture.

Tests were written before the power implementation. The first observed red
check was the subprocess JSONL contract test: the model entrypoint was missing
and returned exit 2. The next observed red checks were the native runner and
scenario driver tests: the example manifest/driver did not yet exist. After
implementation, the commands below returned exit 0:

```bash
python3 -m pytest tools/cosim/test_smart_basket_power.py tools/cosim/test_labwired_ngspice.py -q
# 48 passed
python3 examples/smart-basket-power/run_scenarios.py
# passed: true, assertions: 31, waveform_rows: 420
```

The 44 power tests cover JSONL success/error handling, finite/bounded input
validation, immutable routed parameters, duplicate/backward/gapped time,
USB load-first charging budget, battery supplementation, radio load effect,
signed coulomb balance, independent power ledger, empty-cell cutoff,
software request/acknowledgment, timeout, short, persistent latch/recovery,
full-cell termination, depleted-cell USB blocking, DL33/DL30 input thresholds, native CLI routing, and scenario failure exit/reporting.
Four existing ngspice wrapper tests also ran.

`artifacts/report.json` contains all 31 scenario assertions and actual divider
samples; `artifacts/waveforms.csv` contains 420 model rows. Inputs and outputs
are saved as JSONL for replay. Seven fresh-process deterministic replays agree
byte-for-byte. `artifacts/native_usb.json` and `native_battery.json` retain
10 routed outputs each from two independent native CLI invocations. The report
records exact native commands, model SHA-256 and binary SHA-256.

The driver also exercises real libngspice for the divider/RC closed-form
comparison. Assertion failure behavior was verified by supplying zero USB
budget: the driver returned exit 1 and recorded a false charging assertion.

These receipts establish local behavioral power-model and native adapter
behavior only. They contain no firmware execution, QEMU/Linux boot, physical
carrier, RF link, switching-converter stability, or thermal acceptance evidence.
No push, merge or deployment was performed.

Follow-up checks at `b856cf278` added regressions for source exhaustion below
the OCV floor, stale-success invalidation, and carrier buck loss for radio loads.
The current test total above includes these changes; the stored scenario outputs
were regenerated after them.

The timeout is assumed software policy, not an implemented circuit timer. Exact
supervisor recovery timing is not modeled. Depleted cell protection blocks USB output rails;
USB may charge the cell until explicit reset has a recovered battery source.
Supervisor tolerance and startup KILL-ignore timing remain outside this model.
