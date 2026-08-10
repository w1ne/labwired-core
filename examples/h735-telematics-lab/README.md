# H735 Telematics Lab

**STM32H735 + Quectel BG770A (virtual) + ILI9341 TFT**

CLM-style telematics story for demos (e.g. Proemion):

1. MCU drives modem AT on **USART1**
2. `AT+QGPSLOC` → parse lat/lon
3. Paint fix on **ILI9341** (SPI1, CS=PA4)
4. Publish JSON over modem **MQTT AT** (`QMTOPEN` / `QMTCONN` / `QMTPUB`)
5. Full AT transcript on **USART3** (Serial panel)

## Honesty

| Piece | Reality |
|-------|---------|
| MCU | LabWired’s H735 model (first M7) — swap when customer names exact H7. **Sim-derived**: every reset value comes from RM0468/DS13312, there is no H735 bench part and no silicon diff (`🔵 sim-validated` in VALIDATION_STATUS.md) |
| **CAN** | **NOT MODELLED.** The chip yaml wires no FDCAN — see its own note, “RTC / FDCAN / OCTOSPI / Ethernet: not yet wired”. This demo carries telematics data over the **modem**, not the vehicle bus. For a CAN story use `examples/canmod-gps-sim` or `examples/f103-j1939-monitor`, which decode real frames against a real DBC |
| Other MCU gaps | No ADC, DMA, RTC, OCTOSPI or Ethernet model. The H735 pin table advertises `adc1`, `fdcan1/2`, `i2c3`, `spi3`, `tim4`, `tim8`, `uart2/4/6` as pin functions, but the chip models **none** of them — wiring a part to those pins gets you a labelled pin attached to nothing. Modelled and usable: `usart1`, `usart3`, `lpuart1`, `i2c1`, `i2c2`, `spi1`, `spi2`, `tim1_pwm`, `tim2`, `tim3` |
| Modem | **BG770A AT stand-in**, not production telematics module |
| GPS | Simulator default coordinates from `+QGPSLOC` |
| MQTT | Happy-path Quectel AT model, not a real broker |
| Radio quality | Same **RfMedium** path-loss as VirtualAirBus / lab AirBus. Drag **Range (m)** on the modem (UE ↔ cell). YAML `config.rssi` is a seed only (not a UI SimInput). |
| Messages | **SimMqttFabric** on AirBus: **send** (`QMTPUB`) + **collect** (log / fabric strip / `mqtt_fabric` smoke). Not a full network or real broker. RF path-loss can gate open/pub when range is bad. |

Drag **Range** if you care about CSQ. After Run, the fabric strip shows published `topic` + payload (collect). Two-UE pub/sub: `env-two-ue.yaml` (publisher + subscriber ELFs).

## Build

```bash
# from core/
cargo build -p h735-telematics-lab --release --target thumbv7em-none-eabi \
  --bin h735-telematics-lab --bin h735-telematics-subscriber
```

Copy publisher ELF into playground assets as `demo-h735-telematics-lab.elf` when packaging.

## Run in sim (CLI)

```bash
# from labwired-core / monorepo root — adjust paths to your layout
cargo run -q -p labwired-cli -- test \
  --script examples/h735-telematics-lab/io-smoke.yaml \
  --output-dir /tmp/h735-telematics-out --no-uart-stdout
```

Expect UART log lines containing:

- `LabWired telematics`
- `> AT`
- `QGPSLOC` / `GPS fix`
- `location published`

Plus a **`mqtt_fabric`** assertion that topic `telematics/location` collected payload with `"src":"qgpsloc"`.

### Dual-UE (publisher + subscriber)

```bash
cargo run -q -p labwired-cli -- test \
  --script examples/h735-telematics-lab/env-two-ue-smoke.yaml \
  --output-dir /tmp/h735-two-ue-out --no-uart-stdout
```

`ue_a` runs the publisher; `ue_b` runs `h735-telematics-subscriber` (`QMTSUB` → `+QMTRECV` / `location received` in `uart.log`).

⚠ That env script **asserts nothing** — the environment runner only accepts node-qualified `memory_value`, so it exits 0 regardless. The receive path is gated in CI by grepping `uart.log` for `+QMTRECV` and `location received`; do the same locally before believing a green run.

## What CI actually gates

Both scripts run in **`core-coverage-matrix-smoke.yml`** (cell `h735-telematics-lab`) on nightly, main tip, and manual dispatch — *not* on PRs, matching the repo's heavy-gate policy.

| Check | Where | Proves |
|-------|-------|--------|
| `io-smoke.yaml`, 11 assertions | matrix cell | AT transcript, `mqtt_fabric` collect, and **four `display_region` boxes** — the panel really painted |
| Dual-UE marker grep | matrix cell | `ue_b` genuinely received a `+QMTRECV` |
| Unsupported-instruction audit | matrix cell | 100% instruction support over 20k instructions |
| Playground lab-smoke | `pages-deploy.yml` | the lab starts and the **cycle counter advances**. It does *not* check the screen — `lab-smoke.mjs` states "display motion is REPORTED, never asserted" |

The display assertions are non-vacuous: re-point `dc_pin` from `PA3` to an unconnected pin and the run drops from 11/11 to 7/11 with all three ink regions at 0.0%, while every UART assertion still passes.

## Playground

Board id: `h735-telematics-lab`  
Deep link: `https://app.labwired.com/?board=h735-telematics-lab&run=1`
