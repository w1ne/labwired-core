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
| MCU | LabWired’s H735 model (first M7) — swap when customer names exact H7 |
| Modem | **BG770A AT stand-in**, not production telematics module |
| GPS | Simulator default coordinates from `+QGPSLOC` |
| MQTT | Happy-path Quectel AT model, not a real broker |
| Radio quality | Same **RfMedium** path-loss as VirtualAirBus / lab AirBus. Drag **Range (m)** on the modem (UE ↔ cell). Optional **RSSI override** (CSQ) for scripts. |
| Messages | **SimMqttFabric** on AirBus: **send** (`QMTPUB`) + **collect** (log / playground strip). Not a full network or real broker. RF path-loss can gate open/pub when range is bad. |

Drag **Range** if you care about CSQ. After Run, the fabric strip shows published `topic` + payload (collect). Optional two-UE: `env-two-ue.yaml`.

## Build

```bash
cd core/examples/h735-telematics-lab
cargo build --release --target thumbv7em-none-eabi
```

Copy ELF into playground assets as `demo-h735-telematics-lab.elf` when packaging.

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

## Playground

Board id: `h735-telematics-lab`  
Deep link: `https://app.labwired.com/?board=h735-telematics-lab&run=1`
