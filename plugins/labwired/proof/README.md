# LabWired MCP setup + agent skills — live visual proof

Captured 2026-11-25 and 2026-11-26 against production (`app.labwired.com`, `api.labwired.com`).

| File | What it shows |
| --- | --- |
| `01-live-tools-mcp-setup-page.png` | Live `https://app.labwired.com/tools/mcp` — step 1 (MCP server URL + copy), free-tier banner (10 verified builds/day free). |
| `02-live-auth-quota-connected-apps.png` | Step 2 (anonymous view) — browser OAuth sign-in, daily quota, Connected AI Apps revoke list. |
| `03-live-agent-snippets.png` | Step 3 — 1-click copy snippets for Claude Code, Codex, Cursor, ChatGPT, VS Code Copilot, Gemini CLI. |
| `04-live-skills-install.png` | `w1ne/labwired-core` plugin & Agent Skills install commands + the bundled skills. |
| `05-live-tools-mcp-walkthrough.gif` | Scroll-through of the whole live setup page. |
| `06-agent-install-live-terminal.png` / `.txt` | Real Claude Code run: marketplace add → plugin install `labwired@labwired` → `claude mcp add` → live endpoint checks (`POST /mcp` 401 OAuth-discovery, `GET /tools/mcp` 200). |
| `07-live-signed-in-tools-mcp.png` | Signed-in `https://app.labwired.com/tools/mcp` (hero + Step 1 + Step 2 with live plan/quota status and populated Connected AI Apps). |
| `07b-live-oauth-consent.png` | Live OAuth 2.1 PKCE browser consent screen (`https://app.labwired.com/oauth/authorize`) during dynamic client authorization. |
| `08-live-signed-in-step2-quota-apps.png` | Close-up of Step 2 signed-in state: live quota banner (`GET /v1/mcp/status`) and Connected AI Apps populated with real OAuth grants + working `Revoke` buttons (`DELETE /v1/mcp/grants/:id`). |
| `09-authenticated-mcp-build-terminal.png` / `09-authenticated-mcp-build-transcript.txt` | Real authenticated OAuth 2.1 PKCE session against `https://api.labwired.com/mcp`: `initialize` + `tools/list` (68 tools) → `labwired_list` (`esp32-c3-supermini`) → `labwired_validate` (0 ERC errors/warnings) → `labwired_verify` (`model_verified`, ESP32-C3 ROM boot + `BOOT:ESP32C3_READY` + `HEARTBEAT tick=1` + 4 `GPIO4` edges in 78.846 device ms) → `labwired_price` (live LCSC BOM) → live UI `Revoke` (`DELETE /v1/mcp/grants/:id` 200) → post-revoke `POST /mcp` rejected with `HTTP 401 Unauthorized`. |
