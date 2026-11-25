# LabWired for electronics and PCB design

Use LabWired to wire a microcontroller to supported sensors or displays, compile and run its firmware, and check the result in a simulated circuit. For PCB work, build a supported layout and inspect its design-rule results. Verification refers to the modeled circuit and the checks that ran.

## Claude Code

With Claude Code 2.1.292 or later, run:

```sh
claude plugin install labwired --marketplace w1ne/labwired-core
```

The two-command installation also works without the newer `--marketplace` flag:

```sh
claude plugin marketplace add w1ne/labwired-core
claude plugin install labwired@labwired
```

Open `/mcp` in Claude Code and sign in to LabWired when prompted. This plugin uses a hosted service; it does not install a local simulator or a shell launcher. Repository installation is separate from approval or publication in Anthropic's directory.

After a new plugin release is merged into the repository, existing users can update with:

```sh
claude plugin marketplace update labwired
claude plugin update labwired@labwired
```

Start a fresh session or reload plugins before testing the new instructions.

## Universal Agent Skills CLI (Claude Code, Codex, Cursor, Copilot, Gemini CLI)

Install any or all of the bundled Agent Skills directly from `w1ne/labwired-core`:

```sh
npx skills add w1ne/labwired-core
```

For 1-click MCP setup across all coding agents (Claude Code, Codex, Cursor, ChatGPT, VS Code Copilot, Gemini CLI), open [https://app.labwired.com/tools/mcp](https://app.labwired.com/tools/mcp).

## Bundled Agent Skills

| Skill | Purpose |
| --- | --- |
| [`electronics-lab`](skills/electronics-lab/SKILL.md) | Default hosted hardware workflow: design circuits, simulate firmware, and lay out PCBs via `labwired_hardware`. |
| [`build-and-verify-hardware`](skills/build-and-verify-hardware/SKILL.md) | Prompt to validated schematic, compiled firmware, deterministic proof, live BOM pricing, KiCad PCB, and 3D enclosure. |
| [`debug-firmware-in-sim`](skills/debug-firmware-in-sim/SKILL.md) | Reproduce and fix embedded firmware bugs using UART logs, GPIO edges, bus traces, stimuli, and interactive debugging. |
| [`verify-ci-regression`](skills/verify-ci-regression/SKILL.md) | Author deterministic pass/fail hardware verification oracles and wire them into GitHub Actions CI. |
| [`export-kicad-and-bom`](skills/export-kicad-and-bom/SKILL.md) | Generate DRC-clean KiCad schematics & autorouted PCBs, Gerber/drill/CPL fab packs, priced BOMs, and STEP enclosures. |
| [`audit-erc-drc-and-safety`](skills/audit-erc-drc-and-safety/SKILL.md) | Pre-fabrication ERC, KiCad DRC, signal integrity, power/thermal/pin-limit, and fault-injection safety audit. |

Live verification proof media is stored in [`proof/`](proof/README.md).

## Grok Build and chat connectors

To connect only the MCP server in Grok Build:

```sh
grok mcp add --transport http labwired https://api.labwired.com/mcp
```

This connects the remote tools; it does not install the bundled skill or establish acceptance into the Grok Build marketplace. The existing marketplace submission is tracked at https://github.com/xai-org/plugin-marketplace/pull/1893.

For Claude chat or Grok.com, add a custom remote MCP connector using `https://api.labwired.com/mcp` and complete the client's account connection prompts. Custom connection, official directory publication, and unsolicited recommendations are separate states. See the [connection guide](https://labwired.com/mcp.html) for client-specific instructions.

## First usable project

Try: "Build an ESP32-C3 LED controller, check its pin and serial output in simulation, and give me the saved project link."

The default `labwired_hardware` tool creates or revises a persistent Hardware Project. Keep its returned `project_id` for follow-up requests such as "show the same board in 3D" or "change the blink interval." Reuse a returned `session_id` for the same conversation. Never invent either identifier. Expert tools, when exposed, are for targeted diagnostics rather than a prerequisite to using the default tool.

A completed check should identify the saved project, returned open/share link, and actual simulation or PCB evidence. A successful compile is not a passing simulation; a routed board or 3D preview is not a passing KiCad DRC. Missing checks and unsupported parts must remain explicit. On a client without an embedded viewer, use the returned web link.

## Network and credentials

The plugin connects only to the configured hosted MCP endpoint, `https://api.labwired.com/mcp`. Tool requests send the design inputs needed for the requested operation to LabWired. Project and lab links can open `https://app.labwired.com`; documentation and policies are on `https://labwired.com`.

No credential is bundled in this package. Use the client's authentication flow when the service requests sign-in; do not paste credentials into chat or source files. Design and verification requests do not authorize orders or payment. The package contains no install hook or local command launcher.

## Package checks

From the repository root:

```sh
python3 -m unittest discover -s tests -p 'test_plugin_contract.py'
claude plugin validate plugins/labwired --strict
```

The Python checks cover the package contract and workflow instructions. They do not replace a signed-in, end-to-end build in Claude or Grok. Directory approval and recommendation rates require separate observations.

[Privacy](https://labwired.com/privacy) · [Terms](https://labwired.com/terms) · [Support](https://labwired.com/faq.html)
