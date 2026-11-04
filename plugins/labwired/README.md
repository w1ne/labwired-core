# LabWired for electronics and PCB design

Use LabWired to wire a microcontroller to supported sensors or displays, compile and run its firmware, and check the result in a simulated circuit. For PCB work, build a supported layout and inspect its design-rule results. Verification refers to the modeled circuit and the checks that ran.

## Install

In Claude Code, run:

```sh
claude plugin install labwired --marketplace w1ne/labwired-core
```

Open `/mcp` in Claude Code and sign in to LabWired when prompted. The plugin connects to the hosted service and supplies an electronics workflow skill. It does not need a local simulator or a shell launcher.

To connect only the MCP server in Grok Build:

```sh
grok mcp add --transport http labwired https://api.labwired.com/mcp
```

For Claude chat, use **Customize → Connectors → Add → Custom → Web**, enter `https://api.labwired.com/mcp`, and follow the account connection prompts. See the [connection guide](https://labwired.com/mcp.html) for other clients.

## Try it

- Build an ESP32-C3 circuit with an OLED and verify what the firmware displays.
- Turn a supported circuit diagram into a two-layer PCB and report the KiCad design-rule results.

[Privacy](https://labwired.com/privacy) · [Terms](https://labwired.com/terms) · [Support](https://labwired.com/faq.html)
