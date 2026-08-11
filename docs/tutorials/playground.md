# Playground first run

Open a lab in the browser. Run firmware on a virtual board. Share a link. No local install.

---

## 1. Open Studio

Go to **[app.labwired.com](https://app.labwired.com)** and sign in (or use guest access if offered).

**Try a ready lab:** [SSD1306 hello](https://app.labwired.com/?lab=ssd1306-hello-lab)

---

## 2. Pick a board or starter

- Open a **starter lab** (LED, OLED, sensor), or  
- Create a lab and add an MCU from the catalog  

Popular chips: [ESP32-C3](../boards/esp32c3.md) · [nRF52840](../boards/nrf52840.md) · [RP2040](../boards/rp2040.md)

Check the board page for **artifact type** (ELF vs merged flash image) and the ✅ / ⚠️ / ❌ support matrix.

---

## 3. Build or upload firmware

- Use **in-lab build** when the board has a compile profile, or  
- **Upload** the same binary you would flash to hardware  

If checks fail, confirm you are not asserting on a feature marked ❌ or ⚠️ stub.

---

## 4. Run and watch

Click **Run**. Watch serial, pins, and display widgets.

The twin is deterministic: same inputs → same result on re-run.

---

## 5. Share

Copy the lab share URL for a person, or hand the board / diagram to an agent ([MCP](../agent/mcp.md)).

---

## Next

| Path | Doc |
|------|-----|
| Agent does the loop | [First agent run](../agent/first-run.md) |
| CLI / CI | [Run firmware](../getting_started_firmware.md) · [CI](../ci_integration.md) |
| Add a sensor or actuator | [Onboard a part](../howto/onboard-part.md) |
| What “pass” means | [Fidelity](../fidelity.md) |
