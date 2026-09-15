# First agent run

In one session: **pick a board → run firmware → verify**.  
If the model says “it works” without **`labwired_verify`** (or a green `labwired test`), treat it as **unproven**.

---

## 0. Connect

Follow [Connect MCP](mcp.md) (hosted or stdio).  
Smoke: `labwired_list` returns boards.

---

## 1. Discover hardware

Prompt:

```text
Using LabWired MCP:
1) labwired_list kind=board
2) labwired_describe id=esp32-c3-supermini
Summarize pins, flash/artifact needs, and limits from the description.
Do not invent pins that describe did not return.
```

Prefer ids from `labwired_list` / `labwired_describe`. Board docs: [ESP32-C3](../boards/esp32c3.md).

---

## 2. Compile or attach firmware

**Hosted (easiest):**

```text
Compile a minimal blink (or my project) for board esp32-c3-supermini
with labwired_compile. Keep the firmware_ref.
```

**Stdio:** build with your normal toolchain (ESP-IDF, PlatformIO, …). Pass the ELF, flash image, or a `firmware_ref` the local cache accepts. There is **no** `labwired_compile` on stdio.

ESP32-C3 note: many IDF apps need a **merged flash `.bin`**, not only an app ELF — see the board page.

---

## 3. Run on the twin

```text
labwired_run with the firmware_ref (and board/diagram if required).
Return serial output, faults, and snapshot_id.
```

Optional: `labwired_inspect` with that `snapshot_id` for GPIO or registers.

---

## 4. Verify (required for claims)

```text
labwired_verify with the same board/system and firmware_ref.
Use assertions that match the demo (UART text, GPIO level, …).
Report only pass or fail with the reason from the tool.
```

| Result | Meaning |
|--------|---------|
| **Green** | Oracle accepted the run |
| **Red** | Fix firmware or expectations — do not rebrand as success |

---

## 5. Optional: share with a human

On hosted:

```text
labwired_lab — open a Studio lab and give me the share URL.
```

Or open [app.labwired.com](https://app.labwired.com).

---

## Good habits

| Do | Don’t |
|----|--------|
| Call `labwired_describe` before wiring | Guess pin maps from training data |
| Use `labwired_verify` for claims | “Looks correct from the log” alone |
| Respect ✅ / ⚠️ / ❌ on [board pages](../boards/esp32c3.md) | Assert BLE/analog if marked ❌ |
| Keep firmware as refs / paths | Paste multi‑MB binaries into chat |

---

## Next

- [Tool reference](tools.md)
- [CLI path](../getting_started_firmware.md)
- [CI](../ci_integration.md)
- [Onboard a part](../howto/onboard-part.md)
- [Fidelity](../fidelity.md)
