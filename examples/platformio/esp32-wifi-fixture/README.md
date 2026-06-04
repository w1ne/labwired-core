# esp32-wifi-fixture

A minimal arduino-esp32 WiFi sketch — the **firmware fixture** driving the
LabWired ESP32 WiFi functional model (the "simulated endpoints" track).

It joins an AP, HTTP `GET`s an in-sim server, and prints the result:

```cpp
WiFi.begin("labwired", "hunter2");
while (WiFi.status() != WL_CONNECTED) delay(100);
HTTPClient http;
http.begin("http://192.168.4.1/status");
int code = http.GET();
```

## Build

```sh
cd examples/platformio/esp32-wifi-fixture
pio run                       # → .pio/build/esp32dev/firmware.elf (classic ESP32)
```

## Why it exists

The WiFi model is **simulated endpoints**: cycle-accurate radio is
infeasible (closed blob on an RF coprocessor), so instead the simulator
hosts the servers (`crates/core/src/network/{sim,mqtt}.rs` — `VirtualAp`,
`SimNet`, `EchoServer`/`HttpServer`/`MqttBroker`) and **thunks** the
firmware's WiFi + socket calls onto them. This sketch is the test target
for that bring-up.

### Thunk targets (resolved from this fixture's ELF, classic ESP32)

The bring-up intercepts these PCs, marshals the Xtensa call args (a2–a7),
routes to `SimNet`, and writes the return register:

| Concern              | Symbol                        |
|----------------------|-------------------------------|
| WiFi connected?      | `WiFiSTAClass::status` → `WL_CONNECTED` |
| DNS resolve          | `dns_gethostbyname_addrtype` → `SimNet::resolve` |
| open socket          | `lwip_socket` → fd alloc       |
| connect              | `lwip_connect` → `SimNet::connect` |
| send / write         | `lwip_send` / `lwip_write` → `SimNet::send` |
| recv / read          | `lwip_recv` / `lwip_read` → `SimNet::recv` |
| close                | `lwip_close` → `SimNet::close` |

(Addresses are build-specific; resolve them from the symbol table per ELF,
like the arduino-esp32 ROM thunks already do.)

Getting the full ELF to run through `esp_wifi` init to these call sites is
an e-reader-scale bring-up — this fixture + the resolved target list is the
groundwork.
