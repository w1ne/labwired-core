# First agent run

Goal: in one session, **describe a board → run firmware → verify** with the oracle.  
If the model says “it works” without `labwired_verify` (or an explicit oracle green), treat that as **unproven**.

---

## 0. Connect

Follow [Connect an agent (MCP)](mcp.md) — hosted or stdio.  
Smoke: `labwired_list` returns boards.

---

## 1. Discover hardware

Prompt the agent:

```text
Using LabWired MCP:
1) labwired_list kind=board (or list everything)
2) labwired_describe id=esp32-c3-supermini
Summarize pins, flash/artifact expectations, and any limitations from the description.
Do not invent pins not returned by describe.
```

Prefer ids from `labwired_list` / `labwired_describe`. Example C3 page: [ESP32-C3](../boards/esp32c3.md).

---

## 2. Compile or attach an artifact

**Hosted (easiest):**

```text
Compile a minimal blink (or my project) for board esp32-c3-supermini
with labwired_compile. Keep the firmware_ref.
```

**Stdio:** build with your normal toolchain (ESP-IDF, PlatformIO, …), then pass the ELF / flash image path or a `firmware_ref` the local cache understands. There is **no** `labwired_compile` on stdio.

ESP32-C3 note: many IDF apps need a **merged flash `.bin`**, not a lone app ELF — see the board page.

---

## 3. Run on the twin

```text
labwired_run with the firmware_ref (and diagram/board if required).
Return serial output, any faults, and snapshot_id.
```

Optional: `labwired_inspect` with that `snapshot_id` for GPIO / registers.

---

## 4. Verify (the dispose step)

```text
labwired_verify against the same board/system and firmware_ref
with assertions appropriate to the demo (UART token, GPIO level, etc.).
Report only the oracle result — pass or fail with reason.
```

**Green** = LabWired oracle accepted the run.  
**Red** = fix firmware or expectations; do not “reinterpret” as success.

---

## 5. Optional: share with a human

On hosted:

```text
labwired_lab — open a Studio lab for this board/diagram and give me the share URL.
```

Or open [app.labwired.com](https://app.labwired.com) yourself.

---

## Good agent habits

| Do | Don’t |
|----|--------|
| Call `labwired_describe` before wiring | Guess pin maps from training data |
| Use `labwired_verify` for claims | “Looks correct from the log” without oracle |
| Respect ✅/⚠️/❌ on [board pages](../boards/esp32c3.md) | Assert BLE/analog if marked ❌ |
| Keep firmware as refs | Paste multi‑MB binaries into chat |

---

## Next

- [Tool reference](tools.md)  
- [CLI path](../getting_started_firmware.md)  
- [CI / oracle](../ci_integration.md)  
- [Fidelity](../fidelity.md)  
