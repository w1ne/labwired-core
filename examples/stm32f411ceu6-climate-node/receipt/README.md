# Climate node — run and verify receipts

Stage 3 Task 4. Two receipts, each stating the path that produced it:

| Receipt | Path | Evidence |
|---|---|---|
| `run.json` | **local engine** (`labwired-cli 0.25.0`, the same binary the MCP stdio tool shells out to via `LABWIRED_CLI`) | `run/uart.log`, `run/run-manifest.json` |
| `verify.json` | **local engine run + the shared evaluator** (`@labwired/board-config` `evaluateOracle`/`classifyModelVerification` — the same code the hosted/stdio `labwired_verify` calls) | `oracle.json`, `run/result.json`, negative control |

The MCP stdio attempts are archived in `mcp-stdio-attempt.json`; neither stdio
tool can serve this system, for the reasons below. The hosted tools were not
attempted (no token in this environment); the Stage 5 demo exercises that path.

## Reproduce

```bash
# local receipts (needs the built engine at core/target/release/labwired):
./receipt.sh
# -> receipt/run.json, receipt/verify.json, receipt/run/{uart.log,run-manifest.json,...}

# the MCP stdio attempt (needs the monorepo toolchain: tsx, built
# packages/board-config/dist and packages/kicad-monkey/dist):
node receipt/mcp-attempt.mjs
```

`receipt.sh` runs the smoke twice (byte-identical `uart.log` required, else it
exits 1), runs a 12.5M-cycle negative control (boot state only, before the 15M
stimuli), then composes both receipts with `receipt/local-oracle.mjs`.

## Run receipt (`run.json`)

Command (engine `labwired-cli 0.25.0`):

```bash
labwired test --script io-smoke.yaml \
  --firmware firmware/.pio/build/blackpill_f411ce/firmware.elf \
  --system system.yaml --output-dir receipt/run --run-manifest --watch-gpio gpioa:5
```

- `status: pass`, `stop_reason: max_steps`, 24,000,000 steps; 11/11 assertions.
- 5/5 stimuli `applied` at cycle 15,000,000.
- Inputs: firmware sha256 `8210df4c…` (rebuilt byte-identical by `receipt.sh`),
  system.yaml `274f8628…`, io-smoke.yaml `8f6cc60b…`.
- Transcript sha256 `8cc750cdb9f8239246d6396ab05706ff8ec7db37284dc7a9665eec89929ed02f`
  — byte-identical across this receipt's 2 runs and the 3 runs recorded when
  the example landed (`receipt/run/uart.log`). The `run-manifest.json` digest
  (`851ab720…`, wall-clock-free) is also identical across the two runs.
- PA5 logic capture (canonical `parseLogicEdges` seam,
  `services/labwired-builder/src/run.ts:563-584`): 1 transition, `z→1` at
  cycle 965,643, `dropped: 0`; end-of-run `gpio_pins`: `mode=af func=TIM2_CH1
  pad=true`. See the duty gap below.

## Verify receipt (`verify.json`)

Oracle (`oracle.json`): the smoke's serial contract (9 clauses), the OLED
display clause, and `{pin: PA5, state: toggled}` — evaluated with
`@labwired/board-config` against the local run, fed through the same seams the
hosted builder uses (uart → `serial`; `inspect.devices` → `peripherals`, with
`device_type` folded per the `foldDeviceArtifacts` convention the oracle
documentation states; `logic_edges` → `gpio`).

- **Verdict: `model_verified`, `proven: true`** — 11/11 clauses pass
  (9 serial, 1 display `ink=170`, 1 gpio). The two engine-stated
  `approximation` notes (`derived_device_time`, `unpowered_rail_assumed`)
  are carried as gaps; approximations do not flip the status
  (`packages/board-config/src/model-verification.ts`).
- **Negative control: `failed`** — the same oracle on a 12.5M-cycle run
  (boot only, before the 15M-cycle stimuli) fails exactly the four
  post-stimulus clauses (NTC 79.97 / LDR 794.2 / BME 31.00 / ALARM 1) while
  every boot clause and the display/gpio clauses still pass. The oracle
  discriminates; it is not vacuously true.

## Why the MCP stdio tools cannot serve this system (archived raw output)

- `labwired_run` → `{"error":"INVALID_BOARD","message":"Unknown target ..."}`.
  The stdio board catalog is derived from `@labwired/board-config`'s
  `LABS` registry (`packages/mcp/src/boards.ts:124-128`), and the lookup that
  refuses the target is `getBoard` (`packages/mcp/src/handlers/run.ts:219-223`).
  There is **no F411 lab at all**: `packages/board-config/src/labs.ts` has 72
  labs, none with chip `stm32f411ceu6`, and `labwired_list` returned 67
  runnable boards with zero F411/climate ids. The run receipt therefore comes
  from the local engine — the same code path the tool invokes with
  `LABWIRED_CLI` — as `run.json` states.
- `labwired_verify` → `status: "inconclusive"`, `error: "No builder URL
  configured…"`. The serial/register/gpio clauses route through the hosted
  builder (`packages/mcp/src/handlers/verify.ts:113-117` `needBuilder`; the
  refusal is `runBuildViaBuilder`, `packages/mcp/src/cli.ts:245-248`). The
  display half ran locally (`runDeviceCapture` → `labwired snapshot capture
  --profile arduino-esp32`) and decoded no panel: the stdio readout only
  understands `ssd1680`/`uc8151d` e-paper panels
  (`packages/mcp/src/cli.ts:110-114`), not the SSD1306 OLED. (The attempt
  seeds the documented local stdio artifact cache
  `~/.labwired/artifacts/sha256/<hex>` — `firmware-ref.ts` resolves that
  first — so the local halves really executed.)
- **Hosted not attempted: no token in this environment** (`~/.labwired` holds
  only the evidence store; `~/.config/labwired` only ci-diagnostics
  credentials; no `LABWIRED_MCP`/`LABWIRED_TOKEN`/`access_token` in
  `packages/mcp/src`). No run id is invented.

## Unclaimed clauses (do not read these as verified)

1. **PWM duty on PA5** (`gpio_duty`): the capture shows one drive-enable edge
   and a flat high tail, not a PWM square wave — TIM2 compare toggles of the
   AF pad do not reach the logic tap. A "duty" computed from one edge + tail
   would be an artifact (1.0), so no duty clause is claimed. `gpio` `toggled`
   passes under the canonical semantics (`transitions.length > 0`) with the
   raw transition list in the receipt; it means "the pad transitioned", not
   "the PWM ran".
2. **Serial/register/gpio via the MCP stdio `labwired_verify`** — needs a
   builder URL (see above).
3. **Display via the MCP stdio `labwired_verify`** — e-paper-only decoder; the
   SSD1306 is not observable there (archived result: `painted=false,
   panel=null`).
4. **Anything hosted** — not attempted (no token).

`receipt/run/result.json` (the full 195 KB engine artifact, incl. the display
framebuffer meta and stimulus log) is generated by `receipt.sh` and not
committed; `run/uart.log` and `run/run-manifest.json` are the committed raw
evidence.
