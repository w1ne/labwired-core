# Playground first run

No Rust install. Open a lab, run firmware on a virtual board, share a link.

---

## 1. Open Studio

Go to **[app.labwired.com](https://app.labwired.com)** and sign in (or use the guest path if offered).

---

## 2. Pick a board / starter lab

Start from a starter (LED, OLED, …) or an empty lab and add an MCU from the catalog.

Examples:

- **ESP32-C3 Super Mini** — see [ESP32-C3 board docs](../boards/esp32c3.md)  
- **nRF52840** — see [nRF52840](../boards/nrf52840.md)  

Check the board page for **flash artifact** (ELF vs merged `.bin`) and the support matrix before expecting a peripheral to work.

---

## 3. Build or upload firmware

- Use the in-lab build when the board’s compile profile is available, **or**  
- Upload the artifact you would flash to silicon  

If serial/oracle checks fail, confirm you are not asserting on a **❌ / ⚠️ stub** feature.

---

## 4. Run and observe

Hit run. Watch serial, pins, and any display widgets.  
Deterministic twin: re-run with the same inputs should match.

---

## 5. Share

Use the lab share URL for humans or hand the same board id to an agent via [MCP](../agent/mcp.md).

---

## Next

| Path | Doc |
|------|-----|
| Agent does the loop | [First agent run](../agent/first-run.md) |
| CLI / CI | [Running firmware](../getting_started_firmware.md), [CI](../ci_integration.md) |
| What “pass” means | [Fidelity](../fidelity.md) |
