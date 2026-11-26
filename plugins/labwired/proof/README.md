# LabWired MCP setup + agent skills — live visual proof

Captured 2026-11-25 against production (`app.labwired.com`, `api.labwired.com`).

| File | What it shows |
| --- | --- |
| `01-live-tools-mcp-setup-page.png` | Live `https://app.labwired.com/tools/mcp` — step 1 (MCP server URL + copy), free-tier banner (10 verified builds/day free). |
| `02-live-auth-quota-connected-apps.png` | Step 2 — browser OAuth sign-in, daily quota, Connected AI Apps revoke list. |
| `03-live-agent-snippets.png` | Step 3 — 1-click copy snippets for Claude Code, Codex, Cursor, ChatGPT, VS Code Copilot, Gemini CLI. |
| `04-live-skills-install.png` | `w1ne/labwired-core` plugin & Agent Skills install commands + the bundled skills. |
| `05-live-tools-mcp-walkthrough.gif` | Scroll-through of the whole live setup page. |
| `06-agent-install-live-terminal.png` / `.txt` | Real Claude Code run: marketplace add → plugin install `labwired@labwired` → `claude mcp add` → live endpoint checks (`POST /mcp` 401 OAuth-discovery, `GET /tools/mcp` 200). |

Note: the authenticated screenshots (quota numbers, Connected AI Apps list) require a signed-in
Clerk session, which is credential-gated; the page above is the live anonymous view.
