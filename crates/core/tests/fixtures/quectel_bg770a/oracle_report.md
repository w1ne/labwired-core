# Quectel BG770A-GL — Hardware Oracle Report

> Per-device validation report for `labwired_core::peripherals::components::QuectelBg770a`.
> A "hardware oracle" in LabWired terms: a deterministic model that produces
> the same UART bytes as the real chip, used as ground truth in firmware-in-
> the-loop tests.

## Identity

| Field | Value |
|---|---|
| Manufacturer | Quectel |
| Model | BG770A-GL |
| Firmware (capture source) | `BG770AGLAAR01A05` |
| Module class | LTE-M Cat-M1 / NB-IoT Cat-NB2 |
| Baseband | Sequans Calliope (drives the AT% extension surface) |
| Default UART | 115200 8N1, echo on (ATE1), CMEE off (CMEE=0) |
| Source files | `crates/core/src/peripherals/components/bg770a.rs` (model) |
|  | `crates/core/tests/bg770a_validation.rs` (integration) |
|  | `crates/core/tests/fixtures/quectel_bg770a/datasheet/` (manual V1.3 PDF) |
|  | `crates/core/tests/fixtures/quectel_bg770a/at_harvest.log` (raw bench capture) |

## Validation totals

| Metric | Count |
|---|---|
| Unit tests in `bg770a.rs` | **47** |
| Integration test functions | **4** |
| Byte-exact commands captured from real HW | **132** |
| Shape-pattern (state-varying) commands | **13** |
| Multi-step state sequences (replayed end-to-end) | **9** (**53** total steps) |
| Total assertions covered by validation suite | ≈ **245** |
| Total tests across `labwired-core` (no regressions) | **612 / 612** passing |

Logs from the final validation run are at `test_logs/` next to this file.

## Validation strategies

The integration suite uses three complementary strategies:

1. **Byte-exact** (`EXACT_GOLDEN`): single-shot deterministic commands.
   Compares the model's full UART output (echo + response framing + payload +
   `\r\n\r\nOK\r\n` or `\r\nERROR\r\n` etc.) byte-for-byte against the real
   hardware capture. Catches framing slips, off-by-one whitespace, and
   undocumented field-count quirks.
2. **Shape match** (`SHAPE_GOLDEN`): state-varying commands whose payload
   contains device- or network-specific values (signal, registration, IMEI,
   IMSI, ICCID, PDP context). The captured response is encoded as a template
   with `{N}` (digits), `{H}` (hex), `{R}` (any non-CRLF run) wildcards. The
   model's response must satisfy the same structural template.
3. **State sequence replay** (`SEQUENCES`): multi-command flows captured
   from the bench, replayed through a single long-lived modem instance.
   Tests state retention (echo toggle, CMEE verbosity ladder, CFUN
   write/read, MQTT lifecycle, MQTT-over-TLS lifecycle, HTTP GET path,
   raw TCP socket lifecycle, GPS engine cycle, filesystem CRUD).

## Surface coverage

Every entry below is validated against real hardware (byte-exact or shape) or
covered by a unit test that asserts the documented behaviour from the manual.

### Hayes / V.250 basic set
| Command | Coverage |
|---|---|
| `AT`, `ATE0/1`, `ATQ0/1`, `ATV0/1`, `ATI`, `ATI1` | full |
| `AT&F`, `AT&W`, `AT&V` | full (no-op semantics where applicable) |

### Identity & version
| Command | Coverage |
|---|---|
| `AT+CGMI` / `AT+GMI` | byte-exact: `Quectel` |
| `AT+CGMM` / `AT+GMM` | byte-exact: `BG770A-GL` |
| `AT+CGMR` / `AT+GMR` | byte-exact: `BG770AGLAAR01A05` |
| `AT+CGSN`, `AT+CIMI`, `AT+QCCID` | shape-matched (synthetic IDs in model) |
| `AT+QHVN`, `AT+QHVN?` | static synthetic value |

### SIM & error reporting
| Command | Coverage |
|---|---|
| `AT+CPIN=?` / `?` / `=<pin>` | full (test form returns bare `OK`; read returns `READY`; write returns CME 3 when SIM already ready) |
| `AT+CMEE` modes 0/1/2 | full (verbose error strings from manual Table 27) |

### Network status
| Command | Coverage |
|---|---|
| `AT+CFUN=?` / `?` / `=<fun>[,<rst>]` | full (only `0,1,4` accepted per datasheet) |
| `AT+CSQ=?` / `?` | stateful — `set_signal()` controls return value |
| `AT+QCSQ` | stateful — emits `"NOSERVICE"` or full `"eMTC"` tuple based on `csq_rssi` |
| `AT+CEREG=?` / `?` / `=<n>` | stateful — emits async `+CEREG: <stat>` URC on `set_registration` when `n≥1` |
| `AT+CREG=?` / `?` / `=<n>` | full |
| `AT+COPS?` / `=?` / `=<mode>...` | full (test form errors CME 515 when unattached, mirroring HW quirk) |
| `AT+QNWINFO` | byte-exact: returns cached `"NBIoT","21670","LTE BAND 1",0` |

### Packet domain
| Command | Coverage |
|---|---|
| `AT+CGATT=?` / `?` / `=<state>` | full |
| `AT+CGACT=?` / `?` / `=<state>,<cid>` | full |
| `AT+CGPADDR=?` / `=<cid>` | full (narrows test form to defined cids; omits address when context inactive — HW quirk) |
| `AT+CGDCONT=?` / `?` / `=<cid>,<type>,<apn>...` | full (single context modeled with APN persistence) |

### TCP/IP context (Quectel `+QI*`)
| Command | Coverage |
|---|---|
| `AT+QICSGP=?` / `=<cid>,<type>,<apn>,...` | full |
| `AT+QIACT=?` / `?` / `=<cid>` | full (bare `OK` on read when inactive — HW quirk) |
| `AT+QIDEACT=<cid>` | full |
| `AT+QIDNSCFG=?` / `=<cid>` | full (errors when no PDP active — HW quirk) |
| `AT+QIDNSGIP=?` / `=<cid>,<host>` | async URC (`+QIURC: "dnsgip",...`) |

### Raw TCP/UDP sockets
| Command | Coverage |
|---|---|
| `AT+QIOPEN=?` / `=<ctx>,<connect>,<svc>,<host>,<port>,...` | full state machine + async URC |
| `AT+QISEND=?` / `=<connect>[,<len>]` | prompt mode (`> ` marker, Ctrl-Z submit or fixed length) |
| `AT+QIRD=?` / `=<connect>[,<len>]` | full (reads from per-socket RX buffer; empty by default) |
| `AT+QICLOSE=?` / `=<connect>[,<timeout>]` | full |
| `AT+QISTATE=?` / `?` / `=1,<connect>` | full (lists open sockets with per-tuple shape; bare `OK` when none — HW quirk) |
| `AT+QIGETERROR` | static: `0,operate successfully` |

### TLS sockets (Quectel `+QSSL*`)
| Command | Coverage |
|---|---|
| `AT+QSSLCFG=?` | byte-exact (14 sub-keys with their ranges) |
| `AT+QSSLCFG="seclevel",<ctx>[,<val>]` | full (state persisted per-context) |
| `AT+QSSLCFG="sslversion"/"ciphersuite"/...` | accepted (no state) |
| `AT+QSSLOPEN=?` / `=<ctx>,<ssl_ctx>,<connect>,...` | full state machine (reuses socket table; `service_type="SSL"`) |
| `AT+QSSLSEND=?` / `=<connect>[,<len>]` | prompt mode (shares QISEND infrastructure) |
| `AT+QSSLRECV=?` / `=<connect>,<len>` | full |
| `AT+QSSLCLOSE=?` / `=<connect>` | full |
| `AT+QSSLSTATE?` / `=?` | full |

### MQTT (Quectel `+QMT*`)
| Command | Coverage |
|---|---|
| `AT+QMTCFG=?` | byte-exact (9 sub-keys) |
| `AT+QMTCFG="ssl",<id>[,<enable>,<ctx>]` | full (variable field count per state — HW quirk) |
| `AT+QMTCFG=<other>,...` | accepted (no state) |
| `AT+QMTOPEN=?` / `?` / `=<id>,<host>,<port>` | full state machine + async URC (status 3 when no PDP) |
| `AT+QMTCONN=?` / `?` / `=<id>,<clientID>` | full + async URC |
| `AT+QMTPUB=?` / `=<id>,<msgid>,<qos>,<retain>,<topic>` | prompt mode (`> ` marker) + async URC |
| `AT+QMTSUB=?` / `=<id>,<msgid>,<topic>,<qos>` | full + async URC |
| `AT+QMTDISC=?` / `=<id>` | full + async URC |
| `AT+QMTCLOSE=?` / `=<id>` | full + async URC |
| Incoming `+QMTRECV: <id>,<msgid>,<topic>,<payload>` URC | injected via `inject_mqtt_recv()` |

### HTTP (Quectel `+QHTTP*`)
| Command | Coverage |
|---|---|
| `AT+QHTTPCFG=?` | byte-exact (7 sub-keys) |
| `AT+QHTTPCFG=<key>,<val>` | accepted (no state) |
| `AT+QHTTPURL=?` / `=<len>,<timeout>` | CONNECT prompt mode (fixed length) |
| `AT+QHTTPGET=?` / `=<timeout>...` | full + async URC `+QHTTPGET: 0,200,<len>` |
| `AT+QHTTPPOST=?` / `=<bodyLen>,<rspT>,<reqT>` | CONNECT prompt mode + async URC |
| `AT+QHTTPREAD=?` / `=<waittime>` | emits `CONNECT` + body + `+QHTTPREAD: 0` |

### GPS (`+QGPS*`)
| Command | Coverage |
|---|---|
| `AT+QGPS=?` / `?` / `=<on>[,...]` | stateful — `gps_active` toggle |
| `AT+QGPSEND` | full (CME 505 verbose when GPS off, bypasses CMEE) |
| `AT+QGPSLOC=?` / `=<mode>` | stateful — emits synthetic SF position when active |
| `AT+QGPSCFG=?` | byte-exact (16 sub-keys) |
| `AT+QGPSCFG="outport"/"autogps"/"nmeasrc"/"gnssconfig"[,<val>]` | state persisted for 4 sub-keys |
| `AT+QGPSCFG=<other>,...` | accepted (no state) |

### SMS
| Command | Coverage |
|---|---|
| `AT+CMGF=?` / `?` / `=<mode>` | full (PDU vs text mode persisted) |
| `AT+CMGS=?` / `=<n_or_addr>` | prompt mode + `+CMGS: <mr>` |
| `AT+CMGR=?` / `=<idx>` | stub (bare `OK`) |
| `AT+CMGL=?` / `=<stat>` | stub (bare `OK`) |
| `AT+CMGD=?` / `=<idx>,<delflag>` | stub (bare `OK`) |
| `AT+CNMI=?` / `?` / `=<n1>,...` | full read; writes accepted |
| `AT+CSCS=?` / `?` / `=<chset>` | full (state persisted, GSM/IRA/UCS2) |
| `AT+CSCA=?` / `?` / `=<sca>...` | stub responses |

### Filesystem (UFS)
| Command | Coverage |
|---|---|
| `AT+QFLDS=?` / `=<storage>` | full (in-memory total/free tracking) |
| `AT+QFLST=?` / `=<pattern>` | full (directory listing) |
| `AT+QFUPL=?` / `=<name>,<size>...` | CONNECT prompt mode (fixed length) |
| `AT+QFDWL=?` / `=<name>` | emits `CONNECT` + bytes + `+QFDWL: <len>,<crc>` |
| `AT+QFDEL=?` / `=<name>` | full |
| `AT+QFOPEN=?` / `=<name>,<mode>` | file handle allocation |
| `AT+QFREAD=?` / `=<handle>[,<len>]` | emits `CONNECT <n>` + slice |
| `AT+QFCLOSE=?` / `=<handle>` | full |
| `AT+QFWRITE=?` | test form only |

### NTP / Real-time clock
| Command | Coverage |
|---|---|
| `AT+CCLK=?` / `?` / `=<datetime>` | full (state persisted) |
| `AT+QLTS=?` / `=<mode>` | full (returns current `cclk` string) |
| `AT+QNTP=?` / `=<cid>,<server>[,<port>,<mode>]` | full + async URC `+QNTP: 0,...` |

### Power & power-save
| Command | Coverage |
|---|---|
| `AT+QPOWD` / `=0` / `=1` | full (`POWERED DOWN` URC then silent state) |
| `AT+QSCLK=?` / `?` / `=<mode>` | full (state persisted) |
| `AT+CPSMS=?` / `?` / `=<mode>...` | full (mode persisted) |
| `AT+CEDRXS=?` / `?` / `=<mode>...` | full (mode persisted) |
| `AT+QPSMCFG=?` / `=<thresh>,<ver>` | accepted |

### Network & cell info
| Command | Coverage |
|---|---|
| `AT+QENG=?` | byte-exact |
| `AT+CEINFO=?` | byte-exact |
| `AT+QPING=?` | byte-exact |
| `AT+QLBS=?` / `AT+QLBSCFG=?` | byte-exact |

### FOTA / version-display stubs
| Command | Coverage |
|---|---|
| `AT+QFOTADL=?`, `AT+QKTFOTA=?` | byte-exact (bare `OK`) |
| `AT+QHVN=?` / `AT+QHVN` | byte-exact |

### Phonebook (BG770A doesn't support — modelled as errors)
| Command | Coverage |
|---|---|
| `AT+CPBR=?`, `AT+CPBW=?`, `AT+CPBF=?` | byte-exact: `+CME ERROR: operation not allowed` (bypasses CMEE) |
| `AT+CPBS?`, `AT+CPBS=?` | byte-exact: bare `ERROR` |

### AT% Sequans-extension surface (sampled subset)
| Command | Coverage |
|---|---|
| `AT%RATACT?` | byte-exact: `%RATACT: "NBIOT",1,0` + compact OK |
| `AT%RATSW?` | byte-exact: `%RATSW: 2,1` |
| `AT%MQTTCMD=?` | byte-exact (Sequans drops leading `\r\n`) |
| `AT%CERTCMD=?` | byte-exact (preserves trailing space — Sequans quirk) |
| `AT%MEAS="8"` | byte-exact synthetic measurement |
| `AT%PDNSTAT?`, `AT%SCAN=?`, `AT%PCOINFO?`, `AT%PDNSET=?` | byte-exact (bare OK) |
| `AT%STATEV?`, `AT%PCONI?`, `AT%SCANCFG?`, `AT%MEAS?`, `AT%CCID?` | byte-exact: `+CME ERROR: operation not allowed` |
| `AT%STATUS` | byte-exact: `+CME ERROR: Incorrect parameters` |
| `AT%PDNRDP?` | byte-exact: bare `ERROR` |
| Other AT% commands | default `ERROR` (deterministic miss) |

### AT+VZ Verizon extensions
| Command | Coverage |
|---|---|
| `AT+VZWAPNE?`, `AT+VZWAPNE=?` | byte-exact: bare `ERROR` |
| `AT+VZWRSRP?` | byte-exact: `+CME ERROR: operation not allowed` |

## Real-HW quirks captured

These are behaviours the official AT Commands Manual either omits or contradicts;
the model matches the chip rather than the spec.

| Quirk | Where it shows up |
|---|---|
| Compact `OK` form (single `\r\n` separator, not the standard double) | `AT+QMT{OPEN,CONN,PUB,DISC,CLOSE}=?`, all `AT%*=?` commands |
| Variable-field response based on state | `AT+QMTCFG="ssl",<id>` returns one value when disabled, two when enabled |
| Bare `OK` (no `+...` header) on read forms when collection is empty | `AT+QIACT?`, `AT+QISTATE?`, `AT+QSSLSTATE?`, `AT+QMTOPEN?`, `AT+QMTCONN?` |
| Test form narrows to defined IDs, not the documented range | `AT+CGPADDR=?` returns `(1)` when only cid 1 is defined, not `(1-15)` |
| Address field omitted when context inactive | `AT+CGPADDR=<cid>` returns `+CGPADDR: <cid>` (no address) instead of `<cid>,"0.0.0.0"` |
| State-gated error | `AT+QIDNSCFG=<cid>` errors when no PDP context is active |
| CME error bypasses CMEE filter | `AT+QGPSEND`, `AT+CPBR/W/F=?`, `AT+VZWRSRP?`, `AT%STATEV?`, etc. emit `+CME ERROR: ...` even at CMEE=0 |
| Non-standard verbose CME message | `+CME ERROR: Incorrect parameters` from `AT%STATUS` (not in manual Table 27) |
| Sequans framing drops leading `\r\n` | `AT%MQTTCMD=?` |
| Sequans payload has trailing space | `AT%CERTCMD=?` |
| Cached cell info returned when unattached | `AT+QNWINFO` |
| Test form returns bare `OK` (no documented payload shape) | `AT+QFLDS=?`, `AT+QFLST=?`, `AT+CMGS=?`, `AT+CMGR=?`, `AT+QFOTADL=?`, `AT+CCLK=?`, `AT+QLBS=?` |
| Three distinct prompt-mode markers | `> ` (QMTPUB, QISEND, QSSLSEND, CMGS) / `CONNECT\r\n` (QHTTPURL, QHTTPPOST, QFUPL) / `CONNECT <n>\r\n` (QFREAD) |

## Async URC ordering

Synchronous `OK` always precedes the corresponding async URC on the wire, even
when the URC's own delay is shorter than the command's max response time:

| Command | Sync delay | URC | URC delay (post-OK) |
|---|---|---|---|
| `AT+CFUN=1` (from 0) | 15 s | `+CPIN: READY`, `+QUSIM: 1`, `+QIND: SMS DONE`, `+QIND: PB DONE` | 1 s, 200 ms, 1.8 s, 500 ms |
| `AT+QMTOPEN=...` | 75 s | `+QMTOPEN: <id>,<r>` | 1.5 s |
| `AT+QMTCONN=...` | 5 s | `+QMTCONN: <id>,0,0` | 800 ms |
| `AT+QMTPUB=...` then payload+Ctrl-Z | 400 ms (delivered with OK) | `+QMTPUB: <id>,<mid>,0` | bundled |
| `AT+QMTDISC=<id>` | 300 ms | `+QMTDISC: <id>,0` | 300 ms |
| `AT+QMTCLOSE=<id>` | 300 ms | `+QMTCLOSE: <id>,0` | 300 ms |
| `AT+QIOPEN=...` | 150 s | `+QIOPEN: <connect>,<r>` | 1.5 s |
| `AT+QSSLOPEN=...` | 150 s | `+QSSLOPEN: <connect>,<r>` | 1.5 s |
| `AT+QIDNSGIP=...` | 300 ms | `+QIURC: "dnsgip",0,1` then `+QIURC: "dnsgip","<ip>"` | 3 s |
| `AT+QNTP=...` | 300 ms | `+QNTP: 0,"<cclk>"` | 3 s |
| `AT+QHTTPGET=...` | 300 ms | `+QHTTPGET: 0,<code>,<len>` | 3 s |
| `AT+QHTTPPOST=...` then payload | (after CONNECT prompt) | `+QHTTPPOST: 0,<code>,<len>` | 3 s |
| `AT+QFUPL=...` then payload | (after CONNECT prompt) | `+QFUPL: <len>,0` (bundled with OK) | n/a |
| `AT+QFDWL=<name>` | 300 ms (bundled emission) | `+QFDWL: <len>,0` (in same chunk) | n/a |
| `AT+QPOWD` | 300 ms | `POWERED DOWN` then silent | 700 ms |

## Test injection helpers

The model exposes setters so integration tests can simulate events that, on
real hardware, would come from the network or carrier:

- `set_signal(rssi, ber)` — controls `+CSQ` and `+QCSQ` output
- `set_registration(stat)` — sets `+CEREG?` value; emits URC when `n≥1`
- `set_apn(apn)` — controls `+CGDCONT?` APN field
- `set_cclk(s)` — controls `+CCLK?` and `+QLTS` output
- `complete_network_attach()` — flips `CGATT`/`CGACT`/`CEREG`/`CSQ` into a
  "registered home" state in one call
- `put_file(name, data)` — pre-populates the in-memory UFS
- `inject_mqtt_recv(client_id, topic, payload)` — emits `+QMTRECV:` URC
- `inject_socket_recv(connect_id, data)` — buffers data + emits `+QIURC: "recv",<id>` URC
- `with_boot_urcs()` — schedules the power-on URC chain (`RDY` → `+CPIN: READY` → `+QUSIM` → `+QIND: SMS DONE` → `+QIND: PB DONE`)

## Known gaps

These are intentional non-implementations. The default behaviour for anything
not in the surface table above is `ERROR` (deterministic miss > hallucinated
success).

- Real TLS handshake state machine — TLS sockets reuse the plain-socket state
  table with `service_type = "SSL"`; no certificate verification or cipher
  negotiation is modelled
- Timer-driven `+CEREG` progression (`2 → 1`) — tests must call
  `complete_network_attach()` explicitly
- Twelve of the sixteen `AT+QGPSCFG` sub-keys (only `outport`, `autogps`,
  `nmeasrc`, `gnssconfig` retain state)
- Full Sequans `AT%` surface — ~16 commands sampled out of ~80
- `AT+QENG="servingcell"` write form (only test form modelled)
- `AT+QFWRITE` write form (only test form modelled)
- Audio commands — BG770A is data-only, no PCM/audio path on the chip itself

## Regenerating captures

The byte-exact and shape-pattern data was harvested by typing AT commands at
the BG770A-GL EVB over its UART bridge (USB-PL2303, `/dev/cu.usbserial-1130`
on the dev machine). The Python harness lives inline in the conversation log
that created this work. To re-harvest after a firmware version bump:

1. Power the EVB, ensure `ATE1` is on, `ATE0` may be on initially after factory
   reset — issue `AT&F` then `ATE1` first to normalise.
2. Send each command followed by `\r`, sleep ~700 ms, read everything in the
   RX buffer, record raw bytes.
3. Update the `EXACT_GOLDEN` / `SHAPE_GOLDEN` arrays in
   `bg770a_validation.rs` with the new captures. The unit-test suite will
   catch any regression introduced by the firmware bump.

## How to use the model

```rust
use labwired_core::peripherals::components::QuectelBg770a;
use labwired_core::peripherals::uart::UartStreamDevice;

// Attach to a UART peripheral and drive the firmware.
let mut modem = QuectelBg770a::new();

// Optional: simulate boot URCs the chip emits after power-on.
let mut modem = QuectelBg770a::new().with_boot_urcs();

// Optional: pretend the network attach completed.
modem.complete_network_attach();

// Inject incoming MQTT messages so firmware sees `+QMTRECV:` URCs.
modem.inject_mqtt_recv(0, "sensors/temp", b"22.5");

// Hand to the UART:
// uart.attach_stream(Box::new(modem));
```
